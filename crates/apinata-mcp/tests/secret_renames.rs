//! `list_secret_renames`: the stored secrets a pulled rename left under an old name, by name
//! and status only. Moving them is left to the CLI and the app.

use crate::common;
use crate::git_support;

use std::fs;

use common::{ROOT, TestServer};
use git_support::{Sandbox, write};
use serde_json::json;

fn env(name: &str, secrets: &[&str]) -> String {
    let mut text = format!("name: {name}\nvariables:\n  - name: host\n    value: h.test\n");
    for v in secrets {
        text.push_str(&format!("  - name: {v}\n    secret: true\n"));
    }
    text
}

/// A served repository whose second commit renames dev to staging (token free, key taken) and
/// replaces qa's `old` with two new secrets; returns the server and both commits.
async fn renamed(sb: &Sandbox) -> (TestServer, String, String) {
    let dev = env("dev", &["token", "key"]);
    let qa = env("qa", &["old"]);
    let server = TestServer::start_with(
        &[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", dev.as_str()),
            ("environments/qa.yml", qa.as_str()),
        ],
        |options| {
            let git = sb
                .options()
                .env("GIT_CEILING_DIRECTORIES", options.root().parent().unwrap());
            options.git(git)
        },
    )
    .await;
    let root = server.root().to_path_buf();
    sb.git(&root, &["init", "-q", "-b", "main"]);
    sb.commit_all(&root, "base");
    let base = sb.git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    fs::remove_file(root.join("environments/dev.yml")).unwrap();
    write(
        &root,
        "environments/staging.yml",
        &env("staging", &["token", "key"]),
    );
    write(&root, "environments/qa.yml", &env("qa", &["a", "b"]));
    sb.commit_all(&root, "edit");
    let tip = sb.git(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    for (e, v, value) in [
        ("dev", "token", "SECRETVALUE-1"),
        ("dev", "key", "SECRETVALUE-2"),
        ("staging", "key", "SECRETVALUE-3"),
        ("qa", "old", "SECRETVALUE-4"),
    ] {
        server.set_secret(e, v, value);
    }
    (server, base, tip)
}

#[tokio::test]
async fn lists_each_rename_by_name_and_status_without_values() {
    let sb = Sandbox::new();
    let (server, base, tip) = renamed(&sb).await;
    let got = server
        .ok("list_secret_renames", json!({"since": base[..7]}))
        .await;
    let name = |e: &str, v: &str| json!({"environment": e, "variable": v});
    let target = |e: &str, v: &str, s: &str| json!({"environment": e, "variable": v, "state": s});
    assert_eq!(
        got,
        json!({
            "from": base,
            "to": tip,
            "renames": [
                {"from": name("dev", "key"), "to": [target("staging", "key", "taken")], "status": "taken"},
                {"from": name("dev", "token"), "to": [target("staging", "token", "free")], "status": "ready"},
                {
                    "from": name("qa", "old"),
                    "to": [target("qa", "a", "free"), target("qa", "b", "free")],
                    "status": "ambiguous"
                },
            ]
        })
    );
    assert!(!got.to_string().contains("SECRETVALUE"));
    assert_eq!(
        server.secret("dev", "token").as_deref(),
        Some("SECRETVALUE-1")
    );
}

#[tokio::test]
async fn without_since_it_uses_orig_head_or_asks_for_since() {
    let sb = Sandbox::new();
    let (server, base, _) = renamed(&sb).await;
    let err = server.err("list_secret_renames", json!({})).await;
    assert!(err.contains("since"), "{err}");
    sb.git(server.root(), &["update-ref", "ORIG_HEAD", &base]);
    let got = server.ok("list_secret_renames", json!({})).await;
    assert_eq!(got["from"], base);
    assert_eq!(got["renames"].as_array().map(Vec::len), Some(3));
}

#[tokio::test]
async fn the_tool_is_read_only_and_leaves_moving_to_the_cli_and_app() {
    let server = TestServer::start(&[("opencollection.yml", ROOT)]).await;
    let tools = server.tools().await;
    let tool = tools
        .iter()
        .find(|t| t.name == "list_secret_renames")
        .expect("listed");
    let annotations = tool.annotations.as_ref().expect("annotations");
    assert_eq!(annotations.read_only_hint, Some(true));
    let description = tool.description.as_deref().unwrap_or_default();
    assert!(description.contains("apinata env renames"), "{description}");
}
