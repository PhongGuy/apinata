//! `update_environment`: Bruno's bytes for each variable edit an agent may make, and the secret
//! rules: no secret value is set, returned, written back to the file or deleted through MCP.

use crate::common;

use common::{ROOT, TestServer, files_under, research};
use serde_json::{Value, json};
use std::fs;

const SECRET: &str = "sentinel-s3cr3t value";

async fn normalised(collection: &str) -> TestServer {
    let input = research("2026-10-09-edit-parity/normalized").join(collection);
    TestServer::start_owned(&files_under(&input)).await
}

fn read(server: &TestServer, rel: &str) -> String {
    fs::read_to_string(server.root().join(rel)).expect("read")
}

#[tokio::test]
async fn variable_edits_match_brunos_bytes() {
    let cases: [(&str, &str, Value, &str); 6] = [
        (
            "env-vars-edit",
            "bruno-tests",
            json!({"name": "Local", "set": [{"name": "host", "value": "http://x.test"}]}),
            "Local",
        ),
        (
            "env-vars-add-secret",
            "bruno-tests",
            json!({"name": "Local", "set": [{"name": "token", "secret": true}]}),
            "Local",
        ),
        (
            "env-vars-disable-secret",
            "env-inheritance",
            json!({"name": "staging", "set": [{"name": "api_key", "enabled": false}]}),
            "staging",
        ),
        (
            "env-vars-clear",
            "env-inheritance",
            json!({"name": "qa", "remove": ["api_url"]}),
            "qa",
        ),
        (
            "env-secret-on",
            "env-inheritance",
            json!({"name": "dev", "set": [{"name": "api_url", "secret": true}]}),
            "dev",
        ),
        (
            "env-secret-on-typed",
            "env-inheritance",
            json!({"name": "base", "set": [{"name": "base_number", "secret": true}]}),
            "base",
        ),
    ];
    let mut failures = Vec::new();
    for (id, collection, args, env) in cases {
        let server = normalised(collection).await;
        let result = server.call("update_environment", args).await;
        let file = format!("environments/{env}.yml");
        let expected = fs::read_to_string(
            research("2026-10-09-config-edit-parity/expected")
                .join(id)
                .join(&file),
        )
        .expect("expected");
        let actual = read(&server, &file);
        if result.is_error == Some(true) || actual != expected {
            failures.push(format!(
                "{id}: {}\n  actual {actual:?}",
                common::text(&result)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn making_a_variable_secret_moves_its_value_out_of_the_file_and_the_result() {
    let server = normalised("env-inheritance").await;
    let doc = server
        .ok(
            "update_environment",
            json!({"name": "dev", "set": [{"name": "api_url", "secret": true}]}),
        )
        .await;
    assert_eq!(
        server.secret("dev", "api_url").as_deref(),
        Some("https://dev.example.com")
    );
    assert!(!doc.to_string().contains("dev.example.com"), "{doc}");
    assert!(!read(&server, "environments/dev.yml").contains("dev.example.com"));
}

#[tokio::test]
async fn no_call_sets_unsets_or_removes_a_secret() {
    let env = "name: prod\nvariables:\n  - name: host\n    value: https://x.test\n  - secret: true\n    name: token\n";
    let server =
        TestServer::start(&[("opencollection.yml", ROOT), ("environments/prod.yml", env)]).await;
    server.set_secret("prod", "token", SECRET);
    let refused = [
        json!({"name": "prod", "set": [{"name": "token", "value": "new"}]}),
        json!({"name": "prod", "set": [{"name": "fresh", "value": "v", "secret": true}]}),
        json!({"name": "prod", "set": [{"name": "token", "secret": false}]}),
        json!({"name": "prod", "set": [{"name": "token", "type": "number"}]}),
        json!({"name": "prod", "set": [{"name": "fresh", "type": "number", "secret": true}]}),
        json!({"name": "prod", "remove": ["token"]}),
        // One refused change stops the whole call, the allowed host edit included.
        json!({"name": "prod", "set": [{"name": "host", "value": "https://y.test"}], "remove": ["token"]}),
    ];
    for args in refused {
        let err = server.err("update_environment", args.clone()).await;
        assert!(
            err.contains("token") || err.contains("fresh"),
            "{args}: {err}"
        );
        assert!(!err.contains(SECRET), "{err}");
    }
    assert_eq!(
        read(&server, "environments/prod.yml"),
        env,
        "nothing was written"
    );
    assert_eq!(server.secret("prod", "token").as_deref(), Some(SECRET));
    let shown = server.ok("read_environment", json!({"name": "prod"})).await;
    assert!(!shown.to_string().contains(SECRET), "{shown}");
}

#[tokio::test]
async fn brunos_secret_off_case_is_refused_and_leaves_the_file() {
    let server = normalised("env-inheritance").await;
    let before = read(&server, "environments/staging.yml");
    let err = server
        .err(
            "update_environment",
            json!({"name": "staging", "set": [{"name": "api_key", "secret": false}]}),
        )
        .await;
    assert!(err.contains("api_key"), "{err}");
    assert_eq!(read(&server, "environments/staging.yml"), before);
}

#[tokio::test]
async fn is_annotated_as_a_closed_world_non_destructive_write() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "update_environment")
        .expect("listed");
    let hints = tool.annotations.clone().expect("annotated");
    assert_eq!(hints.destructive_hint, Some(false));
    assert_eq!(hints.open_world_hint, Some(false));
}

#[tokio::test]
async fn a_plain_duplicate_cannot_overwrite_the_stored_secret_of_its_name() {
    let env = "name: prod\nvariables:\n  - name: token\n    value: attacker\n  - secret: true\n    name: token\n";
    let server =
        TestServer::start(&[("opencollection.yml", ROOT), ("environments/prod.yml", env)]).await;
    server.set_secret("prod", "token", SECRET);
    for args in [
        json!({"name": "prod", "set": [{"name": "token", "value": "new"}]}),
        json!({"name": "prod", "set": [{"name": "token", "type": "number"}]}),
        json!({"name": "prod", "set": [{"name": "token", "secret": false}]}),
    ] {
        let err = server.err("update_environment", args.clone()).await;
        assert!(err.contains("token"), "{args}: {err}");
    }
    server
        .call(
            "update_environment",
            json!({"name": "prod", "set": [{"name": "token", "secret": true}]}),
        )
        .await;
    assert_eq!(server.secret("prod", "token").as_deref(), Some(SECRET));
    assert_eq!(read(&server, "environments/prod.yml"), env);
}

const PLAIN_TOKEN: &str = "name: prod\nvariables:\n  - name: host\n    value: a\n  - name: token\n    value: ''\n  - name: typed\n    value: y\n  - name: fresh\n    value: z\n";

async fn plain_token() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/prod.yml", PLAIN_TOKEN),
    ])
    .await
}

#[tokio::test]
async fn a_stored_value_refuses_making_its_name_secret_before_any_write() {
    for name in ["token", "typed"] {
        let server = plain_token().await;
        server.set_secret("prod", name, SECRET);
        let args = json!({"name": "prod", "set": [
            {"name": "host", "value": "b"},
            {"name": "fresh", "secret": true},
            {"name": name, "secret": true},
        ]});
        let err = server.err("update_environment", args).await;
        assert!(err.contains(name), "{err}");
        assert!(err.contains("apinata env forget"), "{err}");
        assert!(!err.contains(SECRET), "{err}");
        assert_eq!(server.secret("prod", name).as_deref(), Some(SECRET));
        assert_eq!(server.secret("prod", "fresh"), None);
        assert_eq!(read(&server, "environments/prod.yml"), PLAIN_TOKEN);
    }
}

#[tokio::test]
async fn a_name_with_nothing_stored_still_becomes_secret() {
    let server = plain_token().await;
    server
        .ok(
            "update_environment",
            json!({"name": "prod", "set": [{"name": "typed", "secret": true}]}),
        )
        .await;
    assert_eq!(server.secret("prod", "typed").as_deref(), Some("y"));
    assert!(read(&server, "environments/prod.yml").contains("secret: true"));
}
