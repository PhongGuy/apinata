//! `create_environment`, `rename_environment` and `delete_environment`: Bruno's bytes, secrets
//! that follow a rename, and the refusal to delete an environment holding a secret.

use crate::common;

use common::{ROOT, TestServer, files_under, research};
use serde_json::{Value, json};
use std::fs;

const SECRET: &str = "sentinel-s3cr3t value";

fn expected(id: &str, file: &str) -> String {
    let path = research("2026-10-09-config-edit-parity/expected")
        .join(id)
        .join(file);
    fs::read_to_string(path).expect("expected bytes")
}

#[tokio::test]
async fn creates_and_renames_match_brunos_bytes() {
    let cases: [(&str, &str, &str, Value, &[&str]); 4] = [
        (
            "env-op-create",
            "env-inheritance",
            "create_environment",
            json!({"name": "dev"}),
            &["environments/dev copy.yml"],
        ),
        (
            "env-op-create-sanitized",
            "env-inheritance",
            "create_environment",
            json!({"name": " a/b?. "}),
            &["environments/a-b-.yml"],
        ),
        (
            "env-op-create-fresh",
            "cli-docs",
            "create_environment",
            json!({"name": "local"}),
            &["environments/local.yml"],
        ),
        (
            "env-op-rename",
            "env-inheritance",
            "rename_environment",
            json!({"from": "base", "to": "root"}),
            &[
                "environments/dev.yml",
                "environments/root.yml",
                "environments/scripted.yml",
                "environments/staging.yml",
            ],
        ),
    ];
    let mut failures = Vec::new();
    for (id, collection, tool, args, files) in cases {
        let input = research("2026-10-09-edit-parity/normalized").join(collection);
        let server = TestServer::start_owned(&files_under(&input)).await;
        let result = server.call(tool, args).await;
        for file in files {
            let actual = fs::read_to_string(server.root().join(file)).unwrap_or_default();
            if result.is_error == Some(true) || actual != expected(id, file) {
                failures.push(format!(
                    "{id} {file}: {}\n  actual {actual:?}",
                    common::text(&result)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn create_returns_the_name_used() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let made = server
        .ok("create_environment", json!({"name": " a/b?. "}))
        .await;
    assert_eq!(made, json!({"name": "a-b-", "warnings": []}));
}

#[tokio::test]
async fn rename_moves_secret_values_without_returning_them() {
    let env = "name: prod\nvariables:\n  - secret: true\n    name: token\n";
    let server =
        TestServer::start(&[("opencollection.yml", ROOT), ("environments/prod.yml", env)]).await;
    server.set_secret("prod", "token", SECRET);
    let renamed = server
        .ok("rename_environment", json!({"from": "prod", "to": "live"}))
        .await;
    assert_eq!(renamed, json!({"name": "live", "warnings": []}));
    assert_eq!(server.secret("live", "token").as_deref(), Some(SECRET));
    assert_eq!(server.secret("prod", "token"), None);
}

#[tokio::test]
async fn delete_removes_a_plain_environment_and_refuses_one_with_a_secret() {
    let prod =
        "name: prod\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/prod.yml", prod),
        (
            "environments/tmp.yml",
            "name: tmp\nvariables:\n  - name: a\n    value: b\n",
        ),
    ])
    .await;
    server.set_secret("prod", "token", SECRET);
    let err = server
        .err("delete_environment", json!({"name": "prod"}))
        .await;
    assert!(err.contains("secret") && err.contains("app"), "{err}");
    assert!(!err.contains(SECRET), "{err}");
    let kept = fs::read_to_string(server.root().join("environments/prod.yml")).expect("read");
    assert_eq!(kept, prod);
    assert_eq!(server.secret("prod", "token").as_deref(), Some(SECRET));
    let deleted = server
        .ok("delete_environment", json!({"name": "tmp"}))
        .await;
    assert_eq!(deleted, json!({"deleted": "tmp"}));
    assert!(!server.root().join("environments/tmp.yml").exists());
    let err = server
        .err("delete_environment", json!({"name": "nope"}))
        .await;
    assert!(err.contains("nope"), "{err}");
}

#[tokio::test]
async fn delete_environment_refuses_unknown_fields_and_deletes_nothing() {
    let tmp = "name: tmp\nvariables:\n  - name: a\n    value: b\n";
    let server =
        TestServer::start(&[("opencollection.yml", ROOT), ("environments/tmp.yml", tmp)]).await;
    let outcome = server
        .try_call("delete_environment", json!({"name": "tmp", "x": 1}))
        .await;
    assert!(
        outcome.as_ref().map_or(true, |r| r.is_error == Some(true)),
        "{outcome:?}"
    );
    assert!(server.root().join("environments/tmp.yml").is_file());
}

#[tokio::test]
async fn only_delete_environment_is_destructive() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    for name in [
        "create_environment",
        "rename_environment",
        "delete_environment",
    ] {
        let tool = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name}"));
        let hints = tool.annotations.clone().expect("annotated");
        assert_eq!(
            hints.destructive_hint,
            Some(name == "delete_environment"),
            "{name}"
        );
        assert_eq!(hints.open_world_hint, Some(false), "{name}");
    }
}

#[tokio::test]
async fn duplicate_copies_the_file_and_secret_values_without_returning_them() {
    let env = "name: prod\nextends: base\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", "name: base\n"),
        ("environments/prod.yml", env),
    ])
    .await;
    server.set_secret("prod", "token", SECRET);
    let copied = server
        .ok("duplicate_environment", json!({"name": "prod"}))
        .await;
    assert_eq!(copied, json!({"name": "prod copy", "warnings": []}));
    let file = fs::read_to_string(server.root().join("environments/prod copy.yml")).expect("read");
    assert_eq!(file, env.replacen("name: prod\n", "name: prod copy\n", 1));
    assert!(!file.contains(SECRET));
    assert_eq!(server.secret("prod copy", "token").as_deref(), Some(SECRET));
    assert_eq!(server.secret("prod", "token").as_deref(), Some(SECRET));
    let err = server
        .err("duplicate_environment", json!({"name": "nope"}))
        .await;
    assert!(err.contains("nope"), "{err}");
}

#[tokio::test]
async fn delete_refuses_an_environment_others_extend_and_keeps_its_file() {
    let base = "name: base\nvariables:\n  - name: a\n    value: b\n";
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", base),
        ("environments/dev.yml", "name: dev\nextends: base\n"),
    ])
    .await;
    let err = server
        .err("delete_environment", json!({"name": "base"}))
        .await;
    assert!(
        err.contains("environment `base` is extended by `dev`; change `extends` in each first"),
        "{err}"
    );
    let kept = fs::read_to_string(server.root().join("environments/base.yml")).expect("read");
    assert_eq!(kept, base);
}
