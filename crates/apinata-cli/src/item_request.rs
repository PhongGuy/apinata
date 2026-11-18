//! `apinata item request`: edit one request's fields through the same core setters MCP's
//! `update_request` calls, in the same section order and skipping a section that reads the
//! same, so both shells leave the same bytes behind.

use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use apinata_core::{
    Assertion, BodySpec, Collection, FileEntry, ItemPath, KeyValue, MultipartField, MultipartValue,
    Param, ParamKind, RequestFile, Timeout, VarValue, Variable, normalize_method,
};
use serde::Serialize;

use crate::item_cmd::{auth_from, name_value, text_from};
use crate::{BodyModeArg, RequestArgs, SyncQueryArg};

/// What `request` did, by collection-relative path.
#[derive(Serialize)]
struct RequestDoc<'a> {
    path: &'a str,
    changed: bool,
}

/// One list section's flags: entries to set from `NAME=VALUE`, remove, and switch on or off.
struct ListFlags<'a> {
    what: &'static str,
    flag: &'static str,
    set: &'a [String],
    unset: &'a [String],
    enable: &'a [String],
    disable: &'a [String],
}

impl ListFlags<'_> {
    fn is_empty(&self) -> bool {
        self.set.is_empty()
            && self.unset.is_empty()
            && self.enable.is_empty()
            && self.disable.is_empty()
    }
}

/// How one kind of list entry is matched by name, switched, and set from a flag's value.
struct Entries<T> {
    matches: fn(&T, &str) -> bool,
    set_enabled: fn(&mut T, bool),
    upsert: fn(&mut Vec<T>, &str, &str) -> anyhow::Result<()>,
}

/// Apply `f` to `list` as `item folder` does: unsets first (each must name an entry), then
/// sets, then the toggles, each of which must name an entry too.
fn edit_list<T>(
    list: &mut Vec<T>,
    f: &ListFlags,
    e: &Entries<T>,
    label: &str,
) -> anyhow::Result<()> {
    for name in f.unset {
        let before = list.len();
        list.retain(|x| !(e.matches)(x, name));
        if list.len() == before {
            bail!("no {} {name} in {label}", f.what);
        }
    }
    for raw in f.set {
        let (name, value) = name_value(f.flag, raw)?;
        (e.upsert)(list, name, value)?;
    }
    for (names, on) in [(f.enable, true), (f.disable, false)] {
        for name in names {
            let mut found = false;
            for x in list.iter_mut().filter(|x| (e.matches)(x, name)) {
                (e.set_enabled)(x, on);
                found = true;
            }
            if !found {
                bail!("no {} {name} in {label}", f.what);
            }
        }
    }
    Ok(())
}

const HEADERS: Entries<KeyValue> = Entries {
    matches: |h, n| h.name == n,
    set_enabled: |h, on| h.enabled = on,
    upsert: |list, name, value| {
        match list.iter_mut().find(|h| h.name == name) {
            Some(h) => {
                h.value = value.to_owned();
                h.enabled = true;
            }
            None => list.push(KeyValue::new(name, value)),
        }
        Ok(())
    },
};

const VARS: Entries<Variable> = Entries {
    matches: |v, n| v.name == n,
    set_enabled: |v, on| v.enabled = on,
    upsert: |list, name, value| {
        match list.iter_mut().find(|v| v.name == name) {
            Some(v) => {
                // A typed variable keeps its declared type; only its data changes.
                v.value = match &v.value {
                    VarValue::Typed { kind, .. } => VarValue::Typed {
                        kind: kind.clone(),
                        data: value.to_owned(),
                    },
                    _ => VarValue::Text(value.to_owned()),
                };
                v.enabled = true;
            }
            None => list.push(Variable::new(name, VarValue::Text(value.to_owned()))),
        }
        Ok(())
    },
};

const QUERY: Entries<Param> = Entries {
    matches: |p, n| p.kind == ParamKind::Query && p.name == n,
    set_enabled: |p, on| p.enabled = on,
    upsert: |list, name, value| {
        match list
            .iter_mut()
            .find(|p| p.kind == ParamKind::Query && p.name == name)
        {
            Some(p) => {
                p.value = value.to_owned();
                p.enabled = true;
            }
            None => list.push(Param::new(name, value, ParamKind::Query)),
        }
        Ok(())
    },
};

const MULTIPART: Entries<MultipartField> = Entries {
    matches: |f, n| f.name == n,
    set_enabled: |f, on| f.enabled = on,
    upsert: |list, name, value| {
        let text = MultipartValue::Text(value.to_owned());
        match list.iter_mut().find(|f| f.name == name) {
            Some(f) => {
                f.value = text;
                f.enabled = true;
            }
            None => list.push(MultipartField::text(name, value)),
        }
        Ok(())
    },
};

const ASSERTIONS: Entries<Assertion> = Entries {
    matches: |a, n| a.expression() == n,
    set_enabled: |a, on| a.set_enabled(on),
    upsert: |list, expression, rest| {
        // `eq 200` is the operator, then its value after the first space, as the file stores it.
        let (operator, value) = match rest.split_once(' ') {
            Some((op, value)) => (op, Some(value.to_owned())),
            None => (rest, None),
        };
        if operator.is_empty() {
            bail!("--assert takes EXPR=OP [VALUE]");
        }
        match list.iter_mut().find(|a| a.expression() == expression) {
            Some(a) => {
                if a.operator() != operator || a.value() != value.as_deref() {
                    a.set_operator(operator, value);
                }
                a.set_enabled(true);
            }
            None => list.push(Assertion::new(expression, operator, value)),
        }
        Ok(())
    },
};

/// Edit the request at `args.path` from the flags and save it: each section is rebuilt from
/// what the file holds and set only when it differs, in `update_request`'s order.
pub(crate) fn update_request(
    c: &mut Collection,
    args: &RequestArgs,
    json: bool,
) -> anyhow::Result<()> {
    let label = args.path.as_str();
    let stdin = Path::new("-");
    let stdin_readers = [
        args.body_file.as_deref(),
        args.docs_file.as_deref(),
        args.auth.auth_json_file.as_deref(),
    ];
    if stdin_readers.iter().filter(|f| **f == Some(stdin)).count() > 1 {
        bail!("only one of --body-file, --docs-file and --auth-json-file can read standard input");
    }
    let lists = [
        ListFlags {
            what: "query param",
            flag: "--query",
            set: &args.query,
            unset: &args.unset_query,
            enable: &args.enable_query,
            disable: &args.disable_query,
        },
        ListFlags {
            what: "header",
            flag: "--header",
            set: &args.headers,
            unset: &args.unset_headers,
            enable: &args.enable_headers,
            disable: &args.disable_headers,
        },
        ListFlags {
            what: "variable",
            flag: "--var",
            set: &args.vars,
            unset: &args.unset_vars,
            enable: &args.enable_vars,
            disable: &args.disable_vars,
        },
        ListFlags {
            what: "form field",
            flag: "--form",
            set: &args.form,
            unset: &args.unset_form,
            enable: &args.enable_form,
            disable: &args.disable_form,
        },
        ListFlags {
            what: "assertion on",
            flag: "--assert",
            set: &args.asserts,
            unset: &args.unset_asserts,
            enable: &args.enable_asserts,
            disable: &args.disable_asserts,
        },
    ];
    let [query, headers, vars, form, asserts] = &lists;
    let auth = auth_from(&args.auth)?;
    let docs = text_from(args.docs.as_deref(), args.docs_file.as_deref(), "the docs")?;
    let body_text = text_from(args.body.as_deref(), args.body_file.as_deref(), "the body")?;
    let body_edit = args.body_mode.is_some()
        || body_text.is_some()
        || args.graphql_variables.is_some()
        || !form.is_empty()
        || !args.form_files.is_empty()
        || !args.form_types.is_empty()
        || args.select_file.is_some()
        || !args.file_types.is_empty()
        || !args.unset_files.is_empty();
    let settings_edit = args.timeout.is_some()
        || args.encode_url.is_some()
        || args.follow_redirects.is_some()
        || args.max_redirects.is_some()
        || args.forward_auth.is_some();
    let nothing = args.name.is_none()
        && args.method.is_none()
        && args.url.is_none()
        && args.path_params.is_empty()
        && lists.iter().all(ListFlags::is_empty)
        && !body_edit
        && auth.is_none()
        && !settings_edit
        && docs.is_none()
        && args.tags.is_empty()
        && args.unset_tags.is_empty()
        && args.seq.is_none()
        && args.sync_query.is_none();
    if nothing {
        bail!("give at least one field flag; `apinata item request --help` lists them");
    }

    let r = find(c, label)?;
    if let Some(name) = &args.name {
        r.set_name(name.clone());
    }
    if let Some(method) = &args.method {
        r.set_method(normalize_method(method).map_err(|e| anyhow!("{e}"))?);
    }
    if let Some(url) = &args.url {
        r.set_url(url.clone());
    }
    if !query.is_empty() || !args.path_params.is_empty() {
        let current = r.shown_params();
        let mut list = current.clone();
        edit_list(&mut list, query, &QUERY, label)?;
        for raw in &args.path_params {
            let (name, value) = name_value("--path-param", raw)?;
            let p = list
                .iter_mut()
                .find(|p| p.kind == ParamKind::Path && p.name == name)
                .with_context(|| format!("no path param {name} in {label}"))?;
            p.value = value.to_owned();
            p.enabled = true;
        }
        if list != current {
            r.set_params(list);
        }
    }
    if !headers.is_empty() {
        let current = r.headers();
        let mut list = current.clone();
        edit_list(&mut list, headers, &HEADERS, label)?;
        if list != current {
            r.set_headers(list).map_err(|e| anyhow!("{label}: {e}"))?;
        }
    }
    if body_edit {
        let current = r.body();
        let body = body(current.clone(), args, form, body_text, label)?;
        if body != current {
            r.set_body(body);
        }
    }
    if let Some(auth) = auth
        && auth != r.auth()
    {
        r.set_auth(auth);
    }
    if !vars.is_empty() {
        let current = r.vars();
        let mut list = current.clone();
        edit_list(&mut list, vars, &VARS, label)?;
        if list != current {
            r.set_vars(list);
        }
    }
    if !asserts.is_empty() {
        let current = r.assertions();
        let mut list = current.clone();
        edit_list(&mut list, asserts, &ASSERTIONS, label)?;
        if list != current {
            r.set_assertions(list);
        }
    }
    if settings_edit {
        let current = r.settings();
        let mut s = current;
        if let Some(raw) = &args.timeout {
            s.timeout = timeout(raw)?;
        }
        if let Some(on) = args.encode_url {
            s.encode_url = on;
        }
        if let Some(on) = args.follow_redirects {
            s.follow_redirects = on;
        }
        if let Some(n) = args.max_redirects {
            s.max_redirects = n;
        }
        if let Some(on) = args.forward_auth {
            s.forward_authorization_header = on;
        }
        if s != current {
            r.set_settings(s);
        }
    }
    if let Some(docs) = docs {
        r.set_docs(docs);
    }
    if !args.tags.is_empty() || !args.unset_tags.is_empty() {
        let mut tags = r.tags();
        for tag in &args.unset_tags {
            let before = tags.len();
            tags.retain(|t| t != tag);
            if tags.len() == before {
                bail!("no tag {tag} in {label}");
            }
        }
        for tag in &args.tags {
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
        r.set_tags(tags);
    }
    if let Some(seq) = args.seq
        && r.seq() != Some(seq)
    {
        r.set_seq(seq);
    }
    // Last, as in `update_request`, so it settles whatever the other flags left.
    match args.sync_query {
        Some(SyncQueryArg::Url) => r.use_url_query(),
        Some(SyncQueryArg::Params) => r.use_params_query(),
        None => {}
    }

    let changed = r.is_dirty();
    r.save()
        .with_context(|| format!("could not save {label}"))?;
    if json {
        let doc = RequestDoc {
            path: label,
            changed,
        };
        outln!("{}", serde_json::to_string(&doc)?);
    } else {
        let verb = if changed { "updated" } else { "unchanged" };
        outln!("{verb} {label}");
    }
    Ok(())
}

/// The request at `raw`, with MCP's messages for a bad path, a missing request and one of a
/// kind this command cannot edit.
fn find<'c>(c: &'c mut Collection, raw: &str) -> anyhow::Result<&'c mut RequestFile> {
    let path = ItemPath::new(raw).with_context(|| format!("invalid item path: {raw}"))?;
    if c.request(&path).is_none() {
        match c.opaque_item(&path) {
            Some(other) => bail!(
                "{raw} is a {} request, which this command cannot edit",
                other.kind()
            ),
            None => bail!("no request at {raw}"),
        }
    }
    c.request_mut(&path)
        .with_context(|| format!("no request at {raw}"))
}

/// The body the flags ask for, built from `current` so list entries keep what core remembers.
/// A GraphQL request takes only its query and variables, or no body.
fn body(
    current: BodySpec,
    args: &RequestArgs,
    form: &ListFlags,
    text: Option<String>,
    label: &str,
) -> anyhow::Result<BodySpec> {
    let multipart_only = !args.form_files.is_empty() || !args.form_types.is_empty();
    let file_only =
        args.select_file.is_some() || !args.file_types.is_empty() || !args.unset_files.is_empty();
    if let BodySpec::Graphql { query, variables } = current {
        match args.body_mode {
            Some(BodyModeArg::None) => return Ok(BodySpec::None),
            Some(_) => bail!("{label} is a graphql request, which takes only a graphql body"),
            None => {}
        }
        if !form.is_empty() || multipart_only || file_only {
            bail!("{label} is a graphql request, which takes only --body and --graphql-variables");
        }
        return Ok(BodySpec::Graphql {
            query: text.unwrap_or(query),
            variables: args.graphql_variables.clone().unwrap_or(variables),
        });
    }
    if args.graphql_variables.is_some() {
        bail!("--graphql-variables is for a graphql request");
    }
    let current_text = match &current {
        BodySpec::Json(t) | BodySpec::Xml(t) | BodySpec::Text(t) | BodySpec::Sparql(t) => {
            Some(t.clone())
        }
        _ => None,
    };
    let mode = match (args.body_mode, &current) {
        (Some(mode), _) => mode,
        (None, BodySpec::None) => BodyModeArg::None,
        (None, BodySpec::Json(_)) => BodyModeArg::Json,
        (None, BodySpec::Xml(_)) => BodyModeArg::Xml,
        (None, BodySpec::Text(_)) => BodyModeArg::Text,
        (None, BodySpec::Sparql(_)) => BodyModeArg::Sparql,
        (None, BodySpec::FormUrlEncoded(_)) => BodyModeArg::FormUrlencoded,
        (None, BodySpec::Multipart(_)) => BodyModeArg::Multipart,
        (None, BodySpec::File(_)) => BodyModeArg::File,
        (None, _) => bail!("{label}'s body mode cannot be edited; pick one with --body-mode"),
    };
    let is_text = matches!(
        mode,
        BodyModeArg::Json | BodyModeArg::Xml | BodyModeArg::Text | BodyModeArg::Sparql
    );
    if text.is_some() && !is_text {
        bail!("--body sets a json, xml, text or sparql body; pick one with --body-mode");
    }
    if !form.is_empty() && !matches!(mode, BodyModeArg::FormUrlencoded | BodyModeArg::Multipart) {
        bail!("--form edits a form-urlencoded or multipart body; pick one with --body-mode");
    }
    if multipart_only && mode != BodyModeArg::Multipart {
        bail!(
            "--form-file and --form-type edit a multipart body; pick it with --body-mode multipart"
        );
    }
    if file_only && mode != BodyModeArg::File {
        bail!(
            "--select-file, --file-type and --unset-file edit a file body; pick it with --body-mode file"
        );
    }
    let text = || text.clone().or(current_text.clone()).unwrap_or_default();
    Ok(match mode {
        BodyModeArg::None => BodySpec::None,
        BodyModeArg::Json => BodySpec::Json(text()),
        BodyModeArg::Xml => BodySpec::Xml(text()),
        BodyModeArg::Text => BodySpec::Text(text()),
        BodyModeArg::Sparql => BodySpec::Sparql(text()),
        BodyModeArg::FormUrlencoded => {
            let mut list = match current {
                BodySpec::FormUrlEncoded(f) => f,
                _ => Vec::new(),
            };
            edit_list(&mut list, form, &HEADERS, label)?;
            BodySpec::FormUrlEncoded(list)
        }
        BodyModeArg::Multipart => {
            let mut list = match current {
                BodySpec::Multipart(f) => f,
                _ => Vec::new(),
            };
            edit_list(&mut list, form, &MULTIPART, label)?;
            multipart_files(&mut list, &args.form_files)?;
            for raw in &args.form_types {
                let (name, kind) = name_value("--form-type", raw)?;
                let f = list
                    .iter_mut()
                    .find(|f| f.name == name)
                    .with_context(|| format!("no form field {name} in {label}"))?;
                f.content_type = kind.to_owned();
            }
            BodySpec::Multipart(list)
        }
        BodyModeArg::File => {
            let mut list = match current {
                BodySpec::File(f) => f,
                _ => Vec::new(),
            };
            file_entries(&mut list, args, label)?;
            BodySpec::File(list)
        }
    })
}

/// Point each `--form-file` field at all the paths given for its name, in flag order.
fn multipart_files(list: &mut Vec<MultipartField>, flags: &[String]) -> anyhow::Result<()> {
    let mut grouped: Vec<(&str, Vec<String>)> = Vec::new();
    for raw in flags {
        let (name, path) = name_value("--form-file", raw)?;
        match grouped.iter_mut().find(|(n, _)| *n == name) {
            Some((_, paths)) => paths.push(path.to_owned()),
            None => grouped.push((name, vec![path.to_owned()])),
        }
    }
    for (name, paths) in grouped {
        match list.iter_mut().find(|f| f.name == name) {
            Some(f) => {
                f.value = MultipartValue::Files(paths);
                f.enabled = true;
            }
            None => list.push(MultipartField::files(name, paths)),
        }
    }
    Ok(())
}

/// Remove, type, then select file body entries; selecting one deselects the rest.
fn file_entries(list: &mut Vec<FileEntry>, args: &RequestArgs, label: &str) -> anyhow::Result<()> {
    for path in &args.unset_files {
        let before = list.len();
        list.retain(|f| &f.path != path);
        if list.len() == before {
            bail!("no file {path} in {label}");
        }
    }
    let entry = |list: &mut Vec<FileEntry>, path: &str| -> usize {
        match list.iter().position(|f| f.path == path) {
            Some(i) => i,
            None => {
                list.push(FileEntry::new(path));
                list.len() - 1
            }
        }
    };
    for raw in &args.file_types {
        let (path, kind) = name_value("--file-type", raw)?;
        let i = entry(list, path);
        list[i].content_type = kind.to_owned();
    }
    if let Some(path) = &args.select_file {
        let i = entry(list, path);
        for (j, f) in list.iter_mut().enumerate() {
            f.selected = i == j;
        }
    }
    Ok(())
}

/// A `--timeout` value; 0 ms is refused as MCP refuses it, since the file writes `unlimited`
/// as 0.
fn timeout(raw: &str) -> anyhow::Result<Timeout> {
    match raw {
        "inherit" => Ok(Timeout::Inherit),
        "unlimited" => Ok(Timeout::Unlimited),
        _ => match raw.parse::<u64>() {
            Ok(0) => bail!("a timeout of 0 ms is `unlimited`; use that"),
            Ok(ms) => Ok(Timeout::Millis(ms)),
            Err(_) => bail!("--timeout takes milliseconds, `unlimited` or `inherit`"),
        },
    }
}
