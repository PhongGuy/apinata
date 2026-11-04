//! New-folder conversion: reference bytes on the corpus, a round trip through the model, refusals.

use apinata_core::legacy::{ConvertError, ConvertTarget, convert, plan_conversion};
use apinata_core::{Collection, VarValue};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const BRUNO_JSON: &str = r#"{"version": "1", "name": "Legacy", "type": "collection"}"#;
const REQUEST: &str =
    "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n";
const COLLECTIONS: [&str; 2] = ["bruno-tests", "response-examples"];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-14-bru-migration-parity")
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn legacy(root: &Path, bruno_json: &str) {
    write(root, "bruno.json", bruno_json);
    write(root, "a.bru", REQUEST);
    write(root, "f/folder.bru", "meta {\n  name: f\n}\n");
    write(root, "f/b.bru", &REQUEST.replace("name: a", "name: b"));
    write(root, ".env", "TOKEN=x\n");
}

fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn converted(name: &str, out: &Path) -> PathBuf {
    let plan = plan_conversion(&corpus().join("sources/collections").join(name))
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let target = out.join(name);
    assert_eq!(
        convert(&plan, ConvertTarget::NewFolder(target.clone())).unwrap(),
        target
    );
    target
}

#[test]
fn corpus_collections_convert_to_the_reference_bytes_and_copy_the_rest() {
    for name in COLLECTIONS {
        let out = tempfile::tempdir().unwrap();
        let target = converted(name, out.path());
        let source = corpus().join("sources/collections").join(name);
        let mut want = files(&corpus().join("expected/collections").join(name));
        for rel in plan_conversion(&source).unwrap().copied() {
            want.insert(rel.clone(), fs::read(source.join(&rel)).unwrap());
        }
        let got = files(&target);
        assert_eq!(
            got.keys().collect::<Vec<_>>(),
            want.keys().collect::<Vec<_>>(),
            "{name}"
        );
        for (rel, bytes) in &want {
            assert!(got[rel] == *bytes, "{name}/{} differs", rel.display());
        }
        assert_eq!(
            fs::read_dir(out.path()).unwrap().count(),
            1,
            "{name}: staging left behind"
        );
    }
}

#[test]
fn converted_corpus_collections_open_and_read_back_the_parsed_values() {
    for name in COLLECTIONS {
        let out = tempfile::tempdir().unwrap();
        let c = Collection::open(converted(name, out.path())).unwrap();
        assert!(c.load_errors().is_empty(), "{name}: {:?}", c.load_errors());
        let mut checked = 0;
        for (rel, bytes) in files(&corpus().join("parsed/collections").join(name)) {
            let rel = rel.to_string_lossy().into_owned();
            let Some(source) = rel.strip_suffix(".json").filter(|s| s.ends_with(".bru")) else {
                continue;
            };
            let source = Path::new(source);
            let json: Value =
                serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{name}: {rel}: {e}"));
            let parent = source.parent().unwrap_or(Path::new(""));
            if parent == Path::new("environments") {
                let stem = source
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                let vars = c
                    .environment(stem)
                    .unwrap_or_else(|| panic!("{name}: env {stem}"))
                    .variables();
                let want = json["variables"].as_array().cloned().unwrap_or_default();
                assert_eq!(vars.len(), want.len(), "{name}: {stem}");
                for (v, w) in vars.iter().zip(want) {
                    let at = format!("{name}: {stem}.{}", v.name);
                    assert_eq!(Some(v.name.as_str()), w["name"].as_str(), "{at}");
                    assert_eq!(v.enabled, w["enabled"].as_bool().unwrap_or(true), "{at}");
                    assert_eq!(v.secret, w["secret"].as_bool().unwrap_or(false), "{at}");
                    if v.secret {
                        continue;
                    }
                    // A string reads back as text; any other JSON value as typed data holding its JSON.
                    match (&v.value, &w["value"]) {
                        (VarValue::Text(t), Value::String(s)) => assert_eq!(t, s, "{at}"),
                        (VarValue::Typed { kind, data }, other) if !other.is_string() => {
                            assert_eq!(
                                Some(kind.as_str()),
                                w["annotations"][0]["name"].as_str(),
                                "{at}"
                            );
                            let read: Value =
                                serde_json::from_str(data).unwrap_or_else(|e| panic!("{at}: {e}"));
                            assert_eq!(&read, other, "{at}");
                        }
                        (got, want) => {
                            panic!("{at}: {} against {want}", matches!(got, VarValue::Text(_)))
                        }
                    }
                }
            } else if source.file_name() == Some("folder.bru".as_ref()) {
                if parent.as_os_str().is_empty() {
                    continue;
                }
                let ip = c
                    .item_path(&c.root().join(parent))
                    .unwrap_or_else(|| panic!("{name}: {rel}"));
                let folder = c
                    .folder(&ip)
                    .unwrap_or_else(|| panic!("{name}: {rel} is not a folder"));
                // A folder.bru without meta reads back under the reference's fallback name, which
                // the expected folder.yml records as its first `  name:` line (under `info:`).
                let want = match json["meta"]["name"].as_str() {
                    Some(n) => n.to_owned(),
                    None => {
                        let yml = corpus()
                            .join("expected/collections")
                            .join(name)
                            .join(parent)
                            .join("folder.yml");
                        let text = fs::read_to_string(&yml)
                            .unwrap_or_else(|e| panic!("{}: {e}", yml.display()));
                        text.lines()
                            .find_map(|l| l.strip_prefix("  name: "))
                            .unwrap_or_else(|| panic!("{}", yml.display()))
                            .to_owned()
                    }
                };
                assert_eq!(folder.name(), want, "{name}: {rel}");
            } else if let Some(http) = json
                .get("http")
                .filter(|_| source != Path::new("collection.bru"))
            {
                let ip = c
                    .item_path(&c.root().join(source.with_extension("yml")))
                    .unwrap_or_else(|| panic!("{name}: {rel}"));
                let req = c
                    .request(&ip)
                    .unwrap_or_else(|| panic!("{name}: {rel} is not a request"));
                if let Some(want) = json["meta"]["name"].as_str() {
                    assert_eq!(req.name(), want, "{name}: {rel}");
                }
                assert_eq!(
                    req.url(),
                    http["url"].as_str().unwrap_or(""),
                    "{name}: {rel}"
                );
                if let Some(method) = http["method"].as_str() {
                    assert!(req.method().eq_ignore_ascii_case(method), "{name}: {rel}");
                }
            } else {
                continue;
            }
            checked += 1;
        }
        assert!(checked > 0, "{name}: nothing compared");
    }
}

#[test]
fn new_folder_in_names_the_folder_like_an_import_and_leaves_the_source() {
    let src = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    legacy(src.path(), BRUNO_JSON);
    fs::create_dir(parent.path().join("Legacy")).unwrap();
    let before = files(src.path());
    let plan = plan_conversion(src.path()).unwrap();
    let out = convert(
        &plan,
        ConvertTarget::NewFolderIn(parent.path().to_path_buf()),
    )
    .unwrap();
    assert_eq!(out, parent.path().join("Legacy - 1"));
    assert!(out.join("opencollection.yml").is_file() && out.join("f/b.yml").is_file());
    assert_eq!(fs::read(out.join(".env")).unwrap(), b"TOKEN=x\n");
    assert!(!out.join("a.bru").exists() && !out.join("bruno.json").exists());
    assert_eq!(files(src.path()), before);
    let c = Collection::open(&out).unwrap();
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    assert_eq!(c.name(), "Legacy");
}

#[test]
fn new_folder_refusals_leave_the_parent_as_it_was() {
    let src = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    legacy(src.path(), BRUNO_JSON);
    let plan = plan_conversion(src.path()).unwrap();
    let taken = parent.path().join("taken");
    fs::create_dir(&taken).unwrap();
    let Err(ConvertError::TargetExists { paths }) =
        convert(&plan, ConvertTarget::NewFolder(taken.clone()))
    else {
        panic!("expected TargetExists");
    };
    assert_eq!(paths, [taken]);
    write(
        src.path(),
        "a.bru",
        &REQUEST.replace("example.com", "example.org"),
    );
    let Err(ConvertError::SourceChanged { path }) = convert(
        &plan,
        ConvertTarget::NewFolderIn(parent.path().to_path_buf()),
    ) else {
        panic!("expected SourceChanged");
    };
    assert_eq!(path, Path::new("a.bru"));
    assert_eq!(
        fs::read_dir(parent.path()).unwrap().count(),
        1,
        "only the taken directory"
    );
}

#[test]
fn a_name_that_sanitises_to_nothing_is_invalid() {
    let src = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    legacy(
        src.path(),
        r#"{"version": "1", "name": "...", "type": "collection"}"#,
    );
    let plan = plan_conversion(src.path()).unwrap();
    let Err(ConvertError::InvalidName { name }) = convert(
        &plan,
        ConvertTarget::NewFolderIn(parent.path().to_path_buf()),
    ) else {
        panic!("expected InvalidName");
    };
    assert_eq!(name, "...");
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn a_name_longer_than_a_file_name_allows_is_refused_and_leaves_nothing() {
    let src = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    // 128 two-byte characters: 256 bytes, one over the file-name limit.
    let name = "\u{e9}".repeat(128);
    legacy(
        src.path(),
        &format!(r#"{{"version": "1", "name": "{name}", "type": "collection"}}"#),
    );
    let plan = plan_conversion(src.path()).unwrap();
    let Err(ConvertError::InvalidName { name: got }) = convert(
        &plan,
        ConvertTarget::NewFolderIn(parent.path().to_path_buf()),
    ) else {
        panic!("expected InvalidName");
    };
    assert_eq!(got, name);
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn a_unique_suffix_that_overflows_the_file_name_limit_is_refused_and_leaves_nothing() {
    // Each name fits on its own, but it is taken, and its " - 1" variant does not fit.
    for name in ["a".repeat(255), "\u{e9}".repeat(127)] {
        let src = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        legacy(
            src.path(),
            &format!(r#"{{"version": "1", "name": "{name}", "type": "collection"}}"#),
        );
        fs::create_dir(parent.path().join(&name)).unwrap();
        let plan = plan_conversion(src.path()).unwrap();
        let result = convert(
            &plan,
            ConvertTarget::NewFolderIn(parent.path().to_path_buf()),
        );
        let Err(ConvertError::InvalidName { name: got }) = result else {
            panic!("expected InvalidName, got {result:?}");
        };
        assert_eq!(got, name);
        assert_eq!(
            fs::read_dir(parent.path()).unwrap().count(),
            1,
            "only the taken directory"
        );
    }
}
