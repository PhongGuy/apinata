//! Converting a request file between HTTP and GraphQL, and undoing the conversion.

use crate::tree_support;

use apinata_core::{BodySpec, ConvertError, RequestKind};
use std::fs;
use tree_support::{ROOT, ip, open};
use yaml_serde::Value;

// Every section a conversion keeps, around a GraphQL body whose variables are already
// indented the way a conversion writes them.
const GRAPHQL: &str = r#"info:
  name: Users
  type: graphql
  seq: 3
  tags:
    - smoke

graphql:
  method: POST
  url: https://api.test/graphql?debug=1
  headers:
    - name: X-Trace
      value: "1"
  params:
    - name: debug
      value: "1"
      type: query
  body:
    query: |-
      query Me($id: ID!) {
        user(id: $id) { name }
      }
    variables: |-
      {
        "id": "7"
      }
  auth:
    type: bearer
    token: "{{token}}"

runtime:
  variables:
    - name: a
      value: b
  scripts:
    - type: before-request
      code: bru.setVar("x", 1)
  assertions:
    - expression: res.status
      operator: eq
      value: "200"

settings:
  encodeUrl: true
  timeout: 0
  followRedirects: true
  maxRedirects: 5

docs: Some docs
"#;

fn http_with_body(body: &str) -> String {
    GRAPHQL
        .replace("type: graphql", "type: http")
        .replace("graphql:\n  method", "http:\n  method")
        .replace(
            "  body:\n    query: |-\n      query Me($id: ID!) {\n        user(id: $id) { name }\n      }\n    variables: |-\n      {\n        \"id\": \"7\"\n      }\n",
            body,
        )
}

fn yaml(text: &str) -> Value {
    yaml_serde::from_str(text).expect("valid yaml")
}

// The sections a conversion must leave alone, the protocol root without its body and
// `info` without its type.
fn kept(text: &str) -> Vec<Value> {
    let mut doc = yaml(text);
    let map = doc.as_mapping_mut().expect("mapping");
    let mut root = map
        .remove("http")
        .or_else(|| map.remove("graphql"))
        .expect("protocol root");
    root.as_mapping_mut().expect("root mapping").remove("body");
    let mut info = map.remove("info").expect("info");
    info.as_mapping_mut().expect("info mapping").remove("type");
    let rest = ["runtime", "settings", "docs"].map(|k| map.get(k).cloned().unwrap_or(Value::Null));
    [info, root].into_iter().chain(rest).collect()
}

#[test]
fn graphql_becomes_a_json_body_of_query_and_variables() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", GRAPHQL)]);
    let done = c.convert_request(&ip("u.yml"), RequestKind::Http).unwrap();
    assert!(done.dropped().is_empty());
    let text = fs::read_to_string(dir.path().join("u.yml")).unwrap();
    assert_eq!(kept(&text), kept(GRAPHQL));
    let r = c.request(&ip("u.yml")).unwrap();
    assert_eq!(r.kind(), RequestKind::Http);
    assert_eq!(
        r.body(),
        BodySpec::Json(
            "{\n  \"query\": \"query Me($id: ID!) {\\n  user(id: $id) { name }\\n}\",\n  \"variables\": {\n    \"id\": \"7\"\n  }\n}"
                .into()
        )
    );
    // The JSON body sends as application/json by default, as GraphQL does, so no header is added.
    assert!(!text.to_lowercase().contains("content-type"));
}

#[test]
fn a_round_trip_through_http_restores_the_graphql_file_byte_for_byte() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", GRAPHQL)]);
    c.convert_request(&ip("u.yml"), RequestKind::Http).unwrap();
    c.convert_request(&ip("u.yml"), RequestKind::Graphql)
        .unwrap();
    let text = fs::read_to_string(dir.path().join("u.yml")).unwrap();
    assert_eq!(text, GRAPHQL);
}

#[test]
fn a_graphql_shaped_json_body_becomes_the_query_and_indented_variables() {
    let body = "  body:\n    type: json\n    data: '{\"query\": \"{ me { id } }\", \"variables\": {\"id\": 1}}'\n";
    let http = http_with_body(body);
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", &http)]);
    let done = c
        .convert_request(&ip("u.yml"), RequestKind::Graphql)
        .unwrap();
    assert!(done.dropped().is_empty());
    let text = fs::read_to_string(dir.path().join("u.yml")).unwrap();
    assert_eq!(kept(&text), kept(&http));
    let r = c.request(&ip("u.yml")).unwrap();
    assert_eq!(r.kind(), RequestKind::Graphql);
    assert_eq!(
        r.body(),
        BodySpec::Graphql {
            query: "{ me { id } }".into(),
            variables: "{\n  \"id\": 1\n}".into(),
        }
    );
}

#[test]
fn an_http_round_trip_keeps_every_compatible_section_byte_for_byte() {
    let body = "  body:\n    type: json\n    data: '{\"query\": \"{ me }\"}'\n";
    let http = http_with_body(body);
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", &http)]);
    c.convert_request(&ip("u.yml"), RequestKind::Graphql)
        .unwrap();
    c.convert_request(&ip("u.yml"), RequestKind::Http).unwrap();
    let text = fs::read_to_string(dir.path().join("u.yml")).unwrap();
    let body_at = |t: &str| t.find("  body:").expect("a body");
    let auth_at = |t: &str| t.find("  auth:").expect("auth");
    assert_eq!(text[..body_at(&text)], http[..body_at(&http)]);
    assert_eq!(text[auth_at(&text)..], http[auth_at(&http)..]);
}

#[test]
fn a_body_that_is_not_a_graphql_payload_is_dropped_and_noted() {
    for body in [
        "  body:\n    type: json\n    data: '{\"query\": \"shoes\", \"page\": 2}'\n",
        "  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: \"1\"\n",
    ] {
        let http = http_with_body(body);
        let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", &http)]);
        let done = c
            .convert_request(&ip("u.yml"), RequestKind::Graphql)
            .unwrap();
        assert_eq!(done.dropped().len(), 1, "{body}");
        assert!(done.dropped()[0].contains("body"), "{:?}", done.dropped());
        let r = c.request(&ip("u.yml")).unwrap();
        let empty = BodySpec::Graphql {
            query: String::new(),
            variables: String::new(),
        };
        assert_eq!(r.body(), empty);
        let text = fs::read_to_string(dir.path().join("u.yml")).unwrap();
        assert_eq!(kept(&text), kept(&http));
    }
}

#[test]
fn saved_examples_are_dropped_and_noted_on_the_way_to_graphql() {
    let http = format!(
        "{}\nexamples:\n  - name: ok\n    response:\n      status: 200\n",
        http_with_body("")
    );
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", &http)]);
    let done = c
        .convert_request(&ip("u.yml"), RequestKind::Graphql)
        .unwrap();
    assert_eq!(done.dropped().len(), 1);
    assert!(
        done.dropped()[0].contains("example"),
        "{:?}",
        done.dropped()
    );
    let text = fs::read_to_string(dir.path().join("u.yml")).unwrap();
    assert!(!text.contains("examples:"));
}

#[test]
fn unsaved_edits_refuse_the_conversion() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", GRAPHQL)]);
    c.request_mut(&ip("u.yml")).unwrap().set_name("Renamed");
    let err = c
        .convert_request(&ip("u.yml"), RequestKind::Http)
        .unwrap_err();
    assert!(matches!(err, ConvertError::Unsaved { .. }), "{err:?}");
    assert!(err.to_string().contains("Save or discard"), "{err}");
    let r = c.request(&ip("u.yml")).unwrap();
    assert_eq!((r.kind(), r.name()), (RequestKind::Graphql, "Renamed"));
    assert_eq!(
        fs::read_to_string(dir.path().join("u.yml")).unwrap(),
        GRAPHQL
    );
}

#[test]
fn converting_to_the_same_kind_is_refused() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", GRAPHQL)]);
    let err = c
        .convert_request(&ip("u.yml"), RequestKind::Graphql)
        .unwrap_err();
    assert!(matches!(err, ConvertError::SameKind { .. }), "{err:?}");
}

#[test]
fn undo_puts_back_the_file_as_it_was() {
    let http = format!(
        "{}\nexamples:\n  - name: ok\n    response:\n      status: 200\n",
        http_with_body("  body:\n    type: text\n    data: hello\n")
    );
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", &http)]);
    let before = fs::read_to_string(dir.path().join("u.yml")).unwrap();
    let done = c
        .convert_request(&ip("u.yml"), RequestKind::Graphql)
        .unwrap();
    c.undo_conversion(&ip("u.yml"), &done).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("u.yml")).unwrap(),
        before
    );
    let r = c.request(&ip("u.yml")).unwrap();
    assert_eq!(r.kind(), RequestKind::Http);
    assert!(!r.is_dirty());
}

#[test]
fn undo_refuses_once_the_request_changed_again() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", GRAPHQL)]);
    let done = c.convert_request(&ip("u.yml"), RequestKind::Http).unwrap();
    let r = c.request_mut(&ip("u.yml")).unwrap();
    r.set_name("Later");
    r.save().unwrap();
    let saved = fs::read_to_string(dir.path().join("u.yml")).unwrap();
    let err = c.undo_conversion(&ip("u.yml"), &done).unwrap_err();
    assert!(matches!(err, ConvertError::Changed { .. }), "{err:?}");
    assert_eq!(fs::read_to_string(dir.path().join("u.yml")).unwrap(), saved);
}

#[test]
fn a_missing_request_is_not_found() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let err = c
        .convert_request(&ip("nope.yml"), RequestKind::Http)
        .unwrap_err();
    assert!(matches!(err, ConvertError::NotFound { .. }), "{err:?}");
}

#[test]
fn a_raw_graphql_content_type_goes_when_the_body_turns_into_json() {
    let body = "  body:\n    type: text\n    data: \"{ me }\"\n";
    let http = http_with_body(body).replace(
        "    - name: X-Trace\n      value: \"1\"\n",
        "    - name: X-Trace\n      value: \"1\"\n    - name: Content-Type\n      value: application/graphql\n",
    );
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", &http)]);
    let done = c
        .convert_request(&ip("u.yml"), RequestKind::Graphql)
        .unwrap();
    assert!(done.dropped().is_empty());
    let r = c.request(&ip("u.yml")).unwrap();
    let names: Vec<String> = r.headers().into_iter().map(|h| h.name).collect();
    assert_eq!(names, ["X-Trace"]);
    let query = match r.body() {
        BodySpec::Graphql { query, .. } => query,
        other => panic!("{other:?}"),
    };
    assert_eq!(query, "{ me }");
    let text = fs::read_to_string(dir.path().join("u.yml")).unwrap();
    assert!(!text.contains("application/graphql"), "{text}");
}

#[test]
fn a_preview_names_the_losses_and_changes_nothing() {
    let http = http_with_body("  body:\n    type: text\n    data: hello\n");
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", &http)]);
    let preview = c
        .conversion_preview(&ip("u.yml"), RequestKind::Graphql)
        .unwrap();
    assert_eq!(preview.dropped().len(), 1);
    assert!(preview.dropped()[0].contains("body"));
    assert_eq!(fs::read_to_string(dir.path().join("u.yml")).unwrap(), http);
    let r = c.request(&ip("u.yml")).unwrap();
    assert_eq!(r.kind(), RequestKind::Http);
    assert!(!r.is_dirty());
    let done = c
        .convert_request(&ip("u.yml"), RequestKind::Graphql)
        .unwrap();
    assert_eq!(done.dropped(), preview.dropped());
}

#[test]
fn a_preview_refuses_as_the_conversion_would() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", GRAPHQL)]);
    c.request_mut(&ip("u.yml")).unwrap().set_name("Renamed");
    let err = c
        .conversion_preview(&ip("u.yml"), RequestKind::Http)
        .unwrap_err();
    assert!(matches!(err, ConvertError::Unsaved { .. }), "{err:?}");
}

#[test]
fn a_graphql_get_keeps_its_method_and_says_so() {
    let get = GRAPHQL.replace("  method: POST\n", "  method: GET\n");
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", &get)]);
    let preview = c
        .conversion_preview(&ip("u.yml"), RequestKind::Http)
        .unwrap();
    assert!(preview.dropped().is_empty());
    assert_eq!(preview.notes().len(), 1);
    assert!(preview.notes()[0].contains("GET"), "{:?}", preview.notes());
    let done = c.convert_request(&ip("u.yml"), RequestKind::Http).unwrap();
    assert_eq!(done.notes(), preview.notes());
    assert_eq!(c.request(&ip("u.yml")).unwrap().method(), "GET");
    // A POST says nothing.
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT), ("u.yml", GRAPHQL)]);
    let done = c.convert_request(&ip("u.yml"), RequestKind::Http).unwrap();
    assert!(done.notes().is_empty());
}
