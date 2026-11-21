//! A row an agent writes without `enabled` is enabled, and the tool schema says so.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::{Value, json};

const REQUEST: &str =
    "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:1/\"\n";

#[tokio::test]
async fn rows_without_enabled_are_written_enabled() {
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("r.yml", REQUEST)]).await;
    server
        .ok(
            "update_request",
            json!({
                "path": "r.yml",
                "headers": [{"name": "X-A", "value": "1"}],
                "params": [{"name": "q", "value": "2", "kind": "query"}],
                "vars": [{"name": "v", "value": "3"}],
                "assertions": [{"expression": "res.status", "operator": "eq", "value": "200"}],
                "body": {"type": "form_urlencoded", "fields": [{"name": "f", "value": "4"}]},
            }),
        )
        .await;
    let read = server.ok("read_request", json!({"path": "r.yml"})).await;
    assert_eq!(read["headers"][0]["enabled"], true, "{read}");
    assert_eq!(read["params"][0]["enabled"], true, "{read}");
    assert_eq!(read["vars"][0]["enabled"], true, "{read}");
    assert_eq!(read["assertions"][0]["enabled"], true, "{read}");
    assert_eq!(read["body"]["fields"][0]["enabled"], true, "{read}");
}

#[tokio::test]
async fn the_schema_advertises_the_default() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "update_request")
        .expect("listed");
    let defs = &tool.input_schema["$defs"];
    for name in ["KeyValueDoc", "ParamDoc", "VarDoc", "AssertionDoc"] {
        let doc = &defs[name];
        assert_eq!(
            doc["properties"]["enabled"]["default"],
            Value::Bool(true),
            "{name}: {doc}"
        );
        let required = doc["required"].as_array().expect("required");
        assert!(!required.iter().any(|n| n == "enabled"), "{name}: {doc}");
    }
}
