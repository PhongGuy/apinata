//! `export::postman`: request settings in `protocolProfileBehavior`, and notes for what has no slot.

use crate::export_support;

use apinata_core::import::{ImportedItem, parse_postman};
use apinata_core::{RequestSettings, Timeout};
use export_support::*;
use serde_json::json;

/// A GET request `a` with `extra` appended after the `http:` block.
fn with(extra: &str) -> (tempfile::TempDir, apinata_core::Collection) {
    open(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &(http("a", "GET", "http://x.test/a", INHERIT) + extra),
        ),
    ])
}

fn reimported(c: &apinata_core::Collection) -> RequestSettings {
    let json = apinata_core::export::postman(c).json;
    let parsed = parse_postman(json.as_bytes()).expect("parses");
    match parsed.items.as_slice() {
        [ImportedItem::Request(r)] => r.request.settings.expect("settings"),
        other => panic!("one request expected: {other:?}"),
    }
}

#[test]
fn changed_settings_survive_a_round_trip() {
    let (_d, c) = with(
        "settings:\n  encodeUrl: false\n  followRedirects: false\n  maxRedirects: 0\n  forwardAuthorizationHeader: true\n",
    );
    let s = reimported(&c);
    assert_eq!(
        (
            s.encode_url,
            s.follow_redirects,
            s.max_redirects,
            s.forward_authorization_header
        ),
        (false, false, 0, true)
    );
    assert_eq!(
        exported(&c)["item"][0]["protocolProfileBehavior"],
        json!({"disableUrlEncoding": true, "followRedirects": false, "maxRedirects": 0, "followAuthorizationHeader": true})
    );
}

#[test]
fn a_changed_redirect_limit_alone_is_written() {
    let (_d, c) = with("settings:\n  maxRedirects: 9\n  forwardAuthorizationHeader: false\n");
    assert_eq!(
        exported(&c)["item"][0]["protocolProfileBehavior"],
        json!({"maxRedirects": 9})
    );
    assert_eq!(reimported(&c).max_redirects, 9);
}

#[test]
fn importer_defaults_write_no_behavior() {
    let (_d, c) = with(
        "settings:\n  encodeUrl: true\n  followRedirects: true\n  maxRedirects: 5\n  forwardAuthorizationHeader: false\n",
    );
    assert!(
        exported(&c)["item"][0]
            .get("protocolProfileBehavior")
            .is_none()
    );
}

#[test]
fn a_file_without_settings_forwards_authorization_so_the_export_says_so() {
    let (_d, c) = with("");
    assert_eq!(
        exported(&c)["item"][0]["protocolProfileBehavior"],
        json!({"followAuthorizationHeader": true})
    );
}

#[test]
fn body_pruning_and_settings_share_one_behavior() {
    let extra = "settings:\n  followRedirects: false\n  forwardAuthorizationHeader: false\n";
    let under = format!("  body:\n    type: json\n    data: '{{}}'\n{INHERIT}");
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &(http("a", "GET", "http://x.test/a", &under) + extra),
        ),
    ]);
    assert_eq!(
        exported(&c)["item"][0]["protocolProfileBehavior"],
        json!({"disableBodyPruning": true, "followRedirects": false})
    );
}

#[test]
fn a_timeout_is_noted_and_unlimited_reads_differently() {
    let (_d, c) = with("settings:\n  timeout: 5000\n");
    let n = notes(&c);
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0].0, "a / timeout");
    assert!(n[0].1.contains("5000"), "{}", n[0].1);
    assert!(n[0].1.contains("inherit the default timeout"), "{}", n[0].1);

    let (_d, c) = with("settings:\n  timeout: 0\n");
    let z = notes(&c);
    assert_eq!(z.len(), 1, "{z:?}");
    assert_eq!(z[0].0, "a / timeout");
    assert!(z[0].1.contains("no timeout"), "{}", z[0].1);
    assert!(z[0].1.contains("inherit the default timeout"), "{}", z[0].1);
    assert_ne!(z[0].1, n[0].1);
}

#[test]
fn an_inherited_timeout_has_no_note() {
    let (_d, c) = with("settings:\n  timeout: inherit\n");
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
    assert_eq!(reimported(&c).timeout, Timeout::Inherit);
}

#[test]
fn tags_are_noted_by_request() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &(http("a", "GET", "http://x.test/a", INHERIT)
                .replace("  seq: 1\n", "  seq: 1\n  tags:\n    - smoke\n    - slow\n")),
        ),
        (
            "b.yml",
            &http("b", "GET", "http://x.test/b", INHERIT).replace("seq: 1", "seq: 2"),
        ),
    ]);
    let n = notes(&c);
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0].0, "a / tags");
    assert!(
        n[0].1.contains("smoke") && n[0].1.contains("slow"),
        "{}",
        n[0].1
    );
}

#[test]
fn a_typed_collection_variable_is_noted_and_a_text_one_is_not() {
    let root = format!(
        "{ROOT}request:\n  variables:\n    - name: n\n      value:\n        type: number\n        data: \"42\"\n    - name: host\n      value: http://x.test\n"
    );
    let (_d, c) = open(&[("opencollection.yml", &root)]);
    let n = notes(&c);
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0].0, "collection / variables / n");
    assert!(
        n[0].1.contains("number") && n[0].1.contains("text"),
        "{}",
        n[0].1
    );
}

fn reimported_omitted(c: &apinata_core::Collection) -> Vec<String> {
    let json = apinata_core::export::postman(c).json;
    let parsed = parse_postman(json.as_bytes()).expect("parses");
    match parsed.items.as_slice() {
        [ImportedItem::Request(r)] => r.request.omitted_headers.clone(),
        other => panic!("one request expected: {other:?}"),
    }
}

#[test]
fn omitted_headers_are_written_lowercased_and_survive_a_round_trip() {
    let (_d, c) = with(
        "settings:\n  forwardAuthorizationHeader: false\n  omitHeaders:\n    - \" User-Agent \"\n    - Accept\n",
    );
    assert_eq!(
        exported(&c)["item"][0]["protocolProfileBehavior"],
        json!({"disabledSystemHeaders": {"user-agent": true, "accept": true}})
    );
    assert_eq!(reimported_omitted(&c), ["user-agent", "accept"]);
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
}

#[test]
fn no_omitted_headers_write_no_field() {
    let (_d, c) = with("settings:\n  forwardAuthorizationHeader: false\n  omitHeaders: []\n");
    assert!(
        exported(&c)["item"][0]
            .get("protocolProfileBehavior")
            .is_none()
    );
    assert!(reimported_omitted(&c).is_empty());
}

#[test]
fn only_true_disabled_system_headers_are_imported() {
    let parsed = parse_postman(
        br#"{"info": {"name": "c", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
             "item": [{"name": "r",
               "protocolProfileBehavior": {"disabledSystemHeaders": {"accept": true, "host": false, " ": true, "user-agent": true}},
               "request": {"method": "GET", "url": "u"}}]}"#,
    )
    .expect("parses");
    let [ImportedItem::Request(r)] = parsed.items.as_slice() else {
        panic!("one request")
    };
    assert_eq!(r.request.omitted_headers, ["accept", "user-agent"]);
    assert!(
        !r.notes
            .iter()
            .any(|n| n.source.contains("disabledSystemHeaders")),
        "{:?}",
        r.notes
    );
}
