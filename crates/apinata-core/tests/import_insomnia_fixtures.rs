//! Every vendored Insomnia fixture imports, is written, and matches its snapshot; no note or
//! error text names another tool.

use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionFormat, CollectionOptions, DocumentError, FormatError, ImportBundle, ImportNote,
    ImportReport, ImportedItem, parse_collections, write_collection,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Fixtures that must fail, and how.
const FAILS: &[(&str, &str)] = &[
    ("bruno/insomnia-malformed.json", "parse"),
    ("kong/global-environment.yaml", "unsupported"),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/insomnia")
}

/// `(sub/name, bytes)` of every fixture, licence files excluded, sorted.
fn fixtures() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for sub in ["bruno", "kong"] {
        let dir = root().join(sub);
        let mut names: Vec<String> = fs::read_dir(&dir)
            .expect("fixture dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with("LICENSE"))
            .collect();
        names.sort();
        for name in names {
            let bytes = fs::read(dir.join(&name)).expect("read");
            out.push((format!("{sub}/{name}"), bytes));
        }
    }
    out
}

fn parse(input: &[u8]) -> Result<Vec<ImportBundle>, FormatError> {
    parse_collections(
        Some(CollectionFormat::Insomnia),
        input,
        &CollectionOptions::default(),
    )
}

/// Every file under `dir` as `== <path>` and its text, sorted by path.
fn tree(dir: &Path) -> String {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).expect("dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(dir)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(rel, fs::read_to_string(&path).expect("utf-8"));
        }
    }
    files
        .into_iter()
        .map(|(p, t)| format!("== {p}\n{t}"))
        .collect()
}

fn report_text(report: &ImportReport) -> String {
    let line = |n: &ImportNote| {
        let file = n
            .file
            .as_deref()
            .map(|f| format!(" ({f})"))
            .unwrap_or_default();
        format!("{:?} {}: {}{file}\n", n.kind, n.source, n.detail)
    };
    report.notes.iter().map(line).collect()
}

/// The detail of every note a parse produced: collection, unwritten, items, environments.
fn details(bundles: &[ImportBundle]) -> Vec<String> {
    fn walk(items: &[ImportedItem], out: &mut Vec<String>) {
        for item in items {
            match item {
                ImportedItem::Folder(f) => {
                    out.extend(f.notes.iter().map(|n| n.detail.clone()));
                    walk(&f.items, out);
                }
                ImportedItem::Request(r) => out.extend(r.notes.iter().map(|n| n.detail.clone())),
                _ => unreachable!("only folders and requests exist"),
            }
        }
    }
    let mut out = Vec::new();
    for b in bundles {
        let c = &b.collection;
        out.extend(c.notes.iter().chain(&c.unwritten).map(|n| n.detail.clone()));
        walk(&c.items, &mut out);
        for env in &b.environments {
            out.extend(env.notes.iter().map(|n| n.detail.clone()));
        }
    }
    out
}

#[test]
fn every_fixture_imports_and_matches_its_snapshot() {
    let mut imported = 0;
    for (name, bytes) in fixtures() {
        let result = parse(&bytes);
        if let Some((_, kind)) = FAILS.iter().find(|(f, _)| *f == name) {
            let got = match &result {
                Err(FormatError::Insomnia(DocumentError::Parse { .. })) => "parse",
                Err(FormatError::Insomnia(DocumentError::Unsupported { .. })) => "unsupported",
                other => panic!("{name}: {other:?}"),
            };
            assert_eq!(got, *kind, "{name}");
            continue;
        }
        let bundles = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            CollectionFormat::detect(&bytes),
            Some(CollectionFormat::Insomnia),
            "{name}"
        );
        let mut text = String::new();
        for bundle in &bundles {
            let dir = TempDir::new().expect("tempdir");
            let (written, report) = write_collection(
                dir.path(),
                &bundle.collection,
                &bundle.environments,
                &MemoryStore::new(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            text.push_str(&format!(
                "# {}\n{}-- report\n{}",
                bundle.collection.name,
                tree(written.root()),
                report_text(&report)
            ));
        }
        insta::assert_snapshot!(format!("insomnia_{}", name.replace(['/', '.'], "_")), text);
        imported += 1;
    }
    assert_eq!(imported, 15, "17 fixtures, 2 refused");
}

#[test]
fn multiple_workspaces_give_two_collections_with_their_own_environments() {
    let bytes = fs::read(root().join("kong/multiple-workspaces.yaml")).expect("read");
    let bundles = parse(&bytes).expect("parses");
    let shape: Vec<(String, Vec<String>)> = bundles
        .iter()
        .map(|b| {
            (
                b.collection.name.clone(),
                b.environments.iter().map(|e| e.name.clone()).collect(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            (
                "Collection 1".to_owned(),
                vec![
                    "Base Environment 1".to_owned(),
                    "Sub Environment 1".to_owned()
                ]
            ),
            (
                "Collection 2".to_owned(),
                vec![
                    "Base Environment 2".to_owned(),
                    "Sub Environment 2".to_owned()
                ]
            ),
        ]
    );
}

#[test]
fn no_note_or_error_names_another_tool() {
    let mut texts = Vec::new();
    for (_, bytes) in fixtures() {
        match parse(&bytes) {
            Ok(bundles) => texts.extend(details(&bundles)),
            Err(e) => texts.push(e.to_string()),
        }
    }
    for bad in [
        r#"{"_type":"export","__export_format":3,"resources":[]}"#,
        r#"{"_type":"export","__export_format":4,"resources":{}}"#,
        r#"{"_type":"export","__export_format":4,"resources":[]}"#,
        "type: mock.insomnia.rest/5.0\n",
        "{",
    ] {
        texts.push(parse(bad.as_bytes()).expect_err(bad).to_string());
    }
    assert!(texts.len() > 10, "only {} texts", texts.len());
    for text in &texts {
        assert!(!text.to_lowercase().contains("bruno"), "{text}");
    }
}
