//! A tool argument that fails to deserialize is refused with its field path, the kind of
//! problem and what was expected, but never with the value the client sent.

use crate::common;

use common::{ROOT, TestServer, text};
use rmcp::model::{CallToolResult, Tool};
use serde_json::{Map, Value, json};

const PREFIX: &str = "failed to deserialize parameters:";
const NUMBER: i64 = 987654321;

fn marker(n: usize) -> String {
    format!("SECRET-{n}, expected ok\"")
}

async fn start() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/\n",
        ),
    ])
    .await
}

/// `schema` with a `$ref` into the tool's `$defs` followed.
fn resolve<'a>(root: &'a Map<String, Value>, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => {
            let name = r.rsplit('/').next().expect("ref name");
            root.get("$defs")
                .and_then(|d| d.get(name))
                .unwrap_or_else(|| panic!("unresolved {r}"))
        }
        None => schema,
    }
}

/// Whether the property takes free text, so a string marker would be legitimate data.
fn takes_free_text(root: &Map<String, Value>, schema: &Value) -> bool {
    let schema = resolve(root, schema);
    if schema.get("enum").is_some() || schema.get("const").is_some() {
        return false;
    }
    let typed = |t: &Value| match t {
        Value::String(s) => s == "string",
        Value::Array(a) => a.iter().any(|t| t == "string"),
        _ => false,
    };
    if let Some(t) = schema.get("type") {
        return typed(t);
    }
    ["anyOf", "oneOf"].iter().any(|k| {
        schema
            .get(*k)
            .and_then(Value::as_array)
            .is_some_and(|alts| alts.iter().any(|a| takes_free_text(root, a)))
    })
}

/// Every text and structured part of a reply, joined.
fn everything(result: &CallToolResult) -> String {
    format!(
        "{}\n{}",
        text(result),
        result
            .structured_content
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_default()
    )
}

#[tokio::test]
async fn no_tool_echoes_a_mistyped_argument() {
    let server = start().await;
    let tools: Vec<Tool> = server.tools().await;
    assert!(tools.len() > 20, "{} tools", tools.len());
    let mut n = 0;
    let mut tried = 0;
    for tool in &tools {
        let schema = tool.input_schema.as_ref();
        let Some(props) = schema.get("properties").and_then(Value::as_object) else {
            continue;
        };
        for (name, prop) in props {
            n += 1;
            let mark = marker(n);
            let value = if takes_free_text(schema, prop) {
                json!(NUMBER)
            } else {
                json!(mark)
            };
            let mut args = Map::new();
            args.insert(name.clone(), value);
            let reply = match server.try_call(&tool.name, Value::Object(args)).await {
                Ok(r) => everything(&r),
                Err(e) => e,
            };
            tried += 1;
            assert!(!reply.contains("SECRET-"), "{} {name}: {reply}", tool.name);
            assert!(
                !reply.contains(&NUMBER.to_string()),
                "{} {name}: {reply}",
                tool.name
            );
            if reply.starts_with(PREFIX) && !reply.contains("missing field") {
                assert!(
                    reply.contains(&format!("`{name}`")),
                    "{} {name}: {reply}",
                    tool.name
                );
            }
        }
    }
    assert!(tried > 50, "only {tried} properties tried");
}

#[tokio::test]
async fn a_nested_bad_value_names_its_path_and_what_was_expected() {
    let server = start().await;
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "headers": [
                {"name": "A", "value": "1", "enabled": true},
                {"name": "B", "value": "2", "enabled": marker(1)},
            ]}),
        )
        .await;
    assert!(message.starts_with(PREFIX), "{message}");
    assert!(message.contains("`headers[1].enabled`"), "{message}");
    assert!(message.contains("invalid type"), "{message}");
    assert!(message.contains("a boolean"), "{message}");
    assert!(!message.contains("SECRET-"), "{message}");
}

#[tokio::test]
async fn an_unknown_variant_inside_the_auth_document_is_not_echoed() {
    let server = start().await;
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "auth": {"type": "apikey", "key": "k", "value": "v",
                "placement": marker(2)}}),
        )
        .await;
    assert!(message.starts_with(PREFIX), "{message}");
    assert!(message.contains("`auth.placement`"), "{message}");
    assert!(message.contains("unknown variant"), "{message}");
    assert!(message.contains("`header`"), "{message}");
    assert!(!message.contains("SECRET-"), "{message}");
}

#[tokio::test]
async fn a_wrong_oauth2_stage_lists_the_files_spellings() {
    let server = start().await;
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "auth": {"type": "oauth2", "flow": "implicit",
                "authorization_url": "", "access_token_url": "", "refresh_token_url": "",
                "callback_url": "", "client_id": "", "client_secret": "", "username": "",
                "password": "", "scope": "", "state": "", "pkce": false, "token_id": "",
                "token_placement": {"type": "unset"}, "token_source": "access_token",
                "auto_fetch": true, "auto_refresh": false,
                "params": [{"stage": "access_token_request", "name": "a", "value": "1",
                    "placement": "query"}]}}),
        )
        .await;
    assert!(message.starts_with(PREFIX), "{message}");
    assert!(message.contains("unknown variant"), "{message}");
    for spelling in [
        "`authorizationRequest`",
        "`accessTokenRequest`",
        "`refreshTokenRequest`",
    ] {
        assert!(message.contains(spelling), "{spelling}: {message}");
    }
    assert!(!message.contains("access_token_request"), "{message}");
}

#[tokio::test]
async fn an_unknown_field_is_named_but_a_long_one_is_cut() {
    let server = start().await;
    let message = server
        .err("update_request", json!({"path": "r.yml", "header": []}))
        .await;
    assert!(message.contains("unknown field `header`"), "{message}");
    let long = "k".repeat(200);
    let message = server
        .err("update_request", json!({"path": "r.yml", long.clone(): 1}))
        .await;
    assert!(message.contains("unknown field"), "{message}");
    assert!(!message.contains(&long), "{message}");
    assert!(message.contains(&"k".repeat(64)), "{message}");
}

/// An implicit-flow oauth2 auth document with `field` replaced by `value`.
fn oauth2_auth(field: &str, value: Value) -> Value {
    let mut auth = json!({"type": "oauth2", "flow": "implicit",
        "authorization_url": "", "access_token_url": "", "refresh_token_url": "",
        "callback_url": "", "client_id": "", "client_secret": "", "username": "",
        "password": "", "scope": "", "state": "", "pkce": false, "token_id": "",
        "token_placement": {"type": "unset"}, "token_source": "access_token",
        "auto_fetch": true, "auto_refresh": false, "params": []});
    auth[field] = value;
    auth
}

#[tokio::test]
async fn a_bad_value_deep_in_a_collection_or_folder_auth_names_its_full_path() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/\n",
        ),
        (
            "f/a.yml",
            "info:\n  name: a\n  type: http\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/\n",
        ),
    ])
    .await;
    let params = json!([
        {"stage": "authorizationRequest", "name": "a", "value": "1", "placement": "query"},
        {"stage": "authorizationRequest", "name": "b", "value": "2", "placement": NUMBER},
    ]);
    // `""` is the collection's own auth, `f` a folder's.
    for target in ["", "f"] {
        let message = server
            .err(
                "update_folder",
                json!({"path": target, "auth": oauth2_auth("params", params.clone())}),
            )
            .await;
        assert!(message.starts_with(PREFIX), "{target:?}: {message}");
        assert!(
            message.contains("`auth.params[1].placement`: invalid type"),
            "{target:?}: {message}"
        );
        assert!(
            !message.contains(&NUMBER.to_string()),
            "{target:?}: {message}"
        );
    }
    let placement = json!({"type": "header", "prefix": NUMBER});
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "auth": oauth2_auth("token_placement", placement)}),
        )
        .await;
    assert!(
        message.contains("`auth.token_placement.prefix`: invalid type"),
        "{message}"
    );
    assert!(!message.contains(&NUMBER.to_string()), "{message}");
}

#[tokio::test]
async fn an_unknown_key_inside_the_auth_document_names_it_and_its_object() {
    let server = start().await;
    let params = json!([{"stage": "authorizationRequest", "name": "a", "value": "1",
        "placement": "query", "colour": NUMBER}]);
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "auth": oauth2_auth("params", params)}),
        )
        .await;
    assert!(message.starts_with(PREFIX), "{message}");
    assert!(
        message.contains("`auth.params[0].colour`: unknown field `colour`"),
        "{message}"
    );
    assert!(!message.contains(&NUMBER.to_string()), "{message}");
    let message = server
        .err(
            "update_request",
            json!({"path": "r.yml", "auth": {"type": "bearer", "token": "t",
                "tokn": marker(3)}}),
        )
        .await;
    assert!(
        message.contains("`auth.tokn`: unknown field `tokn`"),
        "{message}"
    );
    assert!(!message.contains("SECRET-"), "{message}");
}
