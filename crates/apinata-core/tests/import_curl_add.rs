//! `add_request` writes an imported request as a new file, the same bytes as applying it to a fresh one.

use apinata_core::import::{ImportError, NoteKind, add_request, parse_curl};
use apinata_core::{
    AuthSpec, BodySpec, Collection, ItemPath, KeyValue, Param, ParamKind, RequestKind, TreeError,
};
use std::fs;
use tempfile::TempDir;

fn collection() -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    let c = Collection::create(&dir.path().join("c"), "c").expect("create");
    (dir, c)
}

const COMMAND: &str = r#"curl -X PUT 'https://{{host}}/users/:id?x=1' -H 'Authorization: Bearer {{token}}' -u 'u:p:q' --json '{"a":1}'"#;

#[test]
fn add_request_writes_every_field() {
    let (_dir, mut c) = collection();
    let parsed = parse_curl(COMMAND).expect("parses");
    let folder = c.create_folder(&ItemPath::root(), "Users").expect("folder");
    let path = add_request(&mut c, &folder, "Update user", &parsed.request).expect("adds");
    assert_eq!(path, ItemPath::new("Users/Update user.yml").expect("path"));
    let reopened = Collection::open(c.root()).expect("reopens");
    let r = reopened.request(&path).expect("loaded");
    assert_eq!(
        (r.name(), r.method(), r.url()),
        ("Update user", "PUT", "https://{{host}}/users/:id?x=1")
    );
    assert_eq!(
        r.params(),
        [
            Param::new("x", "1", ParamKind::Query),
            Param::new("id", "", ParamKind::Path)
        ]
    );
    assert_eq!(
        r.headers(),
        [
            KeyValue::new("Authorization", "Bearer {{token}}"),
            KeyValue::new("Content-Type", "application/json")
        ]
    );
    assert_eq!(r.body(), BodySpec::Json("{\n  \"a\": 1\n}".into()));
    assert_eq!(
        r.auth(),
        AuthSpec::Basic {
            username: "u".into(),
            password: "p:q".into()
        }
    );
    assert!(!r.settings().encode_url);
    assert!(!r.settings().forward_authorization_header);
}

#[test]
fn applying_to_a_fresh_request_writes_the_same_bytes() {
    let parsed = parse_curl(COMMAND).expect("parses");
    let (_a, mut first) = collection();
    let added = add_request(&mut first, &ItemPath::root(), "R", &parsed.request).expect("adds");
    let (_b, mut second) = collection();
    let created = second
        .create_request(&ItemPath::root(), "R")
        .expect("creates");
    let file = second.request_mut(&created).expect("loaded");
    parsed.request.apply_to(file).expect("applies");
    file.save().expect("saves");
    assert_eq!(
        fs::read_to_string(added.to_path(first.root())).expect("added"),
        fs::read_to_string(created.to_path(second.root())).expect("applied"),
    );
}

#[test]
fn a_request_core_cannot_write_leaves_no_file() {
    let (_dir, mut c) = collection();
    let parsed = parse_curl(r"curl https://x.test -H $'Bad\nName: v'").expect("parses");
    let err = add_request(&mut c, &ItemPath::root(), "Bad", &parsed.request).expect_err("refused");
    assert!(matches!(err, ImportError::Edit(_)), "{err:?}");
    assert!(c.tree().requests().is_empty());
    assert!(!c.root().join("Bad.yml").exists());
}

#[test]
fn an_unknown_parent_is_a_tree_error() {
    let (_dir, mut c) = collection();
    let parsed = parse_curl("curl https://x.test").expect("parses");
    let missing = ItemPath::new("Nope").expect("path");
    let err = add_request(&mut c, &missing, "R", &parsed.request).expect_err("refused");
    assert!(
        matches!(err, ImportError::Tree(TreeError::NotFound { .. })),
        "{err:?}"
    );
}

#[test]
fn a_failed_add_leaves_gapped_siblings_untouched() {
    let (_dir, mut c) = collection();
    let a = c.create_request(&ItemPath::root(), "A").expect("a");
    let b = c.create_request(&ItemPath::root(), "B").expect("b");
    let file = c.request_mut(&b).expect("b loaded");
    file.set_seq(9);
    file.save().expect("saves b");
    let before = [&a, &b].map(|p| fs::read_to_string(p.to_path(c.root())).expect("sibling"));
    let parsed = parse_curl(r"curl https://x.test -H $'Bad\nName: v'").expect("parses");
    add_request(&mut c, &ItemPath::root(), "C", &parsed.request).expect_err("refused");
    let after = [&a, &b].map(|p| fs::read_to_string(p.to_path(c.root())).expect("sibling"));
    assert_eq!(before, after);
    assert!(!c.root().join("C.yml").exists());
    assert_eq!(c.tree().requests().len(), 2);
}

#[test]
fn a_curl_import_keeps_a_graphql_requests_query_and_variables() {
    let (_dir, mut c) = collection();
    let path = c
        .create_request_of_kind(&ItemPath::root(), "G", RequestKind::Graphql)
        .expect("creates");
    let kept = BodySpec::Graphql {
        query: "{ me }".into(),
        variables: "{\"a\": 1}".into(),
    };
    for (command, dropped) in [
        ("curl -X PUT https://x.test/g -H 'X-A: 1'", false),
        (r#"curl https://x.test/g --json '{"b":2}'"#, true),
    ] {
        let file = c.request_mut(&path).expect("loaded");
        file.set_body(kept.clone());
        let parsed = parse_curl(command).expect("parses");
        let notes = parsed.request.apply_to(file).expect("applies");
        assert_eq!(file.body(), kept, "{command}");
        assert_eq!(file.url(), "https://x.test/g", "{command}");
        let note = notes.iter().find(|n| {
            n.kind == NoteKind::Dropped
                && n.detail
                    == "the pasted body was not applied; a GraphQL request keeps its query and \
                        variables"
        });
        assert_eq!(note.is_some(), dropped, "{command}: {notes:?}");
    }
    let file = c.request(&path).expect("loaded");
    assert_eq!(file.method(), "POST");
    assert!(
        file.headers()
            .contains(&KeyValue::new("Content-Type", "application/json"))
    );
}

#[test]
fn a_curl_import_into_an_http_request_has_no_body_note() {
    let (_dir, mut c) = collection();
    let path = c.create_request(&ItemPath::root(), "R").expect("creates");
    let file = c.request_mut(&path).expect("loaded");
    let parsed = parse_curl(COMMAND).expect("parses");
    let notes = parsed.request.apply_to(file).expect("applies");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(file.body(), BodySpec::Json("{\n  \"a\": 1\n}".into()));
}
