//! The GraphQL parity corpus `2026-10-13-graphql-parity`: each edit case's
//! bytes after the matching Apinata edit.

use apinata_core::{AuthSpec, BodySpec, KeyValue, RequestFile};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference/2026-10-13-graphql-parity")
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    fixture: Option<String>,
}

fn expected(id: &str) -> String {
    fs::read_to_string(corpus().join("expected").join(format!("{id}.yml"))).expect("expected")
}

fn graphql(query: &str, variables: &str) -> BodySpec {
    BodySpec::Graphql {
        query: query.to_owned(),
        variables: variables.to_owned(),
    }
}

fn edit(id: &str, r: &mut RequestFile) {
    match id {
        _ if id.starts_with("gql-name-") => r.set_name("renamed"),
        "gql-query-set" => r.set_body(graphql(
            "query Users {\n  users {\n    id\n    name\n  }\n}",
            "",
        )),
        "gql-variables-single" => r.set_body(graphql("{ users { id } }", "{\"limit\": 10}")),
        "gql-variables-multi" => r.set_body(graphql(
            "",
            "{\n  \"limit\": 10,\n  \"after\": \"{{cursor}}\"\n}",
        )),
        "gql-clear-both" => r.set_body(graphql("", "")),
        "gql-url-method" => {
            r.set_url("https://api.example.com/graphql?op=users");
            r.set_method("GET");
        }
        "gql-headers-set" => {
            let mut off = KeyValue::new("x-b", "{{b}}");
            off.enabled = false;
            r.set_headers(vec![KeyValue::new("x-a", "1"), off])
                .expect("headers");
        }
        "gql-headers-clear" => r.set_headers(Vec::new()).expect("headers"),
        "gql-auth-bearer" => r.set_auth(AuthSpec::Bearer {
            token: "{{token}}".into(),
        }),
        "gql-auth-none" => r.set_auth(AuthSpec::None),
        other => panic!("no Apinata edit for {other}"),
    }
}

#[test]
fn every_edit_case_writes_the_reference_bytes() {
    let cases: Vec<Case> =
        serde_json::from_str(&fs::read_to_string(corpus().join("cases.json")).expect("cases"))
            .expect("valid cases.json");
    let mut failures = Vec::new();
    let mut ran = 0;
    for case in cases {
        let Some(fixture) = case.fixture else {
            continue;
        };
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("r.yml");
        let text = fs::read_to_string(corpus().join("normalized").join(&fixture)).expect("fixture");
        fs::write(&path, text).expect("copy");
        let mut r = RequestFile::load(&path).expect("loads");
        edit(&case.id, &mut r);
        r.save().expect("saves");
        let actual = fs::read_to_string(&path).expect("read back");
        if actual != expected(&case.id) {
            failures.push(format!(
                "{}\n  expected {:?}\n  actual   {actual:?}",
                case.id,
                expected(&case.id)
            ));
        }
        ran += 1;
    }
    assert_eq!(ran, 12, "every case with a fixture ran");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn none_clears_the_body_like_two_empty_fields() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yml");
    let text = fs::read_to_string(corpus().join("normalized/synthetic/graphql-full.yml")).unwrap();
    fs::write(&path, text).unwrap();
    let mut r = RequestFile::load(&path).unwrap();
    r.set_body(BodySpec::None);
    r.save().unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        expected("gql-clear-both")
    );
}
