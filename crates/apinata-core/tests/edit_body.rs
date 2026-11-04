//! Bodies: mode switches, same-mode edits and unsupported modes.

use crate::parity;

use apinata_core::{BodySpec, FileEntry, KeyValue, MultipartField, MultipartValue, RequestFile};
use parity::{check_requests, file_with, keys, saved};
use std::path::PathBuf;
use tempfile::TempDir;

fn with_http(extra: &str) -> (TempDir, PathBuf) {
    file_with(&format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: u\n{extra}"
    ))
}

fn s(text: &str) -> String {
    text.to_owned()
}
#[test]
fn body_edits_match_bruno() {
    check_requests("body-", |id, r| {
        let body = match id {
            "body-json-edit" => BodySpec::Json(s("{\"a\": 1}")),
            "body-json-to-form" => {
                let mut b = KeyValue::new("b", "2");
                b.enabled = false;
                BodySpec::FormUrlEncoded(vec![KeyValue::new("a", "1"), b])
            }
            "body-json-to-none" => BodySpec::None,
            "body-text" => BodySpec::Text(s("hello\nworld")),
            "body-xml" => BodySpec::Xml(s("<a/>")),
            "body-form-empty" => BodySpec::FormUrlEncoded(Vec::new()),
            "body-form-edit-one" => {
                let BodySpec::FormUrlEncoded(mut items) = r.body() else {
                    panic!("form body")
                };
                items[0].value = s("plain");
                BodySpec::FormUrlEncoded(items)
            }
            "body-multipart-new" => {
                let mut file = MultipartField::files("f", vec![s("a.png"), s("b.txt")]);
                file.content_type = s("image/png");
                let mut off = MultipartField::text("off", "x");
                off.enabled = false;
                BodySpec::Multipart(vec![MultipartField::text("t", "v"), file, off])
            }
            "body-multipart-disable" => {
                let BodySpec::Multipart(mut items) = r.body() else {
                    panic!("multipart body")
                };
                items[2].enabled = false;
                BodySpec::Multipart(items)
            }
            "body-file" => {
                let mut a = FileEntry::new("data/a.bin");
                a.content_type = s("application/octet-stream");
                a.selected = true;
                BodySpec::File(vec![a, FileEntry::new("b.json")])
            }
            other => panic!("no Apinata edit for {other}"),
        };
        r.set_body(body);
    });
}

#[test]
fn bodies_read_every_supported_mode() {
    let (_dir, path) = with_http("");
    assert_eq!(
        RequestFile::load(&path).expect("loads").body(),
        BodySpec::None
    );
    let (_dir, path) = with_http("  body:\n    type: json\n    data: '{}'\n");
    assert_eq!(
        RequestFile::load(&path).expect("loads").body(),
        BodySpec::Json(s("{}"))
    );
    let multipart = "  body:\n    type: multipart-form\n    data:\n      - name: f\n        type: file\n        value: [a.png]\n        contentType: image/png\n      - name: t\n        type: text\n        value: 5\n        disabled: true\n";
    let (_dir, path) = with_http(multipart);
    let BodySpec::Multipart(fields) = RequestFile::load(&path).expect("loads").body() else {
        panic!("multipart")
    };
    assert_eq!(fields[0].value, MultipartValue::Files(vec![s("a.png")]));
    assert_eq!(fields[0].content_type, "image/png");
    assert_eq!(fields[1].value, MultipartValue::Text(s("5")));
    assert!(!fields[1].enabled);
    let file = "  body:\n    type: file\n    data:\n      - filePath: a.bin\n        contentType: ''\n        selected: true\n";
    let (_dir, path) = with_http(file);
    let BodySpec::File(entries) = RequestFile::load(&path).expect("loads").body() else {
        panic!("file")
    };
    assert_eq!(
        (entries[0].path.as_str(), entries[0].selected),
        ("a.bin", true)
    );
}

#[test]
fn an_unsupported_body_is_shown_and_never_written() {
    let (_dir, path) = with_http("  body:\n    type: msgpack\n    data: gqFhAQ==\n");
    let mut r = RequestFile::load(&path).expect("loads");
    assert_eq!(r.body(), BodySpec::Unsupported { kind: s("msgpack") });
    r.set_body(BodySpec::Unsupported { kind: s("graphql") });
    assert!(!r.is_dirty());
    r.set_body(BodySpec::Text(s("x")));
    r.save().expect("saves");
    assert_eq!(
        saved(&path)["http"]["body"]["type"],
        yaml_serde::Value::from("text")
    );
}

#[test]
fn a_same_mode_edit_keeps_other_body_keys() {
    let (_dir, path) = with_http("  body:\n    type: json\n    x-note: keep\n    data: '{}'\n");
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_body(BodySpec::Json(s("[]")));
    r.save().expect("saves");
    assert_eq!(
        keys(&saved(&path)["http"]["body"]),
        ["type", "x-note", "data"]
    );
}

#[test]
fn a_mode_switch_replaces_the_whole_body() {
    let (_dir, path) = with_http("  body:\n    type: json\n    x-note: gone\n    data: '{}'\n");
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_body(BodySpec::FormUrlEncoded(Vec::new()));
    r.save().expect("saves");
    assert_eq!(keys(&saved(&path)["http"]["body"]), ["type"]);
}

#[test]
fn debug_hides_body_text() {
    let shown = format!(
        "{:?} {:?}",
        BodySpec::Json(s("{\"pw\":\"hunter2\"}")),
        BodySpec::Multipart(vec![MultipartField::text("t", "hunter2")]),
    );
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(
        shown.contains("16 bytes") && shown.contains("7 bytes"),
        "{shown}"
    );
}

#[test]
fn an_empty_body_key_reads_as_no_body() {
    let (_dir, path) = with_http("  body:\n");
    assert_eq!(
        RequestFile::load(&path).expect("loads").body(),
        BodySpec::None
    );
}
