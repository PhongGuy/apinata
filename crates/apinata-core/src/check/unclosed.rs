//! Warns about `{{name}` placeholders that the resolver leaves in the text as they are.

use std::path::Path;

use yaml_serde::{Mapping, Value};

use super::relative;
use super::report::{Finding, FindingKind, Severity};
use crate::collection::Folder;
use crate::interpolate::unclosed_placeholder;
use crate::{Collection, js};

/// One warning per unclosed placeholder in the fields a request expands, for every request
/// in the tree, plus one per placeholder in the headers and auth of each `folder.yml` and of
/// the collection file, which every request below them inherits. The raw files are read, so
/// the result does not depend on an environment.
pub(super) fn check_unclosed(collection: &Collection, folder: &Folder, out: &mut Vec<Finding>) {
    if std::ptr::eq(folder, collection.tree()) {
        let file = collection.file();
        let root = file.document().get(&["request"]);
        report(collection, "collection", file.document().path(), root, out);
    }
    if let Some(file) = folder.file() {
        let root = file.document().get(&["request"]);
        report(collection, "folder", file.document().path(), root, out);
    }
    for req in folder.requests() {
        let root = req.document().get(&[req.root_key()]);
        let found = scan_request(root);
        push_findings(collection, "request", req.path(), found, out);
    }
    for sub in folder.folders() {
        check_unclosed(collection, sub, out);
    }
}

/// The inherited settings of a folder or the collection: headers and auth only.
fn report(
    collection: &Collection,
    what: &str,
    path: &Path,
    root: Option<&Value>,
    out: &mut Vec<Finding>,
) {
    let mut found = Found::default();
    if let Some(Value::Mapping(root)) = root {
        scan_headers_and_auth(root, &mut found);
    }
    push_findings(collection, what, path, found.0, out);
}

fn push_findings(
    collection: &Collection,
    what: &str,
    path: &Path,
    found: Vec<(String, String)>,
    out: &mut Vec<Finding>,
) {
    let item = relative(collection, path);
    for (field, name) in found {
        out.push(Finding::new(
            item.clone(),
            None,
            Severity::Warning,
            FindingKind::UnclosedPlaceholder,
            format!(
                "`{{{{{name}}}` in the {field} of this {what} has no closing `}}}}`; \
                 it is sent as literal text"
            ),
        ));
    }
}

/// The distinct (field label, variable name) pairs found so far, in file order.
#[derive(Default)]
struct Found(Vec<(String, String)>);

impl Found {
    fn scan(&mut self, field: String, value: Option<&Value>) {
        let text = value.and_then(js::yaml_to_string).unwrap_or_default();
        if let Some(name) = unclosed_placeholder(&text) {
            let hit = (field, name.to_owned());
            if !self.0.contains(&hit) {
                self.0.push(hit);
            }
        }
    }
}

/// Headers and auth, the two settings a folder and the collection can also carry. Every auth
/// field except the mode is expanded by some auth kind, so all are read, labelled by their
/// top-level key.
fn scan_headers_and_auth(root: &Mapping, found: &mut Found) {
    for row in rows(root.get("headers")) {
        let name = row.get("name").and_then(js::yaml_to_string);
        let label = format!("header `{}`", name.as_deref().unwrap_or_default());
        found.scan(label.clone(), row.get("name"));
        found.scan(label, row.get("value"));
    }
    if let Some(Value::Mapping(auth)) = root.get("auth") {
        for (key, value) in auth {
            let key = key.as_str().unwrap_or_default();
            if key == "type" {
                continue;
            }
            for leaf in string_leaves(value) {
                found.scan(format!("auth `{key}`"), Some(leaf));
            }
        }
    }
}

/// The (field label, variable name) of each distinct unclosed placeholder, in file order.
/// Scripts, the method and file paths are not expanded as text, so they are not read.
fn scan_request(root: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Mapping(root)) = root else {
        return Vec::new();
    };
    let mut found = Found::default();
    found.scan("url".to_owned(), root.get("url"));
    scan_headers_and_auth(root, &mut found);
    for row in rows(root.get("params")) {
        let kind = row.get("type").and_then(Value::as_str).unwrap_or("query");
        let name = row.get("name").and_then(js::yaml_to_string);
        let label = format!("{kind} param `{}`", name.as_deref().unwrap_or_default());
        found.scan(label, row.get("value"));
    }
    if let Some(Value::Mapping(body)) = root.get("body") {
        let data = body.get("data");
        match body.get("type").and_then(Value::as_str) {
            Some("json" | "xml" | "text") => found.scan("body".to_owned(), data),
            Some("form-urlencoded") => {
                for row in rows(data) {
                    found.scan("body".to_owned(), row.get("name"));
                    found.scan("body".to_owned(), row.get("value"));
                }
            }
            // A file row's value is a path, which is not expanded as text.
            Some("multipart-form") => {
                for row in rows(data) {
                    if row.get("type").and_then(Value::as_str) == Some("text") {
                        found.scan("body".to_owned(), row.get("value"));
                    }
                }
            }
            None => {
                found.scan("body".to_owned(), body.get("query"));
                found.scan("body".to_owned(), body.get("variables"));
            }
            Some(_) => {}
        }
    }
    found.0
}

/// The enabled rows of a list of name/value rows.
fn rows(value: Option<&Value>) -> impl Iterator<Item = &Value> {
    let rows = match value {
        Some(Value::Sequence(rows)) => rows.as_slice(),
        _ => &[],
    };
    rows.iter().filter(|row| !js::is_disabled(row))
}

/// Every scalar under `value`, however deeply nested.
fn string_leaves(value: &Value) -> Vec<&Value> {
    match value {
        Value::Mapping(map) => map.values().flat_map(string_leaves).collect(),
        Value::Sequence(items) => items.iter().flat_map(string_leaves).collect(),
        leaf => vec![leaf],
    }
}
