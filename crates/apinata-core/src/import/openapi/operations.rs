//! The operation walk: each method of each path item becomes one request.

use super::Doc;
use super::refs::RefKind;
use super::version::Version;
use crate::import::{DocumentError, ImportNote, ImportedRequest, ImportedRequestItem, NoteKind};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Path-item keys that hold operations, lower-cased, with the method each sends.
const METHODS: [(&str, &str); 8] = [
    ("get", "GET"),
    ("put", "PUT"),
    ("post", "POST"),
    ("delete", "DELETE"),
    ("options", "OPTIONS"),
    ("head", "HEAD"),
    ("patch", "PATCH"),
    ("trace", "TRACE"),
];

/// Whether `collect` reads a path-item key, as a method (in any case) or as a noted 3.2 key.
pub(crate) fn is_operation_key(key: &str, version: Version) -> bool {
    let method = METHODS
        .iter()
        .any(|(name, _)| key.eq_ignore_ascii_case(name));
    let trace = key.eq_ignore_ascii_case("trace");
    (method && !(trace && version == Version::Swagger2))
        || key.eq_ignore_ascii_case("query")
        || key.eq_ignore_ascii_case("additionalOperations")
}

/// One operation found in `paths`, with the path item it belongs to.
pub(crate) struct Operation<'a> {
    /// The upper-case HTTP method.
    pub(crate) method: &'static str,
    /// The path as written in the document.
    pub(crate) path: &'a str,
    /// The operation's document path, such as `paths./pets.get`.
    pub(crate) source: String,
    /// The resolved path item, shared by its operations so an owned merge is not copied.
    pub(crate) item: Rc<Cow<'a, Value>>,
    /// The path-item key holding the operation.
    pub(crate) key: String,
}

impl Operation<'_> {
    /// The operation object itself.
    pub(crate) fn op(&self) -> &Value {
        &self.item[self.key.as_str()]
    }
}

/// Every operation in document order. A missing `paths` has none; a `paths` that is not an
/// object is `Invalid`. Notes from resolving path items go to `unwritten`.
pub(crate) fn collect<'a>(
    doc: &mut Doc<'a>,
    unwritten: &mut Vec<ImportNote>,
) -> Result<Vec<Operation<'a>>, DocumentError> {
    let paths = match doc.root.get("paths") {
        None => return Ok(Vec::new()),
        Some(Value::Object(paths)) => paths,
        Some(_) => {
            return Err(DocumentError::Invalid {
                message: "`paths` is not an object".to_owned(),
            });
        }
    };
    let mut out = Vec::new();
    for (path, raw) in paths {
        let source = format!("paths.{path}");
        let item = Rc::new(doc.resolve(raw, RefKind::Object, &source)?);
        unwritten.extend(doc.take_ref_notes());
        // Keys other than methods are skipped silently, save the OpenAPI 3.2 additions and
        // method keys whose value is not an object, which get notes. Keys match in any case;
        // Swagger 2.0 has no `trace`.
        let mut found: Vec<(&'static str, String)> = Vec::new();
        for (key, value) in item.as_object().into_iter().flatten() {
            let lower = key.to_ascii_lowercase();
            let method = METHODS
                .iter()
                .find(|(name, _)| *name == lower)
                .filter(|(name, _)| !(*name == "trace" && doc.version == Version::Swagger2));
            let detail = match (lower.as_str(), method) {
                ("query", _) => "the query method (OpenAPI 3.2) is not imported",
                ("additionaloperations", _) => "additionalOperations (OpenAPI 3.2) is not imported",
                (_, Some((_, method))) if value.is_object() => {
                    found.push((method, key.clone()));
                    continue;
                }
                (_, Some(_)) => "not an operation object, so no request was made",
                (_, None) => continue,
            };
            let note_source = format!("{source}.{key}");
            doc.budget.charge(1)?;
            doc.budget
                .charge_bytes((note_source.len() + detail.len()) as u64)?;
            unwritten.push(ImportNote::new(note_source, NoteKind::Dropped, detail));
        }
        for (method, key) in found {
            out.push(Operation {
                method,
                path,
                source: format!("{source}.{key}"),
                item: Rc::clone(&item),
                key,
            });
        }
    }
    Ok(out)
}

/// The request for `op`, with `seq` 0 for the caller to place.
pub(crate) fn request_for<'a: 'o, 'o>(
    doc: &mut Doc<'a>,
    op: &'o Operation<'_>,
) -> Result<ImportedRequestItem, DocumentError> {
    let object = op.op();
    let text = |key: &str| object.get(key).and_then(Value::as_str);
    let mut notes = doc.take_ref_notes();
    doc.budget.charge(1)?;
    let vars = super::servers::request_vars(doc, op, &mut notes)?;
    // Everything copied out of the document is charged first: the name's source, the URL
    // (`{{baseUrl}}` plus the path), the docs and the tags.
    let tags: Vec<&str> = object
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    let chosen = ["summary", "operationId", "description"]
        .into_iter()
        .filter_map(text)
        .find(|s| !s.is_empty());
    let docs = text("description").unwrap_or_default();
    let copied = chosen.map_or(op.path.len() + 8, str::len)
        + "{{baseUrl}}".len()
        + 2 * op.path.len()
        + docs.len()
        + tags.iter().map(|t| t.len()).sum::<usize>();
    doc.budget.charge_bytes(copied as u64)?;
    let colon_path = colon_params(op.path);
    // The first non-empty candidate is chosen before normalizing, so one that normalizes to
    // nothing falls back to the method and path rather than to the next candidate.
    let name = chosen
        .map(normalize_name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{} {colon_path}", op.method.to_ascii_lowercase()));
    let url = single_slashes(&format!("{{{{baseUrl}}}}{colon_path}"));
    let mut request = ImportedRequest::new(op.method, url);
    request.docs = docs.to_owned();
    request.tags = tags.into_iter().map(str::to_owned).collect();
    request.vars = vars;
    super::params::apply(doc, op, &mut request, &mut notes)?;
    super::body::apply(doc, op, &mut request, &mut notes)?;
    request.auth = super::security::operation_auth(doc, op, &mut notes)?;
    super::coverage::operation_notes(doc, op, &mut notes)?;
    Ok(ImportedRequestItem {
        name,
        seq: 0,
        request,
        notes,
    })
}

/// Collapse whitespace runs to one space, trim, then cut trailing dots and spaces.
pub(crate) fn normalize_name(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_end_matches(['.', ' ']).to_owned()
}

/// Give each request in one sibling list a distinct name: the name itself, else with its
/// method appended, else with the first free counter. Each rename notes the operation and
/// charges that note to the budget.
pub(crate) fn dedupe(
    doc: &mut Doc<'_>,
    siblings: &mut [(Operation<'_>, ImportedRequestItem)],
) -> Result<(), DocumentError> {
    let mut taken = HashSet::new();
    // The next counter to try per name, so many requests with one name are renamed in
    // linear time rather than each rescanning from 1.
    let mut next: HashMap<String, u64> = HashMap::new();
    for (op, request) in siblings {
        if taken.insert(request.name.clone()) {
            continue;
        }
        // A rename copies the name into the new name and into its note.
        doc.budget.charge(1)?;
        doc.budget
            .charge_bytes(2 * (request.name.len() as u64 + 64) + op.source.len() as u64)?;
        let by_method = format!("{} ({})", request.name, op.method);
        let name = if taken.insert(by_method.clone()) {
            by_method
        } else {
            let mut counter = next.get(&request.name).copied().unwrap_or(1);
            let name = loop {
                let candidate = format!("{} ({counter})", request.name);
                counter += 1;
                if !taken.contains(&candidate) {
                    break candidate;
                }
            };
            next.insert(request.name.clone(), counter);
            taken.insert(name.clone());
            name
        };
        request.notes.push(ImportNote::new(
            op.source.as_str(),
            NoteKind::Renamed,
            format!("renamed to `{name}` because another request here has the same name"),
        ));
        request.name = name;
    }
    Ok(())
}

/// Collapse each run of two or more `/` to one, except after `:` as in `https://`.
fn single_slashes(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    let mut keep_run = false;
    for c in url.chars() {
        if c == '/' && out.ends_with('/') && !keep_run {
            continue;
        }
        if c == '/' && !out.ends_with('/') {
            keep_run = out.ends_with(':');
        }
        out.push(c);
    }
    out
}

/// `path` with each `{x}` template written as `:x`, the form request URLs use.
fn colon_params(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        out.push_str(&rest[..open]);
        let name = &rest[open + 1..open + close];
        if name.is_empty() {
            out.push_str("{}");
        } else {
            out.push(':');
            out.push_str(name);
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}
