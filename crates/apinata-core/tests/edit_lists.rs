//! Headers, tags and docs: Bruno's bytes on normalised files, minimal edits elsewhere.

use crate::parity;

use apinata_core::{EditError, KeyValue, RequestFile};
use parity::{check_requests, file_with, keys, saved};

const HAND_WRITTEN: &str = "info:\n  name: r\n  type: http\n  tags: [a]\nhttp:\n  method: GET\n  url: u\n  headers:\n    - name: Accept\n      x-note: keep\n      value: 123\n      description: why\n    - name: X-Other\n      value: two\ndocs: text\n";

fn described(name: &str, value: &str, description: &str) -> KeyValue {
    let mut kv = KeyValue::new(name, value);
    kv.description = Some(description.to_owned());
    kv
}

fn disabled(name: &str, value: &str) -> KeyValue {
    let mut kv = KeyValue::new(name, value);
    kv.enabled = false;
    kv
}

#[test]
fn header_edits_match_bruno() {
    check_requests("headers-", |id, r| {
        let mut headers = r.headers();
        match id {
            "headers-add" => headers = vec![KeyValue::new("x-a", "1"), disabled("x-b", "{{b}}")],
            "headers-edit-one" => headers[1].value = "changed".to_owned(),
            "headers-disable-and-remove" => {
                headers[0].enabled = false;
                headers.remove(2);
            }
            "headers-clear" => headers.clear(),
            other => panic!("no Apinata edit for {other}"),
        }
        r.set_headers(headers).expect("valid headers");
    });
}

#[test]
fn tag_and_docs_edits_match_bruno() {
    check_requests("tags-", |id, r| match id {
        "tags-set" => r.set_tags(vec!["smoke".to_owned(), "v2".to_owned()]),
        other => panic!("no Apinata edit for {other}"),
    });
    check_requests("docs-", |id, r| match id {
        "docs-set" => r.set_docs("# Title\nline two"),
        other => panic!("no Apinata edit for {other}"),
    });
}

#[test]
fn headers_read_names_values_and_state_with_brunos_coercions() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let r = RequestFile::load(&path).expect("loads");
    assert_eq!(
        r.headers(),
        vec![
            described("Accept", "123", "why"),
            KeyValue::new("X-Other", "two")
        ]
    );
    assert_eq!(r.tags(), ["a"]);
    assert_eq!(r.docs(), "text");
    let (_dir, path) = file_with("info:\n  name: r\nhttp:\n  url: u\n");
    let r = RequestFile::load(&path).expect("loads");
    assert!(r.headers().is_empty());
    assert!(r.tags().is_empty());
    assert_eq!(r.docs(), "");
}

#[test]
fn writing_back_what_was_read_leaves_the_file_clean() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    let headers = r.headers();
    r.set_headers(headers).expect("valid");
    r.set_tags(r.tags());
    r.set_docs(r.docs());
    assert!(!r.is_dirty());
}

#[test]
fn an_edited_header_keeps_its_unknown_keys_in_place() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    let mut headers = r.headers();
    headers[0].enabled = false;
    r.set_headers(headers).expect("valid");
    r.save().expect("saves");
    let after = saved(&path);
    let first = &after["http"]["headers"][0];
    assert_eq!(
        keys(first),
        ["name", "x-note", "value", "description", "disabled"]
    );
    assert_eq!(first["value"], yaml_serde::Value::from("123"));
    assert_eq!(first["description"], yaml_serde::Value::from("why"));
    // The untouched header is written back exactly as read.
    assert_eq!(keys(&after["http"]["headers"][1]), ["name", "value"]);
}

#[test]
fn an_untouched_header_keeps_its_original_scalar() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    let mut headers = r.headers();
    headers[1].value = "three".to_owned();
    r.set_headers(headers).expect("valid");
    r.save().expect("saves");
    let after = saved(&path);
    assert_eq!(
        after["http"]["headers"][0]["value"],
        yaml_serde::Value::from(123)
    );
    assert_eq!(
        after["http"]["headers"][1]["value"],
        yaml_serde::Value::from("three")
    );
}

#[test]
fn a_header_name_with_a_line_break_is_refused_without_a_change() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    let mut headers = r.headers();
    headers.push(KeyValue::new("X-Bad\r\nInjected", "v"));
    assert_eq!(
        r.set_headers(headers),
        Err(EditError::InvalidHeaderName { row: 2 })
    );
    assert!(!r.is_dirty());
    let err = r
        .set_headers(vec![KeyValue::new("a\nb", "")])
        .expect_err("refused");
    assert_eq!(err.to_string(), "`headers[0].name` cannot be written");
}

#[test]
fn empty_lists_and_blank_docs_remove_their_keys() {
    let (_dir, path) = file_with(HAND_WRITTEN);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_headers(Vec::new()).expect("valid");
    r.set_tags(Vec::new());
    r.set_docs(" \n\t");
    r.save().expect("saves");
    let after = saved(&path);
    assert_eq!(keys(&after), ["info", "http"]);
    assert_eq!(keys(&after["http"]), ["method", "url"]);
    assert_eq!(keys(&after["info"]), ["name", "type"]);
}

#[test]
fn new_headers_go_after_url_on_a_file_without_headers() {
    let (_dir, path) = file_with(
        "info:\n  name: r\nhttp:\n  method: GET\n  body:\n    type: text\n    data: x\n  url: u\n",
    );
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_headers(vec![KeyValue::new("a", "b")]).expect("valid");
    r.save().expect("saves");
    assert_eq!(
        keys(&saved(&path)["http"]),
        ["method", "body", "url", "headers"]
    );
}

#[test]
fn debug_shows_header_names_and_sizes_only() {
    let shown = format!(
        "{:?}",
        KeyValue::new("Authorization", "Bearer s3cret-token")
    );
    assert!(shown.contains("Authorization"), "{shown}");
    assert!(shown.contains("19 bytes"), "{shown}");
    assert!(!shown.contains("s3cret"), "{shown}");
}

#[test]
fn writing_back_unchanged_numeric_docs_leaves_the_file_clean() {
    let (_dir, path) = file_with("info:\n  name: r\nhttp:\n  url: u\ndocs: 123\n");
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_docs(r.docs());
    assert!(!r.is_dirty());
}
