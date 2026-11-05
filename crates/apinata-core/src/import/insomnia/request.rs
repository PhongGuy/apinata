//! Requests and folders: URL and params, headers, bodies, settings, docs and scripts.

use super::auth::auth;
use super::model::{FolderSource, Member, RequestSource, UNREAD, rest_paths, text};
use super::template::apply;
use super::{Budget, Crumbs};
use crate::edit::{
    AuthSpec, BodySpec, FileEntry, KeyValue, MultipartField, Param, ParamKind, RequestSettings,
};
use crate::import::request::graphql_parts;
use crate::import::{DocumentError, ImportNote, ImportedRequest, NoteKind};
use crate::request::RequestKind;
use crate::url_params::{build_query_string, split_on_first};
use serde_json::{Map, Value};

/// Map a request. `trail` ends with the request's name and prefixes every note source.
pub(super) fn request(
    source: RequestSource,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<ImportedRequest, DocumentError> {
    let method = member_text(source.method);
    let method = if method.trim().is_empty() {
        "GET".to_owned()
    } else {
        method
    };
    // Template notes follow the fields: URL, parameters, headers, body, authentication.
    let mut url = apply(&member_text(source.url), trail, "url", notes, budget)?;
    let mut params: Vec<Param> = rows(source.parameters, &[], trail, notes, budget)?
        .into_iter()
        .map(|row| param(row.kv, ParamKind::Query))
        .collect();
    // Apinata sends the URL as written, so the enabled query params join it as well as the list.
    let query = build_query_string(
        params
            .iter()
            .filter(|p| p.enabled)
            .map(|p| (p.name.as_str(), p.value.as_str())),
    );
    if !query.is_empty() {
        budget.charge(query.len() + 1)?;
        // The query belongs before any fragment; a `?` inside the fragment does not count.
        let at = match split_on_first(&url, '#') {
            (before, Some(_)) => Some(before.len()),
            (_, None) => None,
        };
        let fragment = at.map(|at| url.split_off(at));
        match split_on_first(&url, '?').1 {
            None => url.push('?'),
            Some(q) if !q.is_empty() && !q.ends_with('&') => url.push('&'),
            Some(_) => {}
        }
        url.push_str(&query);
        url.extend(fragment);
    }
    let path_params = rows(source.path_parameters, &[], trail, notes, budget)?;
    params.extend(
        path_params
            .into_iter()
            .map(|row| param(row.kv, ParamKind::Path)),
    );
    let mut request = ImportedRequest::new(method, url);
    request.params = params;
    request.headers = headers(source.headers, trail, notes, budget)?;
    request.body = body(source.body, trail, notes, budget)?;
    if matches!(request.body, BodySpec::Graphql { .. }) {
        request.kind = RequestKind::Graphql;
    }
    request.auth = auth(source.auth, trail, &mut request.headers, notes, budget)?;
    request.docs = member_text(source.description);
    request.settings = settings(source.encode_url.as_ref(), source.follow_redirects.as_ref());
    for (setting, default) in source.other_settings {
        if !matches!(setting.value, Value::Null) && setting.value != Value::Bool(default) {
            let detail = "this setting has no counterpart and is not imported";
            notes.push(budget.note(trail, &setting.path, NoteKind::Dropped, detail)?);
        }
    }
    scripts(source.scripts, trail, notes, budget)?;
    budget.unread(trail, source.unread, notes)?;
    Ok(request)
}

/// A folder's docs, headers and auth. `trail` ends with the folder's name.
pub(super) fn folder_parts(
    source: FolderSource,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<(String, Vec<KeyValue>, AuthSpec), DocumentError> {
    let mut headers = headers(source.headers, trail, notes, budget)?;
    let auth = auth(source.auth, trail, &mut headers, notes, budget)?;
    scripts(source.scripts, trail, notes, budget)?;
    budget.unread(trail, source.unread, notes)?;
    Ok((member_text(source.description), headers, auth))
}

/// A body's MIME type without parameters, trimmed and lower-cased.
fn mime(value: Option<&Value>) -> String {
    let full = text(value);
    let essence = full.split(';').next().unwrap_or_default();
    essence.trim().to_ascii_lowercase()
}

fn member_text(member: Option<Member>) -> String {
    text(member.map(|m| m.value).as_ref())
}

fn headers(
    member: Option<Member>,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Vec<KeyValue>, DocumentError> {
    let rows = rows(member, &[], trail, notes, budget)?;
    Ok(rows.into_iter().map(|row| row.kv).collect())
}

fn param(kv: KeyValue, kind: ParamKind) -> Param {
    let mut param = Param::new(kv.name, kv.value, kind);
    param.enabled = kv.enabled;
    param.description = kv.description;
    param
}

/// One object entry of a `{name, value, description, disabled}` list.
struct Row {
    index: usize,
    entry: Map<String, Value>,
    kv: KeyValue,
}

/// The keys every list entry may hold; `id` names the entry and carries no data.
const ROW_KEYS: [&str; 5] = ["name", "value", "disabled", "description", "id"];

/// The object entries of a list member. A member that is not a list, or an entry that is not
/// an object, is noted and skipped; so is each entry key that is neither a row key nor in `read`.
fn rows(
    member: Option<Member>,
    read: &[&str],
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Vec<Row>, DocumentError> {
    let Some(Member { value, path }) = member else {
        return Ok(Vec::new());
    };
    let list = match value {
        Value::Array(list) => list,
        Value::Null => return Ok(Vec::new()),
        _ => {
            let detail = "not a list; not imported";
            notes.push(budget.note(trail, &path, NoteKind::Dropped, detail)?);
            return Ok(Vec::new());
        }
    };
    let skip: Vec<&str> = ROW_KEYS.iter().chain(read).copied().collect();
    let mut out = Vec::with_capacity(list.len());
    for (i, entry) in list.into_iter().enumerate() {
        let Value::Object(entry) = entry else {
            let detail = "not an object; not imported";
            let at = format!("{path}[{i}]");
            notes.push(budget.note(trail, &at, NoteKind::Dropped, detail)?);
            continue;
        };
        budget.entry()?;
        let at = format!("{path}[{i}].value");
        let value = apply(&text(entry.get("value")), trail, &at, notes, budget)?;
        let mut kv = KeyValue::new(text(entry.get("name")), value);
        kv.enabled = entry.get("disabled").and_then(Value::as_bool) != Some(true);
        let description = text(entry.get("description"));
        kv.description = (!description.trim().is_empty()).then_some(description);
        let mut unread = Vec::new();
        rest_paths(&entry, &format!("{path}[{i}]."), &skip, &mut unread);
        budget.unread(trail, unread, notes)?;
        out.push(Row {
            index: i,
            entry,
            kv,
        });
    }
    Ok(out)
}

fn body(
    member: Option<Member>,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<BodySpec, DocumentError> {
    let (mut body, path) = match member {
        None => return Ok(BodySpec::None),
        Some(Member {
            value: Value::Object(body),
            path,
        }) => (body, path),
        Some(Member { value, path }) => {
            if !value.is_null() {
                let detail = "not an object; not imported";
                notes.push(budget.note(trail, &path, NoteKind::Dropped, detail)?);
            }
            return Ok(BodySpec::None);
        }
    };
    let mime = mime(body.get("mimeType"));
    let content = text(body.get("text"));
    let text_at = format!("{path}.text");
    let templated = |notes: &mut Vec<ImportNote>, budget: &mut Budget| {
        apply(&content, trail, &text_at, notes, budget)
    };
    let file_name = text(body.get("fileName"));
    let mut params = body.remove("params");
    let params_at = format!("{path}.params");
    // An absolute path from the exporting machine is kept, but the user has to check it.
    let exported_path = "the file path is from the exporting machine; check that it exists here";
    let spec = match mime.as_str() {
        "application/json" => BodySpec::Json(templated(notes, budget)?),
        "application/x-www-form-urlencoded" => {
            let params = Member::new(params.take(), &params_at);
            let rows = rows(params, &[], trail, notes, budget)?;
            BodySpec::FormUrlEncoded(rows.into_iter().map(|row| row.kv).collect())
        }
        "multipart/form-data" => {
            let mut parts = Vec::new();
            let params = Member::new(params.take(), &params_at);
            for Row { index, entry, kv } in
                rows(params, &["type", "fileName"], trail, notes, budget)?
            {
                let mut part = if text(entry.get("type")) == "file" {
                    let file = text(entry.get("fileName"));
                    if file.is_empty() {
                        MultipartField::files(kv.name, Vec::new())
                    } else {
                        let at = format!("{path}.params[{index}].fileName");
                        notes.push(budget.note(trail, &at, NoteKind::Converted, exported_path)?);
                        MultipartField::files(kv.name, vec![file])
                    }
                } else {
                    MultipartField::text(kv.name, kv.value)
                };
                part.enabled = kv.enabled;
                part.description = kv.description;
                parts.push(part);
            }
            BodySpec::Multipart(parts)
        }
        "application/graphql" => graphql(&content, trail, &text_at, notes, budget)?,
        "text/plain" => BodySpec::Text(templated(notes, budget)?),
        "application/xml" | "text/xml" => BodySpec::Xml(templated(notes, budget)?),
        "application/octet-stream" if !file_name.is_empty() => {
            let at = format!("{path}.fileName");
            notes.push(budget.note(trail, &at, NoteKind::Converted, exported_path)?);
            let mut file = FileEntry::new(file_name);
            file.selected = true;
            BodySpec::File(vec![file])
        }
        _ if content.is_empty() => BodySpec::None,
        "" => BodySpec::Text(templated(notes, budget)?),
        other => {
            let at = format!("{path}.mimeType");
            let detail = format!("the `{other}` body is imported as text");
            notes.push(budget.note(trail, &at, NoteKind::Converted, detail)?);
            BodySpec::Text(templated(notes, budget)?)
        }
    };
    // Insomnia keeps the members of a previous body type; those the chosen type does not use
    // are noted when they hold anything, and so is every unknown key.
    let mut unread = Vec::new();
    let used = [
        (
            "text",
            matches!(
                spec,
                BodySpec::Json(_) | BodySpec::Text(_) | BodySpec::Xml(_) | BodySpec::Graphql { .. }
            ),
        ),
        ("fileName", matches!(spec, BodySpec::File(_))),
    ];
    for (key, used) in used {
        if !used && body.get(key).is_some_and(holds_data) {
            unread.push((format!("{path}.{key}"), UNREAD));
        }
    }
    if params.as_ref().is_some_and(holds_data) {
        unread.push((params_at, UNREAD));
    }
    let skip = ["mimeType", "fileName", "text"];
    rest_paths(&body, &format!("{path}."), &skip, &mut unread);
    budget.unread(trail, unread, notes)?;
    Ok(spec)
}

/// A GraphQL body from its JSON text, templates rewritten in the query and variables. Text
/// that is not a JSON object imports empty with a note, as does a named operation, which a
/// request file cannot hold.
fn graphql(
    content: &str,
    trail: &[String],
    text_at: &str,
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<BodySpec, DocumentError> {
    let parsed = serde_json::from_str::<Value>(content)
        .ok()
        .filter(Value::is_object);
    let Some(parsed) = parsed else {
        if !content.trim().is_empty() {
            let detail = "the GraphQL body is not JSON with a query and variables; not imported";
            notes.push(budget.note(trail, text_at, NoteKind::Dropped, detail)?);
        }
        return Ok(BodySpec::Graphql {
            query: String::new(),
            variables: String::new(),
        });
    };
    if let Some(Value::String(name)) = parsed.get("operationName")
        && !name.is_empty()
    {
        let detail =
            format!("the operation name `{name}` is not imported; the request is sent without one");
        notes.push(budget.note(trail, text_at, NoteKind::Dropped, detail)?);
    }
    let (query, variables) = graphql_parts(Some(&parsed));
    // Indenting nested variables can outgrow the input, so their text is charged.
    budget.charge(variables.len())?;
    Ok(BodySpec::Graphql {
        query: apply(&query, trail, text_at, notes, budget)?,
        variables: apply(&variables, trail, text_at, notes, budget)?,
    })
}

/// Whether a leftover member holds anything: not `null`, nor an empty string, list or object.
fn holds_data(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(list) => !list.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

/// Encoding and redirects, set only when the request states either.
fn settings(encode_url: Option<&Member>, follow: Option<&Member>) -> Option<RequestSettings> {
    if encode_url.is_none() && follow.is_none() {
        return None;
    }
    let mut settings = RequestSettings::for_new_file();
    if let Some(encode) = encode_url.and_then(|m| m.value.as_bool()) {
        settings.encode_url = encode;
    }
    // `global` defers to the application setting, whose default is to follow.
    match follow.and_then(|m| m.value.as_str()) {
        Some("on") => settings.follow_redirects = true,
        Some("off") => settings.follow_redirects = false,
        _ => {}
    }
    Some(settings)
}

fn scripts(
    scripts: Vec<Member>,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<(), DocumentError> {
    for script in scripts {
        if !text(Some(&script.value)).trim().is_empty() {
            let detail = "scripts are not imported";
            notes.push(budget.note(trail, &script.path, NoteKind::Dropped, detail)?);
        }
    }
    Ok(())
}
