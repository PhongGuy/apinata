//! Auth: mode switches, same-mode edits and unsupported modes.

use crate::parity;

use apinata_core::{ApiKeyPlacement, AuthSpec, RequestFile};
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
fn auth_edits_match_bruno() {
    check_requests("auth-", |id, r| {
        r.set_auth(match id {
            "auth-basic" => AuthSpec::Basic {
                username: s("u"),
                password: s("{{pw}}"),
            },
            "auth-bearer" => AuthSpec::Bearer { token: s("t0k") },
            "auth-apikey-query" => AuthSpec::ApiKey {
                key: s("k"),
                value: s("v"),
                placement: ApiKeyPlacement::Query,
            },
            "auth-inherit" => AuthSpec::Inherit,
            other => panic!("no Apinata edit for {other}"),
        });
    });
}

#[test]
fn auth_reads_every_supported_mode() {
    let cases = [
        ("", AuthSpec::None),
        ("  auth: none\n", AuthSpec::None),
        ("  auth: inherit\n", AuthSpec::Inherit),
        (
            "  auth:\n    type: bearer\n    token: t\n",
            AuthSpec::Bearer { token: s("t") },
        ),
        (
            "  auth:\n    type: apikey\n    key: k\n    value: 1\n",
            AuthSpec::ApiKey {
                key: s("k"),
                value: s("1"),
                placement: ApiKeyPlacement::Header,
            },
        ),
        (
            "  auth:\n    type: oauth2\n    flow: x\n",
            AuthSpec::Unsupported { mode: s("oauth2") },
        ),
        (
            "  auth: digestish\n",
            AuthSpec::Unsupported {
                mode: s("digestish"),
            },
        ),
    ];
    for (extra, expected) in cases {
        let (_dir, path) = with_http(extra);
        assert_eq!(
            RequestFile::load(&path).expect("loads").auth(),
            expected,
            "{extra}"
        );
    }
}

#[test]
fn auth_none_removes_the_key_and_unsupported_writes_nothing() {
    let (_dir, path) = with_http("  auth:\n    type: oauth2\n    flow: x\n");
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_auth(AuthSpec::Unsupported { mode: s("oauth2") });
    assert!(!r.is_dirty());
    r.set_auth(AuthSpec::None);
    r.save().expect("saves");
    assert_eq!(keys(&saved(&path)["http"]), ["method", "url"]);
}

#[test]
fn a_same_mode_auth_edit_keeps_unknown_keys() {
    let (_dir, path) =
        with_http("  auth:\n    type: basic\n    x-note: keep\n    username: a\n    password: b\n");
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_auth(AuthSpec::Basic {
        username: s("a"),
        password: s("c"),
    });
    r.save().expect("saves");
    let auth = &saved(&path)["http"]["auth"];
    assert_eq!(keys(auth), ["type", "x-note", "username", "password"]);
    assert_eq!(auth["password"], yaml_serde::Value::from("c"));
}

#[test]
fn debug_hides_auth_secrets() {
    let shown = format!(
        "{:?} {:?} {:?}",
        AuthSpec::Basic {
            username: s("admin"),
            password: s("hunter2")
        },
        AuthSpec::Bearer {
            token: s("hunter2")
        },
        AuthSpec::ApiKey {
            key: s("X-Key"),
            value: s("hunter2"),
            placement: ApiKeyPlacement::Header
        },
    );
    assert!(
        !shown.contains("hunter2") && !shown.contains("admin"),
        "{shown}"
    );
    assert!(
        shown.contains("X-Key") && shown.contains("7 bytes"),
        "{shown}"
    );
}
