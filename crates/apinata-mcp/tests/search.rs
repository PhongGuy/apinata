//! `search`: the same hits as `apinata search`, read-only, with the cap note and no secrets.

use crate::common;

use common::{ROOT, TestServer, request};
use serde_json::json;

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

#[tokio::test]
async fn search_returns_hits_with_a_cap_flag_and_never_secret_values() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "http://x/needle")),
        ("environments/dev.yml", DEV),
        (".env", "token=s3cr3t-dotenv-value\n"),
    ])
    .await;
    let found = server.ok("search", json!({"query": "NEEDLE"})).await;
    assert_eq!(found["capped"], false);
    assert_eq!(found["hits"][0]["path"], "a.yml");
    assert_eq!(found["hits"][0]["field"], "url");
    assert_eq!(found["hits"][0]["snippet"], "http://x/needle");

    let none = server
        .ok("search", json!({"query": "s3cr3t-dotenv-value"}))
        .await;
    assert_eq!(none["hits"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn search_is_read_only_and_flags_the_cap() {
    let owned: Vec<(String, String)> = (0..501)
        .map(|i| {
            (
                format!("r{i:03}.yml"),
                request(&format!("r{i:03}"), 1, "GET", "http://x/needle"),
            )
        })
        .collect();
    let mut files: Vec<(String, String)> = vec![("opencollection.yml".into(), ROOT.into())];
    files.extend(owned);
    let server = TestServer::start_owned(&files).await;
    let tools = server.tools().await;
    let tool = tools.iter().find(|t| t.name == "search").expect("listed");
    let hints = tool.annotations.as_ref().expect("annotations");
    assert_eq!(hints.read_only_hint, Some(true));
    assert_eq!(hints.open_world_hint, Some(false));
    let found = server.ok("search", json!({"query": "needle"})).await;
    assert_eq!(found["capped"], true);
    assert_eq!(found["hits"].as_array().unwrap().len(), 500);
}

/// A collection with one lower-case and one upper-case match, plus a secret in `.env`.
async fn case_server() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "http://x/needle")),
        ("b.yml", &request("b", 2, "GET", "http://x/NEEDLE")),
        ("environments/dev.yml", DEV),
        (".env", "token=s3cr3t-dotenv-value\n"),
    ])
    .await
}

fn paths(found: &serde_json::Value) -> Vec<String> {
    found["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn search_ignores_case_unless_asked() {
    let server = case_server().await;
    let default = server.ok("search", json!({"query": "needle"})).await;
    assert_eq!(paths(&default), ["a.yml", "b.yml"]);
    let exact = server
        .ok("search", json!({"query": "NEEDLE", "case": true}))
        .await;
    assert_eq!(paths(&exact), ["b.yml"]);
    let off = server
        .ok("search", json!({"query": "NEEDLE", "case": false}))
        .await;
    assert_eq!(paths(&off), ["a.yml", "b.yml"]);
}

#[tokio::test]
async fn search_regex_mode_matches_patterns_and_groups() {
    let server = case_server().await;
    let pattern = server
        .ok("search", json!({"query": "ne+dle", "regex": true}))
        .await;
    assert_eq!(paths(&pattern), ["a.yml", "b.yml"]);
    let group = server
        .ok("search", json!({"query": "x/(need|pin)le$", "regex": true}))
        .await;
    assert_eq!(paths(&group), ["a.yml", "b.yml"]);
    // Without regex the same text is a literal and finds nothing.
    let literal = server.ok("search", json!({"query": "ne+dle"})).await;
    assert!(paths(&literal).is_empty());
    let both = server
        .ok(
            "search",
            json!({"query": "ne+dle", "regex": true, "case": true}),
        )
        .await;
    assert_eq!(paths(&both), ["a.yml"]);
}

#[tokio::test]
async fn search_rejects_an_invalid_regex_with_the_core_message() {
    let server = case_server().await;
    let message = server
        .err("search", json!({"query": "(", "regex": true}))
        .await;
    assert!(
        message.contains("invalid regular expression"),
        "got: {message}"
    );
}

#[tokio::test]
async fn search_regex_mode_never_reaches_secret_values() {
    let server = case_server().await;
    let found = server
        .ok("search", json!({"query": "s3cr3t.*", "regex": true}))
        .await;
    assert!(paths(&found).is_empty());
}

#[tokio::test]
async fn search_schema_lists_the_optional_case_and_regex_flags() {
    let server = case_server().await;
    let tools = server.tools().await;
    let tool = tools.iter().find(|t| t.name == "search").expect("listed");
    let schema = serde_json::Value::Object((*tool.input_schema).clone());
    assert_eq!(schema["properties"]["case"]["type"], "boolean");
    assert_eq!(schema["properties"]["regex"]["type"], "boolean");
    let required = schema["required"].as_array().unwrap();
    assert_eq!(required, &[json!("query")]);
}

#[tokio::test]
async fn without_paging_arguments_the_reply_is_the_legacy_shape() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "http://x/needle")),
    ])
    .await;
    let found = server.ok("search", json!({"query": "needle"})).await;
    assert_eq!(
        found,
        json!({"hits": [{"path": "a.yml", "request": "a", "field": "url",
                         "snippet": "http://x/needle", "match_start": 9, "match_end": 15}],
               "capped": false})
    );
}

#[tokio::test]
async fn search_pages_past_the_cap_with_total_and_has_more() {
    let mut files: Vec<(String, String)> = vec![("opencollection.yml".into(), ROOT.into())];
    files.extend((0..1200).map(|i| {
        (
            format!("r{i:04}.yml"),
            request(&format!("r{i:04}"), 1, "GET", "http://x/needle"),
        )
    }));
    let server = TestServer::start_owned(&files).await;
    let mut all = Vec::new();
    for (offset, len, more) in [(0, 500, true), (500, 500, true), (1000, 200, false)] {
        let page = server
            .ok(
                "search",
                json!({"query": "needle", "limit": 500, "offset": offset}),
            )
            .await;
        let hits = page["hits"].as_array().unwrap();
        assert_eq!(hits.len(), len, "{offset}");
        assert_eq!(
            (&page["total"], &page["has_more"]),
            (&json!(1200), &json!(more))
        );
        assert_eq!(page["limit"], 500);
        assert!(page.get("capped").is_none());
        all.extend(hits.iter().map(|h| h["path"].as_str().unwrap().to_owned()));
    }
    let expected: Vec<String> = (0..1200).map(|i| format!("r{i:04}.yml")).collect();
    assert_eq!(all, expected);
    let past = server
        .ok("search", json!({"query": "needle", "offset": 9000}))
        .await;
    assert_eq!(past["hits"], json!([]));
    assert_eq!(
        (&past["total"], &past["has_more"]),
        (&json!(1200), &json!(false))
    );
    let zero = server
        .err("search", json!({"query": "needle", "limit": 0}))
        .await;
    assert!(zero.contains("limit"), "{zero}");
    let big = server
        .ok("search", json!({"query": "needle", "limit": 100000}))
        .await;
    assert_eq!(big["limit"], 500);
    let negative = server
        .err("search", json!({"query": "needle", "offset": -1}))
        .await;
    assert!(negative.contains("`offset`: invalid value"), "{negative}");
    assert!(!negative.contains("-1"), "{negative}");
}

#[tokio::test]
async fn the_search_description_tells_an_agent_to_page() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools.iter().find(|t| t.name == "search").unwrap();
    let text = tool.description.as_deref().unwrap();
    for word in ["offset", "limit", "has_more", "total"] {
        assert!(text.contains(word), "{word}: {text}");
    }
}
