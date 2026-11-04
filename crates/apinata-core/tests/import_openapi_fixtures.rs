//! Every OpenAPI fixture imports and writes the reviewed tree and report (spec criterion 2).

use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionOptions, ImportBundle, ImportedItem, parse_collections, write_collection,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// The OpenAPI fixtures 4f's detection matrix leaves undetected.
const UNDETECTED: &[&str] = &[
    "bruno/openapi-broken.json",
    "bruno/openapi-empty.yaml",
    "bruno/openapi-invalid-extension.txt",
    "bruno/openapi-malformed.yaml",
    "bruno/openapi-multifile-endpoint.yaml",
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("dir") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            walk(&path, out);
        } else if !matches!(name.as_str(), "NOTICE.md" | "LICENSE" | "LICENSE.md")
            // The spec-update fixtures are this project's own and tested by import_update_*.
            && !name.starts_with("update-")
        {
            out.push(path);
        }
    }
}

/// Every file under `root`, keyed by its `/`-separated relative path.
fn files(root: &Path) -> BTreeMap<String, String> {
    let mut list = Vec::new();
    walk(root, &mut list);
    list.into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            (rel, fs::read_to_string(&p).expect("utf-8"))
        })
        .collect()
}

/// A note's detail without its backtick- or double-quoted spans: the text the importer wrote.
fn fixed_text(detail: &str) -> String {
    let unticked: String = detail.split('`').step_by(2).collect();
    unticked.split('"').step_by(2).collect()
}

fn render(bundles: &[ImportBundle]) -> String {
    let mut out = String::new();
    for bundle in bundles {
        let parent = TempDir::new().expect("tempdir");
        let (collection, report) = write_collection(
            parent.path(),
            &bundle.collection,
            &bundle.environments,
            &MemoryStore::new(),
        )
        .expect("writes");
        for (path, text) in files(collection.root()) {
            out.push_str(&format!("=== {path}\n{text}\n"));
        }
        for n in &report.notes {
            assert!(
                !fixed_text(&n.detail).to_lowercase().contains("bruno"),
                "{}",
                n.detail
            );
            out.push_str(&format!(
                "note {:?} {} | {} | {}\n",
                n.kind,
                n.file.as_deref().unwrap_or("-"),
                n.source,
                n.detail
            ));
        }
        for (env, var) in &report.secrets_to_enter {
            out.push_str(&format!("secret {env} {var}\n"));
        }
    }
    out
}

#[test]
fn every_openapi_fixture_matches_its_snapshot() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi");
    let mut list = Vec::new();
    walk(&root, &mut list);
    list.sort();
    let mut seen = 0;
    for path in list {
        let rel = path
            .strip_prefix(&root)
            .expect("under root")
            .to_string_lossy()
            .replace('\\', "/");
        if UNDETECTED.contains(&rel.as_str()) {
            continue;
        }
        let rendered = match parse_collections(
            None,
            &fs::read(&path).expect("read"),
            &CollectionOptions::default(),
        ) {
            Ok(bundles) => render(&bundles),
            Err(e) => format!("error: {e}\n"),
        };
        if rel == "bruno/openapi-invalid-version.yaml" {
            assert_eq!(rendered, "error: OpenAPI: unsupported openapi 2.0\n");
        }
        insta::assert_snapshot!(rel.replace('/', "__"), rendered);
        seen += 1;
    }
    assert_eq!(seen, 31, "fixture count changed");
}

/// Requests, and notes about cut samples, under `items`.
fn count(items: &[ImportedItem]) -> (usize, usize) {
    items
        .iter()
        .fold((0, 0), |(requests, cut), item| match item {
            ImportedItem::Folder(f) => {
                let (r, c) = count(&f.items);
                (requests + r, cut + c)
            }
            ImportedItem::Request(r) => (
                requests + 1,
                cut + r
                    .notes
                    .iter()
                    .filter(|n| n.detail.contains("sample"))
                    .count(),
            ),
            _ => (requests, cut),
        })
}

#[test]
#[ignore = "reads large public descriptions listed in APINATA_OPENAPI_SMOKE"]
fn large_public_descriptions_import() {
    let list = std::env::var("APINATA_OPENAPI_SMOKE")
        .expect("APINATA_OPENAPI_SMOKE: colon-separated paths");
    for path in list.split(':') {
        let started = std::time::Instant::now();
        let bundles = parse_collections(
            None,
            &fs::read(path).expect("read"),
            &CollectionOptions::default(),
        )
        .expect("imports");
        let (requests, cut) = count(&bundles[0].collection.items);
        println!(
            "{path}: {requests} requests, {cut} cut samples, {} ms",
            started.elapsed().as_millis()
        );
        assert_eq!(cut, 0, "{path}");
        if path.contains("api.github.com") {
            // R0's operation count, confirmed by the 2026-10-10 smoke run.
            assert_eq!(requests, 1232);
        }
    }
}
