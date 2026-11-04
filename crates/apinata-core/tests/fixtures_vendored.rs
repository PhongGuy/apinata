//! The vendored Bruno fixtures are present, attributed, and parse as YAML.

use std::fs;
use std::path::{Path, PathBuf};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bruno")
}

fn yml_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("readable fixture dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(yml_files(&path));
        } else if path.extension().is_some_and(|e| e == "yml") {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn all_27_fixture_files_are_vendored_and_parse() {
    let files = yml_files(&fixtures_root());
    assert_eq!(files.len(), 27, "{files:#?}");
    for file in files {
        let text = fs::read_to_string(&file).expect("utf-8 fixture");
        let parsed: Result<yaml_serde::Value, _> = yaml_serde::from_str(&text);
        assert!(parsed.is_ok(), "{}: {:?}", file.display(), parsed.err());
    }
}

#[test]
fn fixtures_carry_bruno_attribution() {
    let notice = fs::read_to_string(fixtures_root().join("NOTICE.md")).expect("NOTICE.md");
    assert!(notice.contains("1301ea17"));
    assert!(notice.contains("MIT License"));
}

#[test]
fn the_global_environment_fixtures_are_vendored_attributed_and_parse() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/global-environments");
    let files = yml_files(&dir);
    assert_eq!(files.len(), 5, "{files:#?}");
    for file in files {
        let text = fs::read_to_string(&file).expect("utf-8 fixture");
        let parsed: Result<yaml_serde::Value, _> = yaml_serde::from_str(&text);
        assert!(parsed.is_ok(), "{}: {:?}", file.display(), parsed.err());
    }
    let notice = fs::read_to_string(dir.join("NOTICE.md")).expect("NOTICE.md");
    assert!(notice.contains("1301ea17"));
    assert!(notice.contains("MIT License"));
}
