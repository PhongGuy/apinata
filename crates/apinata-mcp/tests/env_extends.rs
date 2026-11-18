//! `extends` on `update_environment` and its global twin: a name sets the parent, null clears it,
//! an absent field leaves it, and core's refusals come back as tool errors.

use crate::common;

use apinata_core::Collection;
use common::{ROOT, TestServer};
use serde_json::json;
use std::fs;

const BASE: &str = "name: base\nvariables:\n  - name: host\n    value: base.test\n";
const DEV: &str = "name: dev\nvariables:\n  - name: port\n    value: \"8080\"\n";

async fn server() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", BASE),
        ("environments/dev.yml", DEV),
    ])
    .await
}

fn read(server: &TestServer, env: &str) -> String {
    fs::read_to_string(server.root().join(format!("environments/{env}.yml"))).expect("read")
}

#[tokio::test]
async fn extends_sets_the_parent_with_the_apps_bytes() {
    let server = server().await;
    let doc = server
        .ok(
            "update_environment",
            json!({"name": "dev", "extends": "base"}),
        )
        .await;
    assert_eq!(doc["extends"], "base", "{doc}");

    let twin = tempfile::TempDir::new().expect("tempdir");
    fs::write(twin.path().join("opencollection.yml"), ROOT).expect("root");
    fs::create_dir(twin.path().join("environments")).expect("mkdir");
    fs::write(twin.path().join("environments/base.yml"), BASE).expect("base");
    fs::write(twin.path().join("environments/dev.yml"), DEV).expect("dev");
    let mut c = Collection::open(twin.path()).expect("opens");
    let env = c.environment_mut("dev").expect("dev");
    env.set_extends(Some("base".into()));
    env.save().expect("saves");
    let expected = fs::read_to_string(twin.path().join("environments/dev.yml")).expect("read");
    assert_eq!(read(&server, "dev"), expected);

    let doc = server.ok("read_environment", json!({"name": "dev"})).await;
    assert_eq!(doc["extends"], "base", "{doc}");
}

#[tokio::test]
async fn null_clears_the_parent_and_absence_keeps_it() {
    let server = server().await;
    server
        .ok(
            "update_environment",
            json!({"name": "dev", "extends": "base"}),
        )
        .await;
    let args = json!({"name": "dev", "set": [{"name": "port", "value": "9090"}]});
    let doc = server.ok("update_environment", args).await;
    assert_eq!(doc["extends"], "base", "{doc}");
    let doc = server
        .ok(
            "update_environment",
            json!({"name": "dev", "extends": null}),
        )
        .await;
    assert!(doc.get("extends").is_none_or(|v| v.is_null()), "{doc}");
    assert!(!read(&server, "dev").contains("extends"));
}

#[tokio::test]
async fn a_cycle_is_refused_with_cores_error_and_nothing_changes() {
    let server = server().await;
    server
        .ok(
            "update_environment",
            json!({"name": "dev", "extends": "base"}),
        )
        .await;
    let args = json!({
        "name": "base",
        "extends": "dev",
        "set": [{"name": "host", "value": "changed.test"}],
    });
    let result = server.call("update_environment", args).await;
    assert_eq!(result.is_error, Some(true));
    let text = common::text(&result);
    assert!(
        text.contains("environment inheritance cycle: base -> dev -> base"),
        "{text}"
    );
    assert_eq!(read(&server, "base"), BASE);
}

#[tokio::test]
async fn global_environments_take_extends_too() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    for name in ["a", "b"] {
        server
            .ok("create_global_environment", json!({"name": name}))
            .await;
    }
    let doc = server
        .ok(
            "update_global_environment",
            json!({"name": "b", "extends": "a"}),
        )
        .await;
    assert_eq!(doc["extends"], "a", "{doc}");
    let doc = server
        .ok("read_global_environment", json!({"name": "b"}))
        .await;
    assert_eq!(doc["extends"], "a", "{doc}");
    let result = server
        .call(
            "update_global_environment",
            json!({"name": "a", "extends": "b"}),
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        common::text(&result).contains("environment inheritance cycle: a -> b -> a"),
        "{}",
        common::text(&result)
    );
    let missing = server
        .call(
            "update_global_environment",
            json!({"name": "a", "extends": "gone"}),
        )
        .await;
    assert!(
        common::text(&missing).contains("environment `a` extends `gone`, which does not exist"),
        "{}",
        common::text(&missing)
    );
}
