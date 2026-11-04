//! SPARQL bodies: sent as raw text with `application/sparql-query`, and edited against the
//! SPARQL parity corpus.

use crate::common;

use apinata_core::{BodySpec, RequestFile, ResolveError};
use common::{at, executor, setup};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: limit\n      value: \"5\"\n    - name: q\n      value: a\"b\n";

/// A POST of `url` with a SPARQL body; `headers` sits under `http:` before the body.
fn sparql(url: &str, headers: &str, data: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{url}\"\n{headers}  body:\n    type: sparql\n    data: {data}\n"
    )
}

/// The content type and body of the one request `server` received.
async fn send_one(text: &str) -> (Option<String>, String) {
    let server = MockServer::start().await;
    Mock::given(path("/q"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let text = text.replace("<URL>", &format!("{}/q", server.uri()));
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    executor()
        .send(&at(&c, &ctx, "r.yml").expect("resolves"))
        .await
        .expect("sent");
    let all = server.received_requests().await.expect("recorded");
    assert_eq!(all.len(), 1);
    let content_type = all[0]
        .headers
        .get("content-type")
        .map(|v| v.to_str().expect("ascii").to_owned());
    (
        content_type,
        String::from_utf8(all[0].body.clone()).expect("utf-8"),
    )
}

#[tokio::test]
async fn a_sparql_body_is_sent_as_its_text_with_variables_resolved_unescaped() {
    let (content_type, body) = send_one(&sparql(
        "<URL>",
        "",
        "\"SELECT ?s WHERE { ?s ?p \\\"{{q}}\\\" } LIMIT {{limit}}\"",
    ))
    .await;
    assert_eq!(content_type.as_deref(), Some("application/sparql-query"));
    assert_eq!(body, "SELECT ?s WHERE { ?s ?p \"a\"b\" } LIMIT 5");
}

#[tokio::test]
async fn a_user_content_type_wins_over_the_sparql_default() {
    let headers = "  headers:\n    - name: Content-Type\n      value: text/plain\n";
    let (content_type, body) = send_one(&sparql("<URL>", headers, "ASK {}")).await;
    assert_eq!(content_type.as_deref(), Some("text/plain"));
    assert_eq!(body, "ASK {}");
}

#[test]
fn an_unresolved_variable_in_a_sparql_body_is_an_error() {
    let text = sparql("https://x.test/", "", "\"LIMIT {{nope}}\"");
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    assert!(matches!(
        at(&c, &ctx, "r.yml"),
        Err(ResolveError::UndefinedVariable { name, .. }) if name == "nope"
    ));
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reference/2026-10-27-sparql-parity")
}

fn expected(id: &str) -> String {
    fs::read_to_string(corpus().join("expected").join(format!("{id}.yml"))).expect("expected")
}

/// A temporary copy of a normalised corpus source, loaded.
fn load(fixture: &str) -> (TempDir, PathBuf, RequestFile) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("r.yml");
    let text = fs::read_to_string(corpus().join("normalized").join(fixture)).expect("fixture");
    fs::write(&path, text).expect("copy");
    let r = RequestFile::load(&path).expect("loads");
    (dir, path, r)
}

const SOURCE_QUERY: &str = "PREFIX foaf: <http://xmlns.com/foaf/0.1/>\nSELECT ?name WHERE {\n  ?p foaf:name ?name .\n}\nLIMIT {{limit}}";

#[test]
fn a_sparql_file_loads_as_a_sparql_body() {
    let (_dir, _path, r) = load("synthetic/sparql.yml");
    assert_eq!(r.body(), BodySpec::Sparql(SOURCE_QUERY.to_owned()));
}

#[test]
fn setting_the_same_sparql_text_leaves_the_file_clean() {
    let (_dir, _path, mut r) = load("synthetic/sparql.yml");
    r.set_body(BodySpec::Sparql(SOURCE_QUERY.to_owned()));
    assert!(!r.is_dirty());
}

#[test]
fn saving_an_unedited_sparql_file_keeps_its_bytes() {
    let (_dir, path, mut r) = load("synthetic/sparql.yml");
    r.save().expect("saves");
    let source = fs::read_to_string(corpus().join("normalized/synthetic/sparql.yml")).expect("src");
    assert_eq!(fs::read_to_string(&path).expect("saved"), source);
}

#[derive(serde::Deserialize)]
struct Case {
    id: String,
    fixture: String,
}

fn edit(id: &str, r: &mut RequestFile) {
    let text = |t: &str| t.to_owned();
    match id {
        "sparql-name" => r.set_name("renamed"),
        "sparql-data-set" => r.set_body(BodySpec::Sparql(text(
            "SELECT ?s WHERE {\n  ?s a <{{type}}> .\n}\nLIMIT 10",
        ))),
        "sparql-data-single" => r.set_body(BodySpec::Sparql(text("ASK { ?s ?p ?o }"))),
        "sparql-data-clear" => r.set_body(BodySpec::Sparql(String::new())),
        "sparql-from-text" => r.set_body(BodySpec::Sparql(text(
            "SELECT * WHERE { ?s ?p ?o } LIMIT 1",
        ))),
        "sparql-to-text" => r.set_body(BodySpec::Text(text(SOURCE_QUERY))),
        other => panic!("no Apinata edit for {other}"),
    }
}

#[test]
fn every_sparql_edit_case_writes_the_reference_bytes() {
    let cases: Vec<Case> =
        serde_json::from_str(&fs::read_to_string(corpus().join("cases.json")).expect("cases"))
            .expect("valid cases.json");
    let mut failures = Vec::new();
    for case in &cases {
        let (_dir, path, mut r) = load(&case.fixture);
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
    }
    assert_eq!(cases.len(), 6, "every case ran");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
