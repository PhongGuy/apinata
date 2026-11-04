//! `apinata item`: create, copy, rename, move and remove requests and folders, and edit a
//! folder's shared sections, through the same core calls the MCP tools make, so both leave
//! the same files behind.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use apinata_core::item_ops::{self, FolderPatch, FolderSections};
use apinata_core::{
    ApiKeyPlacement, AuthSpec, Collection, KeyValue, NewRequest, RequestKind, SystemTrash,
    TrashLog, VarValue, Variable,
};
use serde::Serialize;

use crate::trust;
use crate::{
    ApiKeyInArg, EXIT_USAGE, FolderArgs, FolderAuthArg, Format, ItemAction, ItemArgs, ItemKindArg,
    collection_arg,
};

/// Where an action left its item: the MCP tools' `ItemPathResult`.
#[derive(Serialize)]
struct PathDoc<'a> {
    path: &'a str,
}

/// What `rm` removed, named the way `import --prune` names the trash.
#[derive(Serialize)]
struct RemovedDoc<'a> {
    removed: &'a str,
    destination: &'static str,
    warnings: &'a [String],
}

/// What `restore` put back: the MCP tool's `RestoredItem`.
#[derive(Serialize)]
struct RestoredDoc<'a> {
    restored: &'a str,
    warnings: &'a [String],
}

/// What `restore --pruned` put back: the MCP tool's `RestoredPrune`.
#[derive(Serialize)]
struct RestoredAllDoc<'a> {
    restored: Vec<String>,
    warnings: &'a [String],
}

/// What `scrub-userinfo` found: the MCP tool's `ScrubbedUserinfo`.
#[derive(Serialize)]
struct ScrubDoc<'a> {
    written: bool,
    changes: Vec<ScrubChangeDoc<'a>>,
}

#[derive(Serialize)]
struct ScrubChangeDoc<'a> {
    item: String,
    example: &'a str,
    url: &'a str,
}

/// What `folder` did, by collection-relative paths.
#[derive(Serialize)]
struct FolderDoc<'a> {
    path: &'a str,
    file: &'a str,
    changed: bool,
}

/// Run the action: 0 when done, 3 for a usage, collection or tree error.
pub fn run(args: &ItemArgs) -> ExitCode {
    match go(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn go(args: &ItemArgs) -> anyhow::Result<()> {
    let root = collection_arg::with_hint(
        trust::directory_root(args.collection.as_deref()),
        "pass the collection with --collection <DIR>",
    )?;
    let mut c = Collection::open(&root)?;
    let json = matches!(args.format, Format::Json);
    // Each arm prints the item's new path, prefixed in text by what happened to it.
    let (verb, path) = match &args.action {
        ItemAction::New {
            name,
            parent,
            kind,
            method,
            url,
        } => {
            let mut new = NewRequest::new();
            if let Some(kind) = kind {
                new = new.kind(match kind {
                    ItemKindArg::Http => RequestKind::Http,
                    ItemKindArg::Graphql => RequestKind::Graphql,
                });
            }
            if let Some(method) = method {
                new = new.method(method.as_str());
            }
            if let Some(url) = url {
                new = new.url(url.as_str());
            }
            let made = item_ops::create_request(&mut c, folder(parent.as_deref()), name, &new)?;
            ("created".to_owned(), made)
        }
        ItemAction::Mkdir { name, parent } => {
            let made = item_ops::create_folder(&mut c, folder(parent.as_deref()), name)?;
            ("created".to_owned(), made)
        }
        ItemAction::Cp { path, name } => (
            format!("copied {path} to"),
            item_ops::clone_item(&mut c, path, name)?,
        ),
        ItemAction::Rename { path, name } => (
            format!("renamed {path} to"),
            item_ops::rename_item(&mut c, path, name)?,
        ),
        ItemAction::Mv {
            path,
            into,
            before,
            after,
        } => {
            let into = into.as_deref().map(|f| folder(Some(f)));
            let moved =
                item_ops::move_item(&mut c, path, into, before.as_deref(), after.as_deref())?;
            (format!("moved {path} to"), moved)
        }
        ItemAction::Rm { path } => return remove(&mut c, path, json),
        ItemAction::Restore { pruned: false, .. } => return restore(&mut c, json),
        ItemAction::Restore {
            pruned: true,
            batch,
        } => return restore_pruned(&mut c, batch.as_deref(), json),
        ItemAction::ScrubUserinfo { write } => return scrub_userinfo(&mut c, *write, json),
        ItemAction::Folder(args) => return update_folder(&mut c, args, json),
    };
    let path = path.to_string();
    if json {
        outln!("{}", serde_json::to_string(&PathDoc { path: &path })?);
    } else {
        outln!("{verb} {path}");
    }
    Ok(())
}

/// Move the item to the system trash and record it for `restore`; a sibling that could not
/// be renumbered, or a record that could not be written, is a warning.
fn remove(c: &mut Collection, path: &str, json: bool) -> anyhow::Result<()> {
    let log = TrashLog::default_dir().map(TrashLog::new);
    let warnings = item_ops::trash_item_logged(c, path, &SystemTrash::new(), log.as_ref())?;
    if json {
        let doc = RemovedDoc {
            removed: path,
            destination: "trash",
            warnings: &warnings,
        };
        outln!("{}", serde_json::to_string(&doc)?);
    } else {
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        outln!("removed {path} (moved to the trash)");
    }
    Ok(())
}

/// Put back the newest item this collection's `rm` trashed, as MCP's `restore_item` does.
fn restore(c: &mut Collection, json: bool) -> anyhow::Result<()> {
    let log = TrashLog::new(
        TrashLog::default_dir().context("restoring is not available: no data directory")?,
    );
    let restored = item_ops::restore_last_trashed(c, &SystemTrash::new(), &log)?;
    let path = restored.item().to_string();
    if json {
        let doc = RestoredDoc {
            restored: &path,
            warnings: restored.warnings(),
        };
        outln!("{}", serde_json::to_string(&doc)?);
    } else {
        for warning in restored.warnings() {
            eprintln!("warning: {warning}");
        }
        outln!("restored {path}");
    }
    Ok(())
}

/// Put back every request `batch` pruned, or the newest prune's without one, as MCP's
/// `restore_pruned` does.
fn restore_pruned(c: &mut Collection, batch: Option<&str>, json: bool) -> anyhow::Result<()> {
    let log = TrashLog::new(
        TrashLog::default_dir().context("restoring is not available: no data directory")?,
    );
    let bin = SystemTrash::new();
    let restored = match batch {
        Some(batch) => item_ops::restore_prune(c, &bin, &log, batch)?,
        None => item_ops::restore_last_prune(c, &bin, &log)?,
    };
    let paths: Vec<String> = restored.items().iter().map(|p| p.to_string()).collect();
    if json {
        let doc = RestoredAllDoc {
            restored: paths,
            warnings: restored.warnings(),
        };
        outln!("{}", serde_json::to_string(&doc)?);
    } else {
        for warning in restored.warnings() {
            eprintln!("warning: {warning}");
        }
        for path in &paths {
            outln!("restored {path}");
        }
    }
    Ok(())
}

/// List, or with `write` save, the saved examples whose recorded URL held userinfo; only the
/// scrubbed URL is ever printed.
fn scrub_userinfo(c: &mut Collection, write: bool, json: bool) -> anyhow::Result<()> {
    let report = apinata_core::scrub_example_userinfo(c, write)?;
    if json {
        let doc = ScrubDoc {
            written: report.written(),
            changes: report
                .changes()
                .iter()
                .map(|ch| ScrubChangeDoc {
                    item: ch.item().to_string(),
                    example: ch.example(),
                    url: ch.url(),
                })
                .collect(),
        };
        outln!("{}", serde_json::to_string(&doc)?);
        return Ok(());
    }
    if report.changes().is_empty() {
        outln!("no saved example holds userinfo");
        return Ok(());
    }
    let verb = if write { "scrubbed" } else { "would scrub" };
    for ch in report.changes() {
        outln!(
            "{verb} {} example {:?}: {}",
            ch.item(),
            ch.example(),
            ch.url()
        );
    }
    if !write {
        outln!("run again with --write to save");
    }
    Ok(())
}

/// A folder argument as core takes it: absent and `.` both mean the root.
fn folder(raw: Option<&str>) -> &str {
    match raw {
        None | Some(".") => "",
        Some(f) => f,
    }
}

/// Edit the folder's lists from the flags and send them, with any auth and docs, in one core
/// `update_folder` call: unsets first (each must name an entry), then sets, then toggles.
fn update_folder(c: &mut Collection, args: &FolderArgs, json: bool) -> anyhow::Result<()> {
    let FolderArgs {
        path,
        headers,
        unset_headers,
        enable_headers,
        disable_headers,
        vars,
        unset_vars,
        enable_vars,
        disable_vars,
        auth,
        bearer,
        basic,
        digest,
        api_key,
        api_key_in,
        auth_json,
        auth_json_file,
        docs,
        docs_file,
    } = args;
    let path = folder(Some(path));
    let stdin = Path::new("-");
    if auth_json_file.as_deref() == Some(stdin) && docs_file.as_deref() == Some(stdin) {
        bail!("only one of --auth-json-file and --docs-file can read standard input");
    }
    let auth = match (auth_json, auth_json_file) {
        (Some(text), _) => Some(parse_auth(text)?),
        (None, Some(file)) => Some(parse_auth(&read_text(file, "the auth JSON")?)?),
        (None, None) => auth_flag(*auth, bearer, basic, digest, api_key, *api_key_in)?,
    };
    let docs = match (docs, docs_file) {
        (Some(text), _) => Some(text.clone()),
        (None, Some(file)) => Some(read_text(file, "the docs")?),
        (None, None) => None,
    };
    let header_edit = !(headers.is_empty()
        && unset_headers.is_empty()
        && enable_headers.is_empty()
        && disable_headers.is_empty());
    let var_edit = !(vars.is_empty()
        && unset_vars.is_empty()
        && enable_vars.is_empty()
        && disable_vars.is_empty());
    if !header_edit && !var_edit && auth.is_none() && docs.is_none() {
        bail!(
            "give at least one of --header, --unset-header, --enable-header, --disable-header, \
             --var, --unset-var, --enable-var, --disable-var, an auth flag, --docs or --docs-file"
        );
    }
    let label = if path.is_empty() {
        "the collection".to_owned()
    } else {
        path.to_owned()
    };
    let current: FolderSections = item_ops::read_folder(c, path)?;
    let mut patch = FolderPatch::new();
    if header_edit {
        let mut list = current.headers;
        for name in unset_headers {
            remove_named(&mut list, name, |h| &h.name)
                .with_context(|| format!("no header {name} in {label}"))?;
        }
        for flag in headers {
            let (name, value) = name_value("--header", flag)?;
            match list.iter_mut().find(|h| h.name == name) {
                Some(h) => {
                    h.value = value.to_owned();
                    h.enabled = true;
                }
                None => list.push(KeyValue::new(name, value)),
            }
        }
        toggle(&mut list, enable_headers, disable_headers, |h| {
            (&h.name, &mut h.enabled)
        })
        .map_err(|name| anyhow::anyhow!("no header {name} in {label}"))?;
        patch = patch.headers(list);
    }
    if var_edit {
        let mut list = current.vars;
        for name in unset_vars {
            remove_named(&mut list, name, |v| &v.name)
                .with_context(|| format!("no variable {name} in {label}"))?;
        }
        for flag in vars {
            let (name, value) = name_value("--var", flag)?;
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
        }
        toggle(&mut list, enable_vars, disable_vars, |v| {
            (&v.name, &mut v.enabled)
        })
        .map_err(|name| anyhow::anyhow!("no variable {name} in {label}"))?;
        patch = patch.vars(list);
    }
    if let Some(auth) = auth {
        patch = patch.auth(auth);
    }
    if let Some(docs) = docs {
        patch = patch.docs(docs);
    }
    let done = item_ops::update_folder(c, path, &patch)?;
    let file = relative(c.root(), done.file());
    if json {
        let doc = FolderDoc {
            path,
            file: &file,
            changed: done.changed(),
        };
        outln!("{}", serde_json::to_string(&doc)?);
    } else {
        let verb = if done.changed() {
            "updated"
        } else {
            "unchanged"
        };
        outln!("{verb} {label} ({file})");
    }
    Ok(())
}

/// The auth the flags name, if any; clap already allows at most one.
fn auth_flag(
    mode: Option<FolderAuthArg>,
    bearer: &Option<String>,
    basic: &Option<String>,
    digest: &Option<String>,
    api_key: &Option<String>,
    api_key_in: Option<ApiKeyInArg>,
) -> anyhow::Result<Option<AuthSpec>> {
    let pair = |flag: &str, raw: &str| -> anyhow::Result<(String, String)> {
        // Credentials stay out of the message.
        match raw.split_once(':') {
            Some((user, pass)) if !user.is_empty() => Ok((user.to_owned(), pass.to_owned())),
            _ => bail!("{flag} takes USER:PASS"),
        }
    };
    Ok(Some(match (mode, bearer, basic, digest, api_key) {
        (Some(FolderAuthArg::None), ..) => AuthSpec::None,
        (Some(FolderAuthArg::Inherit), ..) => AuthSpec::Inherit,
        (_, Some(token), ..) => AuthSpec::Bearer {
            token: token.clone(),
        },
        (_, _, Some(raw), ..) => {
            let (username, password) = pair("--basic", raw)?;
            AuthSpec::Basic { username, password }
        }
        (_, _, _, Some(raw), _) => {
            let (username, password) = pair("--digest", raw)?;
            AuthSpec::Digest { username, password }
        }
        (.., Some(raw)) => match raw.split_once('=') {
            Some((key, value)) if !key.is_empty() => AuthSpec::ApiKey {
                key: key.to_owned(),
                value: value.to_owned(),
                placement: match api_key_in {
                    Some(ApiKeyInArg::Query) => ApiKeyPlacement::Query,
                    _ => ApiKeyPlacement::Header,
                },
            },
            _ => bail!("--api-key takes NAME=VALUE"),
        },
        _ => return Ok(None),
    }))
}

/// Core's spec for an `--auth-json` document; core's message never repeats a value.
fn parse_auth(text: &str) -> anyhow::Result<AuthSpec> {
    apinata_core::parse_auth_json(text).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Enable, then disable, every entry with each name given; `Err` names the first name that
/// matches no entry.
fn toggle<'a, T>(
    list: &mut [T],
    enable: &'a [String],
    disable: &'a [String],
    entry: fn(&mut T) -> (&String, &mut bool),
) -> Result<(), &'a str> {
    for (names, on) in [(enable, true), (disable, false)] {
        for name in names {
            let mut found = false;
            for e in list.iter_mut() {
                let (n, enabled) = entry(e);
                if n == name {
                    *enabled = on;
                    found = true;
                }
            }
            if !found {
                return Err(name);
            }
        }
    }
    Ok(())
}

/// Split a `NAME=VALUE` flag at its first `=`; the message never repeats the value.
fn name_value<'a>(flag: &str, raw: &'a str) -> anyhow::Result<(&'a str, &'a str)> {
    match raw.split_once('=') {
        Some((name, value)) if !name.is_empty() => Ok((name, value)),
        _ => bail!("{flag} takes NAME=VALUE"),
    }
}

/// Remove every entry named `name`; `None` when there was none.
fn remove_named<T>(list: &mut Vec<T>, name: &str, of: fn(&T) -> &String) -> Option<()> {
    let before = list.len();
    list.retain(|e| of(e) != name);
    (list.len() < before).then_some(())
}

/// `file`'s text, or standard input for `-`; `what` names it in the error.
fn read_text(file: &Path, what: &str) -> anyhow::Result<String> {
    if file == Path::new("-") {
        return std::io::read_to_string(std::io::stdin())
            .with_context(|| format!("could not read {what} from standard input"));
    }
    std::fs::read_to_string(file).with_context(|| format!("could not read {}", file.display()))
}

/// `file` relative to the collection root, with `/` separators.
fn relative(root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
