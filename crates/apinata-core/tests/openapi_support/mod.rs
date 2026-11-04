//! Helpers shared by the OpenAPI importer tests.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::ParamKind;
use apinata_core::import::{
    CollectionFormat, CollectionOptions, DocumentError, FormatError, GroupBy, ImportBundle,
    ImportNote, ImportedCollection, ImportedFolder, ImportedItem, ImportedRequestItem,
    parse_collections,
};
use serde_json::{Value, json};

/// Import `input` as OpenAPI, unwrapping the format layer of the error.
pub fn import_bytes(input: &[u8], group_by: GroupBy) -> Result<Vec<ImportBundle>, DocumentError> {
    let mut options = CollectionOptions::default();
    options.group_by = group_by;
    match parse_collections(Some(CollectionFormat::OpenApi), input, &options) {
        Ok(bundles) => Ok(bundles),
        Err(FormatError::OpenApi(e)) => Err(e),
        Err(other) => panic!("not an OpenAPI error: {other}"),
    }
}

pub fn error(doc: &Value) -> DocumentError {
    import_bytes(doc.to_string().as_bytes(), GroupBy::Tags).expect_err("refused")
}

/// The single bundle `doc` imports to.
pub fn import_by(doc: &Value, group_by: GroupBy) -> ImportBundle {
    let mut bundles = import_bytes(doc.to_string().as_bytes(), group_by).expect("imports");
    assert_eq!(bundles.len(), 1, "one bundle");
    bundles.remove(0)
}

pub fn import(doc: &Value) -> ImportBundle {
    import_by(doc, GroupBy::Tags)
}

fn walk<'a>(
    items: &'a [ImportedItem],
    prefix: &str,
    depth: usize,
    out: &mut Vec<(String, usize, &'a ImportedItem)>,
) {
    for item in items {
        let name = match item {
            ImportedItem::Folder(f) => &f.name,
            ImportedItem::Request(r) => &r.name,
            _ => unreachable!("only folders and requests exist"),
        };
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        out.push((path.clone(), depth, item));
        if let ImportedItem::Folder(f) = item {
            walk(&f.items, &path, depth + 1, out);
        }
    }
}

fn items(c: &ImportedCollection) -> Vec<(String, usize, &ImportedItem)> {
    let mut out = Vec::new();
    walk(&c.items, "", 0, &mut out);
    out
}

/// Depth first: `F <seq> <name>` or `R <seq> <name> <METHOD> <url>`, two spaces per level.
pub fn tree(c: &ImportedCollection) -> Vec<String> {
    items(c)
        .into_iter()
        .map(|(_, depth, item)| {
            let pad = "  ".repeat(depth);
            match item {
                ImportedItem::Folder(f) => format!("{pad}F {} {}", f.seq, f.name),
                ImportedItem::Request(r) => format!(
                    "{pad}R {} {} {} {}",
                    r.seq, r.name, r.request.method, r.request.url
                ),
                _ => unreachable!("only folders and requests exist"),
            }
        })
        .collect()
}

/// Every request with its path: folder names and its own name joined by `/`.
pub fn requests(c: &ImportedCollection) -> Vec<(String, &ImportedRequestItem)> {
    items(c)
        .into_iter()
        .filter_map(|(path, _, item)| match item {
            ImportedItem::Request(r) => Some((path, r)),
            _ => None,
        })
        .collect()
}

pub fn request<'a>(c: &'a ImportedCollection, path: &str) -> &'a ImportedRequestItem {
    let all = requests(c);
    let names: Vec<String> = all.iter().map(|(p, _)| p.clone()).collect();
    all.into_iter()
        .find(|(p, _)| p == path)
        .map(|(_, r)| r)
        .unwrap_or_else(|| panic!("no request {path} in {names:?}"))
}

pub fn folder<'a>(c: &'a ImportedCollection, path: &str) -> &'a ImportedFolder {
    items(c)
        .into_iter()
        .find_map(|(p, _, item)| match item {
            ImportedItem::Folder(f) if p == path => Some(f),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no folder {path}"))
}

/// Every note: the collection's, the unwritten ones, then folders' and requests' depth first.
pub fn notes(c: &ImportedCollection) -> Vec<&ImportNote> {
    let mut out: Vec<&ImportNote> = c.notes.iter().chain(&c.unwritten).collect();
    for (_, _, item) in items(c) {
        match item {
            ImportedItem::Folder(f) => out.extend(&f.notes),
            ImportedItem::Request(r) => out.extend(&r.notes),
            _ => unreachable!("only folders and requests exist"),
        }
    }
    out
}

pub const Q: ParamKind = ParamKind::Query;
pub const P: ParamKind = ParamKind::Path;

/// A request's params as `(name, value, kind, enabled)`.
pub fn params(r: &ImportedRequestItem) -> Vec<(String, String, ParamKind, bool)> {
    r.request
        .params
        .iter()
        .map(|p| (p.name.clone(), p.value.clone(), p.kind, p.enabled))
        .collect()
}

/// A request's headers as `(name, value, enabled)`.
pub fn headers(r: &ImportedRequestItem) -> Vec<(String, String, bool)> {
    r.request
        .headers
        .iter()
        .map(|h| (h.name.clone(), h.value.clone(), h.enabled))
        .collect()
}

pub fn p(
    name: &str,
    value: &str,
    kind: ParamKind,
    enabled: bool,
) -> (String, String, ParamKind, bool) {
    (name.to_owned(), value.to_owned(), kind, enabled)
}

pub fn h(name: &str, value: &str, enabled: bool) -> (String, String, bool) {
    (name.to_owned(), value.to_owned(), enabled)
}

/// A 3.1 document whose one request, `Q` at `GET /q`, has these parameters.
pub fn one(parameters: Value) -> Value {
    json!({"openapi": "3.1.0", "servers": [{"url": "https://x.test"}], "paths": {"/q": {"get": {"summary": "Q", "parameters": parameters}}}})
}
