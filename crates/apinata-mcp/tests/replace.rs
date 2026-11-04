//! `replace`: a dry run by default, the CLI's summary on a real run, and no secret ever matched.

use crate::common;

use common::{ROOT, TestServer};
use serde_json::json;
use std::fs;

/// A request file in the writer's canonical form, so a save changes only the matched text.
fn request(name: &str, seq: u32, method: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\n\nhttp:\n  method: {method}\n  url: {url}\n"
    )
}

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n";

/// Two requests with `needle` in the URL, one lower case and one upper case, plus a secret.
async fn server() -> TestServer {
    TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, "GET", "http://x/needle")),
        ("b.yml", &request("b", 2, "GET", "http://x/NEEDLE")),
        ("c.yml", &request("c", 3, "GET", "http://x/other")),
        ("environments/dev.yml", DEV),
        (".env", "token=s3cr3t-dotenv-value\n"),
    ])
    .await
}

fn read(server: &TestServer, rel: &str) -> String {
    fs::read_to_string(server.root().join(rel)).expect("read")
}

#[tokio::test]
async fn schema_requires_only_pattern_and_replacement_and_is_not_read_only() {
    let server = server().await;
    let tools = server.tools().await;
    let tool = tools.iter().find(|t| t.name == "replace").expect("listed");
    let props = tool.input_schema["properties"].as_object().expect("props");
    for name in ["pattern", "replacement", "regex", "case", "dry_run"] {
        assert!(props.contains_key(name), "missing {name}");
    }
    let mut required: Vec<&str> = tool.input_schema["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    required.sort();
    assert_eq!(required, ["pattern", "replacement"]);
    let hints = tool.annotations.as_ref().expect("annotations");
    assert_ne!(hints.read_only_hint, Some(true));
    assert_eq!(hints.destructive_hint, Some(false));
    assert_eq!(hints.open_world_hint, Some(false));
    let description = tool.description.as_deref().unwrap_or_default();
    assert!(
        description.contains("dry_run defaults to true"),
        "{description}"
    );
}

#[tokio::test]
async fn omitting_dry_run_lists_the_hits_and_writes_nothing() {
    let server = server().await;
    let before = (read(&server, "a.yml"), read(&server, "b.yml"));
    let out = server
        .ok(
            "replace",
            json!({"pattern": "needle", "replacement": "thread"}),
        )
        .await;
    assert_eq!(out["dry_run"], true);
    assert_eq!(out["hits"][0]["path"], "a.yml");
    assert_eq!(out["hits"][1]["path"], "b.yml");
    assert_eq!(out["summary"], "Would replace in 2 files");
    assert_eq!((read(&server, "a.yml"), read(&server, "b.yml")), before);
}

#[tokio::test]
async fn a_real_run_writes_only_the_matched_text_and_lists_the_changed_files() {
    let server = server().await;
    let before = read(&server, "a.yml");
    let out = server
        .ok(
            "replace",
            json!({"pattern": "needle", "replacement": "thread", "dry_run": false}),
        )
        .await;
    assert_eq!(out["dry_run"], false);
    assert_eq!(out["replaced"], 2);
    assert_eq!(out["files_changed"], 2);
    assert_eq!(out["summary"], "Replaced 2 in 2 files");
    assert_eq!(out["changed_files"], json!(["a.yml", "b.yml"]));
    assert_eq!(out["scripts_changed"], false);
    assert_eq!(read(&server, "a.yml"), before.replace("needle", "thread"));
    assert_eq!(
        read(&server, "c.yml"),
        request("c", 3, "GET", "http://x/other")
    );
}

#[tokio::test]
async fn regex_groups_are_expanded_in_the_replacement() {
    let server = server().await;
    server
        .ok(
            "replace",
            json!({"pattern": "x/(\\w+)", "replacement": "y/$1-1", "regex": true,
                   "case": true, "dry_run": false}),
        )
        .await;
    assert!(read(&server, "a.yml").contains("http://y/needle-1"));
    assert!(read(&server, "b.yml").contains("http://y/NEEDLE-1"));
}

#[tokio::test]
async fn case_sensitivity_is_opt_in() {
    let server = server().await;
    let out = server
        .ok(
            "replace",
            json!({"pattern": "needle", "replacement": "thread", "case": true,
                   "dry_run": false}),
        )
        .await;
    assert_eq!(out["summary"], "Replaced 1 in 1 file");
    assert!(read(&server, "b.yml").contains("NEEDLE"));
}

#[tokio::test]
async fn an_invalid_regex_is_an_error_and_writes_nothing() {
    let server = server().await;
    let before = read(&server, "a.yml");
    for dry_run in [true, false] {
        let message = server
            .err(
                "replace",
                json!({"pattern": "(", "replacement": "x", "regex": true,
                       "dry_run": dry_run}),
            )
            .await;
        assert!(message.contains("invalid regular expression"), "{message}");
    }
    assert_eq!(read(&server, "a.yml"), before);
}

#[tokio::test]
async fn secret_values_are_never_matched_or_written() {
    let server = server().await;
    let env_before = read(&server, "environments/dev.yml");
    for dry_run in [true, false] {
        let out = server
            .ok(
                "replace",
                json!({"pattern": "s3cr3t-dotenv-value", "replacement": "x",
                       "dry_run": dry_run}),
            )
            .await;
        if dry_run {
            assert_eq!(out["hits"].as_array().unwrap().len(), 0);
        } else {
            assert_eq!(out["replaced"], 0);
        }
    }
    assert_eq!(read(&server, ".env"), "token=s3cr3t-dotenv-value\n");
    assert_eq!(read(&server, "environments/dev.yml"), env_before);
}

#[tokio::test]
async fn a_replaced_script_sets_scripts_changed_and_notes_the_trust_prompt() {
    let script = "info:\n  name: s\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://x/ok\n\nruntime:\n  scripts:\n    - type: before-request\n      code: console.log(\"needle\");\n";
    let server = TestServer::start(&[("opencollection.yml", ROOT), ("s.yml", script)]).await;
    let out = server
        .ok(
            "replace",
            json!({"pattern": "needle", "replacement": "thread", "dry_run": false}),
        )
        .await;
    assert_eq!(out["scripts_changed"], true);
    let note = out["note"].as_str().expect("note");
    assert!(note.contains("trust"), "{note}");
    assert_eq!(read(&server, "s.yml"), script.replace("needle", "thread"));
}

#[tokio::test]
async fn a_real_run_lists_every_changed_file_past_the_hit_cap() {
    let files: Vec<(String, String)> = (0..501)
        .map(|i| {
            (
                format!("r{i:03}.yml"),
                request("same", i + 1, "GET", "http://x/needle"),
            )
        })
        .collect();
    let mut refs: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    refs.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let server = TestServer::start(&refs).await;
    let dry = server
        .ok(
            "replace",
            json!({"pattern": "needle", "replacement": "thread"}),
        )
        .await;
    assert_eq!(dry["capped"], true);
    assert!(
        dry["summary"]
            .as_str()
            .unwrap()
            .starts_with("Would replace in at least"),
        "{dry}"
    );
    let out = server
        .ok(
            "replace",
            json!({"pattern": "needle", "replacement": "thread", "dry_run": false}),
        )
        .await;
    assert_eq!(out["files_changed"], 501);
    assert_eq!(out["changed_files"].as_array().unwrap().len(), 501);
}

#[tokio::test]
async fn same_named_requests_in_two_folders_are_both_listed() {
    let server = TestServer::start(&[
        ("opencollection.yml", ROOT),
        ("one/a.yml", &request("same", 1, "GET", "http://x/needle")),
        ("two/a.yml", &request("same", 1, "GET", "http://x/needle")),
    ])
    .await;
    let out = server
        .ok(
            "replace",
            json!({"pattern": "needle", "replacement": "thread", "dry_run": false}),
        )
        .await;
    assert_eq!(out["changed_files"], json!(["one/a.yml", "two/a.yml"]));
}
