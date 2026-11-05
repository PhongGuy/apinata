//! `ExampleResponse::scrub` and `request_secrets`: what a saved example keeps of a response.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, ExampleBodyKind, ExampleResponse, MemoryStore, Response,
    SecretStore, VarContext, derived_secrets, request_secrets,
};
use common::{request as file, write_tree};
use std::sync::Arc;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
const REQUEST: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test/\"\n  headers:\n    - name: X-Token\n      value: \"{{token}}\"\n";
const TOKEN: &str = "s3cr3t-Token";

/// The secrets `r.yml` knows with `dev` selected and `token` stored, plus `derived`.
fn secrets(derived: &[String]) -> Vec<apinata_core::NamedSecret> {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", REQUEST),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    store.set(&c.secret_key("dev", "token"), TOKEN).unwrap();
    let ctx = VarContext::new(
        &c,
        ContextOptions::new().env("dev").secrets(Arc::new(store)),
    )
    .expect("context");
    request_secrets(file(&c, "r.yml"), &ctx, derived).expect("resolves")
}

fn echoed() -> ExampleResponse {
    ExampleResponse::from_response(
        &Response::new(200)
            .with_header("set-cookie", "sid=abc123; Path=/")
            .with_header("x-token", TOKEN)
            .with_text_body(format!("{{\"token\":\"{TOKEN}\"}}")),
    )
}

#[test]
fn without_secrets_only_the_headers_are_masked() {
    let mut example = echoed();
    let scrubbed = example.scrub(None);
    assert!(!scrubbed.withheld);
    assert_eq!(
        example.headers,
        vec![
            ("set-cookie".to_owned(), "sid=[redacted]".to_owned()),
            ("x-token".to_owned(), TOKEN.to_owned()),
        ]
    );
    assert!(example.body.unwrap().data.contains(TOKEN));
}

#[test]
fn known_secrets_become_placeholders_in_headers_and_text_body() {
    let mut example = echoed();
    let scrubbed = example.scrub(Some(&secrets(&[])));
    assert!(!scrubbed.withheld);
    assert_eq!(
        example.headers,
        vec![
            ("set-cookie".to_owned(), "sid=[redacted]".to_owned()),
            ("x-token".to_owned(), "{{token}}".to_owned()),
        ]
    );
    assert_eq!(example.body.unwrap().data, "{\"token\":\"{{token}}\"}");
}

#[test]
fn a_binary_body_holding_a_secret_is_withheld_and_the_description_says_so() {
    let bytes = [b"\xff\x00".as_slice(), TOKEN.as_bytes(), b"\xfe"].concat();
    let mut example = ExampleResponse::from_response(
        &Response::new(200)
            .with_header("content-type", "application/octet-stream")
            .with_body(bytes, String::new()),
    );
    assert_eq!(example.body.as_ref().unwrap().kind, ExampleBodyKind::Binary);
    let scrubbed = example.scrub(Some(&secrets(&[])));
    assert!(scrubbed.withheld);
    assert!(example.body.is_none());
    assert_eq!(example.status, Some(200));
    assert_eq!(
        scrubbed.description(""),
        ExampleResponse::WITHHELD_NOTE.to_owned()
    );
    assert_eq!(
        scrubbed.description("mine"),
        format!("mine\n{}", ExampleResponse::WITHHELD_NOTE)
    );
}

#[test]
fn a_binary_body_holding_a_secret_as_utf16_is_withheld() {
    for little in [true, false] {
        let wide: Vec<u8> = TOKEN
            .encode_utf16()
            .flat_map(|u| {
                if little {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                }
            })
            .collect();
        let mut example = ExampleResponse::from_response(
            &Response::new(200)
                .with_header("content-type", "application/octet-stream")
                .with_body([b"\xff\xfe".as_slice(), &wide].concat(), String::new()),
        );
        assert!(
            example.scrub(Some(&secrets(&[]))).withheld,
            "little {little}"
        );
        assert!(example.body.is_none());
    }
}

#[test]
fn a_clean_binary_body_is_kept_and_the_description_is_untouched() {
    let mut example = ExampleResponse::from_response(
        &Response::new(200)
            .with_header("content-type", "application/octet-stream")
            .with_body(vec![0xff, 0x00, 1, 2], String::new()),
    );
    let before = example.body.clone();
    let scrubbed = example.scrub(Some(&derived_secrets(&[TOKEN.to_owned()])));
    assert!(!scrubbed.withheld);
    assert_eq!(example.body, before);
    assert_eq!(scrubbed.description("mine"), "mine");
}

#[test]
fn request_secrets_put_derived_values_after_the_named_ones() {
    let found = secrets(&["derived-Value-1".to_owned()]);
    let labels: Vec<&str> = found.iter().map(|s| s.label()).collect();
    assert_eq!(labels, ["token", "derived credential"]);
    assert_eq!(found[1].placeholder(), "[redacted]");
}

#[test]
fn only_a_whole_response_under_the_limit_is_savable() {
    assert!(ExampleResponse::savable(
        &Response::new(200).with_text_body("ok")
    ));
    assert!(!ExampleResponse::savable(
        &Response::new(200).with_text_body("ok").with_truncated(10)
    ));
    let limit = ExampleResponse::BODY_LIMIT as usize;
    assert!(!ExampleResponse::savable(
        &Response::new(200).with_body(vec![b'a'; limit], "")
    ));
    assert!(ExampleResponse::savable(
        &Response::new(200).with_body(vec![b'a'; limit - 1], "")
    ));
}

#[test]
fn has_example_matches_a_name_trimmed_as_core_trims_it() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", REQUEST)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let item = apinata_core::ItemPath::new("r.yml").expect("path");
    c.add_example(&item, "one", "", &ExampleResponse::new(200))
        .expect("added");
    let request = file(&c, "r.yml");
    // U+FEFF is JavaScript whitespace but not Rust's.
    assert!(request.has_example("\u{feff}one "));
    assert!(!request.has_example("two"));
}
