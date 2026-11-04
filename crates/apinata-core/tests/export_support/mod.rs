//! Helpers shared by the export tests.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::Collection;
use apinata_core::import::{ImportedItem, ImportedRequestItem};
use apinata_core::{Folder, RequestFile};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: c\n";
pub const INHERIT: &str = "  auth: inherit\n";

/// Open a collection written from (relative path, text) pairs; load errors are allowed.
pub fn open(files: &[(&str, &str)]) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

/// A request file at seq 1; `under_http` goes inside `http:` after the method and URL.
pub fn http(name: &str, method: &str, url: &str, under_http: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: {method}\n  url: \"{url}\"\n{under_http}"
    )
}

pub fn exported(c: &Collection) -> Value {
    serde_json::from_str(&apinata_core::export::postman(c).json).expect("valid JSON")
}

pub fn notes(c: &Collection) -> Vec<(String, String)> {
    let out = apinata_core::export::postman(c);
    out.notes
        .into_iter()
        .map(|n| (n.source, n.detail))
        .collect()
}

pub fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

pub fn keys(v: &Value) -> Vec<&str> {
    v.as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect()
}

/// The names of the items of `v["item"]`, in order.
pub fn names(v: &Value) -> Vec<&str> {
    v["item"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["name"].as_str().expect("name"))
        .collect()
}

/// The collections the round trips and snapshots cover, by label.
pub fn corpus() -> Vec<(&'static str, PathBuf)> {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture = |name: &str| core.join("tests/fixtures/bruno").join(name);
    vec![
        ("demo", core.join("../../examples/demo")),
        ("bruno-tests", fixture("bruno-tests")),
        ("cli-docs", fixture("cli-docs")),
        ("env-inheritance", fixture("env-inheritance")),
    ]
}

fn crumb(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix} / {name}")
    }
}

/// Every request, named by its folder names and its own, sorted by that name.
pub fn source_requests<'a>(folder: &'a Folder, prefix: &str) -> Vec<(String, &'a RequestFile)> {
    let mut out = Vec::new();
    for sub in folder.folders() {
        out.extend(source_requests(sub, &crumb(prefix, sub.name())));
    }
    out.extend(
        folder
            .requests()
            .iter()
            .map(|r| (crumb(prefix, r.name()), r)),
    );
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Every imported request, named and sorted the same way.
pub fn imported_requests<'a>(
    items: &'a [ImportedItem],
    prefix: &str,
) -> Vec<(String, &'a ImportedRequestItem)> {
    let mut out = Vec::new();
    for item in items {
        match item {
            ImportedItem::Folder(f) => {
                out.extend(imported_requests(&f.items, &crumb(prefix, &f.name)))
            }
            ImportedItem::Request(r) => out.push((crumb(prefix, &r.name), r)),
            _ => {}
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A request file at `seq`; `under_http` goes inside `http:` after the method and URL.
pub fn http_at(name: &str, seq: u32, method: &str, url: &str, under_http: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: {method}\n  url: \"{url}\"\n{under_http}"
    )
}

/// `open` with `root` as `opencollection.yml` plus owned (path, text) pairs.
pub fn open_with_root(root: &str, files: &[(String, String)]) -> (TempDir, Collection) {
    let mut all = vec![("opencollection.yml", root)];
    all.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    open(&all)
}

/// Panics unless oas3 reads `json` as an OpenAPI 3.1.0 document.
pub fn assert_valid_openapi(json: &str) {
    let spec = oas3::from_json(json).unwrap_or_else(|e| panic!("oas3 rejects it: {e:?}\n{json}"));
    let version = spec.validate_version().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(version.to_string(), "3.1.0");
}

/// `export::openapi(c)` parsed, after `assert_valid_openapi`.
pub fn openapi_doc(c: &Collection) -> Value {
    let out = apinata_core::export::openapi(c);
    assert_valid_openapi(&out.json);
    serde_json::from_str(&out.json).expect("valid JSON")
}

pub fn openapi_notes(c: &Collection) -> Vec<(String, String)> {
    let out = apinata_core::export::openapi(c);
    out.notes
        .into_iter()
        .map(|n| (n.source, n.detail))
        .collect()
}

/// Every (path, method) of an OpenAPI document, sorted.
pub fn operations(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (path, item) in v["paths"].as_object().expect("paths") {
        out.extend(
            item.as_object()
                .expect("path item")
                .keys()
                .map(|m| (path.clone(), m.clone())),
        );
    }
    out.sort();
    out
}

/// The three injection payloads in one line, single-quoted for YAML.
pub const HOSTILE_YAML: &str =
    "'<script>alert(1)</script> \"><img src=x onerror=alert(1)> [x](javascript:alert(1))'";
/// The same line as the escaper writes it.
pub const HOSTILE_ESCAPED: &str = "&lt;script&gt;alert(1)&lt;/script&gt; &quot;&gt;&lt;img src=x onerror=alert(1)&gt; [x](javascript:alert(1))";
/// A top-level `docs` key holding the payloads as three markdown paragraphs.
pub const HOSTILE_DOCS: &str = "docs: |-\n  <script>alert(1)</script>\n\n  \"><img src=x onerror=alert(1)>\n\n  [x](javascript:alert(1))\n";

pub fn html_page(c: &Collection) -> String {
    apinata_core::export::html(c).html
}

pub fn html_notes(c: &Collection) -> Vec<(String, String)> {
    let out = apinata_core::export::html(c);
    out.notes
        .into_iter()
        .map(|n| (n.source, n.detail))
        .collect()
}

/// Panics if a payload became live markup: a script or img tag, or a `javascript:` link.
pub fn assert_inert(page: &str) {
    let lower = page.to_ascii_lowercase();
    assert!(!lower.contains("<script"), "{page}");
    assert!(!lower.contains("<img"), "{page}");
    assert!(!lower.contains("href=\"javascript:"), "{page}");
    assert!(!lower.contains("href='javascript:"), "{page}");
}

/// A `folder.yml`; `under_request` goes inside `request:` and is left out when empty.
pub fn folder_yml(name: &str, seq: u32, under_request: &str) -> String {
    let request = if under_request.is_empty() {
        String::new()
    } else {
        format!("request:\n{under_request}")
    };
    format!("info:\n  name: {name}\n  type: folder\n  seq: {seq}\n{request}")
}

/// The article with `id`, from its opening tag up to its closing tag.
pub fn article<'a>(page: &'a str, id: &str) -> &'a str {
    let start = page
        .find(&format!("<article id=\"{id}\">"))
        .unwrap_or_else(|| panic!("no {id} in {page}"));
    let end = start + page[start..].find("</article>\n").expect("closed");
    &page[start..end]
}
