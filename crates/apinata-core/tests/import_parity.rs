//! Postman import parity with Bruno's converter over the `2026-10-09-import-parity` corpus.
//! A file no note concerns must match Bruno's bytes; noted files and the report are pinned by
//! insta snapshots; every other divergence is listed with its reason in `deviations.json`.

use apinata_core::import::{
    ImportReport, NoteKind, add_environment, parse_postman, parse_postman_environment,
    write_collection,
};
use apinata_core::{Collection, MemoryStore};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    kind: String,
    input: String,
    error: Option<String>,
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn corpus() -> PathBuf {
    manifest().join("tests/fixtures/reference/2026-10-09-import-parity")
}

fn input(case: &Case) -> Vec<u8> {
    fs::read(manifest().join("tests/fixtures/postman").join(&case.input)).expect("input")
}

/// Every file under `root`, keyed by its `/`-separated path relative to `root`.
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .expect("under root")
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.insert(rel, fs::read(&path).expect("read"));
        }
    }
    out
}

/// Import one case into `dir`: the written files and the report.
fn import(case: &Case, dir: &Path) -> (BTreeMap<String, Vec<u8>>, ImportReport) {
    let store = MemoryStore::new();
    if case.kind == "environment" {
        let env = parse_postman_environment(&input(case)).expect("parses");
        let mut collection = Collection::create(&dir.join("c"), "c").expect("create");
        let (_, report) = add_environment(&mut collection, &env, &store).expect("adds");
        let all = files(collection.root());
        (
            all.into_iter()
                .filter(|(p, _)| p.starts_with("environments/"))
                .collect(),
            report,
        )
    } else {
        let imported = parse_postman(&input(case)).expect("parses");
        let (collection, report) = write_collection(dir, &imported, &[], &store).expect("writes");
        (files(collection.root()), report)
    }
}

/// Bruno's bytes with each request's `timeout: 0` read as `inherit`: Postman has no request
/// timeout, so Apinata imports take the default one.
fn with_inherited_timeouts(files: BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    files
        .into_iter()
        .map(|(path, bytes)| match String::from_utf8(bytes) {
            Ok(text) => {
                let text = text.replace("\n  timeout: 0\n", "\n  timeout: inherit\n");
                (path, text.into_bytes())
            }
            Err(e) => (path, e.into_bytes()),
        })
        .collect()
}

#[test]
fn postman_imports_match_bruno_or_are_noted_or_listed() {
    let cases: Vec<Case> =
        serde_json::from_str(&fs::read_to_string(corpus().join("cases.json")).expect("cases"))
            .expect("valid");
    let listed: BTreeMap<String, String> = serde_json::from_str(
        &fs::read_to_string(manifest().join("tests/fixtures/postman/deviations.json"))
            .expect("deviations"),
    )
    .expect("valid");
    let mut divergent = BTreeSet::new();
    for case in &cases {
        if case.error.is_some() {
            let failed = match case.kind.as_str() {
                "environment" => parse_postman_environment(&input(case)).is_err(),
                _ => parse_postman(&input(case)).is_err(),
            };
            assert!(
                failed,
                "{}: Bruno rejects this input, so must Apinata",
                case.id
            );
            continue;
        }
        let dir = TempDir::new().expect("tempdir");
        let (ours, report) = import(case, dir.path());
        let theirs = with_inherited_timeouts(files(&corpus().join("expected").join(&case.id)));
        // A carried script is Bruno's own output, so its notes do not excuse a file.
        let noted: BTreeSet<&str> = report
            .notes
            .iter()
            .filter(|n| !(n.kind == NoteKind::Converted && n.source.contains("event[")))
            .filter_map(|n| n.file.as_deref())
            .collect();
        let mut snapshot = String::new();
        for note in &report.notes {
            snapshot.push_str(&format!(
                "{:?} {} [{}]: {}\n",
                note.kind,
                note.source,
                note.file.as_deref().unwrap_or("-"),
                note.detail
            ));
        }
        for path in ours.keys().chain(theirs.keys()).collect::<BTreeSet<_>>() {
            let (a, b) = (ours.get(path), theirs.get(path));
            if noted.contains(path.as_str()) {
                let text = a.map_or("(not written)\n".into(), |a| {
                    String::from_utf8_lossy(a).into_owned()
                });
                snapshot.push_str(&format!("== {path}\n{text}"));
            } else if a != b {
                divergent.insert(format!("{}/{}", case.id, path));
            }
        }
        if !snapshot.is_empty() {
            insta::assert_snapshot!(format!("import_{}", case.id), snapshot);
        }
    }
    let unlisted: Vec<_> = divergent
        .iter()
        .filter(|d| !listed.contains_key(*d))
        .collect();
    let stale: Vec<_> = listed.keys().filter(|d| !divergent.contains(*d)).collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "unlisted divergences (fix them or list them with a reason): {unlisted:#?}\nlisted but now matching (remove them): {stale:#?}"
    );
}
