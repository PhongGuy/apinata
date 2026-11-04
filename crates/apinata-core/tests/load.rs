//! Loading single request files: accepted encodings and every rejected shape.

use apinata_core::RequestFile;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Write `text` to a request file in a fresh directory and load it.
fn load_text(text: &str) -> Result<RequestFile, String> {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yml");
    fs::write(&path, text).expect("write");
    RequestFile::load(&path).map_err(|e| e.message().to_owned())
}

const REQUEST: &str = "info:\n  name: w\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://example.test/w\n";

#[test]
fn a_request_file_loads_on_its_own() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bruno/cli-docs/request_2.yml");
    let r = RequestFile::load(path).expect("loads");
    assert_eq!(r.name(), "request_2");
    assert_eq!(r.file_name(), "request_2.yml");
    assert_eq!(r.seq(), Some(2));
    assert_eq!(r.method(), "GET");
    assert_eq!(r.url(), "https://api.example.com/two");
}

#[test]
fn defaults_apply_when_keys_are_missing() {
    let r = load_text("http:\n  url: u\n").expect("loads");
    assert_eq!(r.name(), "r");
    assert_eq!(r.method(), "GET");
    assert_eq!(r.seq(), None);
}

#[test]
fn crlf_and_bom_files_load() {
    let r = load_text(&format!("\u{feff}{}", REQUEST.replace('\n', "\r\n"))).expect("loads");
    assert_eq!(r.url(), "https://example.test/w");
    assert_eq!(r.seq(), Some(1));
}

#[test]
fn duplicate_keys_are_rejected() {
    let message =
        load_text("info:\n  name: a\ninfo:\n  name: b\nhttp:\n  url: u\n").expect_err("rejected");
    assert!(message.contains("duplicate"), "{message}");
}

#[test]
fn an_integer_beyond_u64_is_rejected() {
    assert!(load_text("info:\n  seq: 99999999999999999999999\nhttp:\n  url: u\n").is_err());
}

#[test]
fn invalid_shapes_are_rejected_with_a_named_reason() {
    let cases = [
        ("- a\n", "mapping"),
        ("", "mapping"),
        (
            "info:\n  name: !custom x\nhttp:\n  method: GET\n  url: u\n",
            "tag",
        ),
        (
            "info:\n  name: g\n  type: graphql\nhttp:\n  method: POST\n  url: u\n",
            "graphql",
        ),
        ("info:\n  name: n\n  type: http\n", "http"),
    ];
    for (text, reason) in cases {
        let message = load_text(text).expect_err(text);
        assert!(message.contains(reason), "{text:?}: {message}");
    }
}
