//! `apinata item restore --pruned` and `apinata item scrub-userinfo`, each against its MCP
//! twin (`restore_pruned`, `scrub_example_userinfo`) with the same JSON.

use crate::cli_item::{Copy, stdout};
use crate::common::{ROOT, apinata, write_tree};

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../apinata-core/tests/fixtures/openapi")
        .join(name)
}

/// `apinata <args>` in the copy's scratch home, kept away from any real secret store.
fn run(copy: &Copy, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = apinata();
    for (k, v) in copy.envs() {
        cmd.env(k, v);
    }
    cmd.env(
        "DBUS_SESSION_BUS_ADDRESS",
        "unix:path=/nonexistent/apinata-bus",
    )
    .args(args)
    .timeout(std::time::Duration::from_secs(30))
    .assert()
}

/// A copy of the v1 import, with its legacy request pruned by a v2 update.
fn pruned() -> Copy {
    let parent = tempfile::tempdir().expect("tempdir");
    apinata()
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/apinata-bus",
        )
        .env("XDG_RUNTIME_DIR", parent.path())
        .args(["import", "collection", "--no-keychain"])
        .arg(fixture("update-v1.json"))
        .arg("--into")
        .arg(parent.path())
        .assert()
        .code(0);
    let copy = Copy::of_dir(&parent.path().join("Shop"));
    let spec = fixture("update-v2.json");
    let root = copy.root();
    run(
        &copy,
        &[
            "import",
            "collection",
            "--update",
            spec.to_str().expect("utf-8"),
            "--into",
            root.to_str().expect("utf-8"),
            "--prune",
        ],
    )
    .code(0);
    assert!(!copy.root().join("legacy/removeLegacy.yml").exists());
    copy
}

#[test]
fn restore_pruned_puts_the_pruned_request_back() {
    let a = pruned();
    a.item(&["restore", "--pruned"])
        .code(0)
        .stdout("restored legacy/removeLegacy.yml\n");
    assert!(a.root().join("legacy/removeLegacy.yml").is_file());
}

#[tokio::test]
async fn restore_pruned_json_matches_the_mcp_tool() {
    let a = pruned();
    let out = a.item(&["--format", "json", "restore", "--pruned"]).code(0);
    let cli: Value = serde_json::from_str(&stdout(&out)).expect("json");
    let want = json!({"restored": ["legacy/removeLegacy.yml"], "warnings": []});
    assert_eq!(cli, want);
    let b = pruned();
    let mcp = b.mcp("restore_pruned", json!({})).await.expect("restored");
    assert_eq!(mcp, want);
    assert_eq!(a.tree(), b.tree());
}

#[test]
fn restore_pruned_refuses_a_plain_delete() {
    let a = pruned();
    a.item(&["rm", "people/getUser.yml"]).code(0);
    a.item(&["restore", "--pruned"])
        .code(3)
        .stdout("")
        .stderr(predicates::str::contains(
            "the newest delete was not an import prune",
        ));
    assert!(!a.root().join("people/getUser.yml").exists());
}

/// The batch id a refusal lists first.
fn listed_batch(message: &str) -> String {
    let (_, rest) = message.split_once("newest first: ").expect(message);
    let (batch, _) = rest.split_once(';').expect(message);
    batch.to_owned()
}

#[test]
fn a_batch_needs_pruned() {
    let a = pruned();
    a.item(&["restore", "--batch", "any"])
        .code(3)
        .stdout("")
        .stderr(predicates::str::contains("--pruned"));
    assert!(!a.root().join("legacy/removeLegacy.yml").exists());
}

#[test]
fn an_unknown_batch_is_refused() {
    let a = pruned();
    a.item(&["restore", "--pruned", "--batch", "nope"])
        .code(3)
        .stdout("")
        .stderr(predicates::str::contains(
            "no prune nope in the trash record",
        ));
    assert!(!a.root().join("legacy/removeLegacy.yml").exists());
}

#[tokio::test]
async fn restore_pruned_with_a_batch_json_matches_the_mcp_tool() {
    let a = pruned();
    a.item(&["rm", "people/getUser.yml"]).code(0);
    let refused = a.item(&["restore", "--pruned"]).code(3);
    let message = String::from_utf8_lossy(&refused.get_output().stderr).into_owned();
    let batch = listed_batch(&message);
    let args = ["--format", "json", "restore", "--pruned", "--batch", &batch];
    let cli: Value = serde_json::from_str(&stdout(&a.item(&args).code(0))).expect("json");
    assert_eq!(
        cli,
        json!({"restored": ["legacy/removeLegacy.yml"], "warnings": []})
    );

    let b = pruned();
    b.mcp("delete_item", json!({"path": "people/getUser.yml"}))
        .await
        .expect("deleted");
    let refused = b.mcp("restore_pruned", json!({})).await.unwrap_err();
    let batch = listed_batch(&refused);
    let mcp = b
        .mcp("restore_pruned", json!({"batch": batch}))
        .await
        .expect("restored");
    assert_eq!(mcp, cli);
    assert_eq!(a.tree(), b.tree());
}

const LEAKY: &str = "info:
  name: leaky
  type: http
  seq: 1

http:
  method: GET
  url: https://u:pa55word@x.test/

examples:
  - name: one
    request:
      url: https://u:pa55word@x.test/a
      method: GET
    response:
      status: 200
";

fn leaky() -> Copy {
    Copy::of(&write_tree(&[
        ("opencollection.yml", ROOT),
        ("leaky.yml", LEAKY),
    ]))
}

#[test]
fn scrub_userinfo_is_a_dry_run_by_default() {
    let a = leaky();
    let out = a.item(&["scrub-userinfo"]).code(0);
    assert_eq!(
        stdout(&out),
        "would scrub leaky.yml example \"one\": https://x.test/a\n\
         run again with --write to save\n"
    );
    assert!(!stdout(&out).contains("pa55word"));
    assert_eq!(
        fs::read_to_string(a.root().join("leaky.yml")).unwrap(),
        LEAKY
    );
}

#[test]
fn scrub_userinfo_writes_with_the_flag() {
    let a = leaky();
    a.item(&["scrub-userinfo", "--write"])
        .code(0)
        .stdout("scrubbed leaky.yml example \"one\": https://x.test/a\n");
    let text = fs::read_to_string(a.root().join("leaky.yml")).unwrap();
    assert_eq!(text, LEAKY.replace("u:pa55word@x.test/a", "x.test/a"));
    a.item(&["scrub-userinfo"])
        .code(0)
        .stdout("no saved example holds userinfo\n");
}

#[tokio::test]
async fn scrub_userinfo_json_matches_the_mcp_tool() {
    for write in [false, true] {
        let a = leaky();
        let mut args = vec!["--format", "json", "scrub-userinfo"];
        if write {
            args.push("--write");
        }
        let cli: Value = serde_json::from_str(&stdout(&a.item(&args).code(0))).expect("json");
        assert_eq!(
            cli,
            json!({"written": write, "changes": [
                {"item": "leaky.yml", "example": "one", "url": "https://x.test/a"}
            ]})
        );
        let b = leaky();
        let mcp = b
            .mcp("scrub_example_userinfo", json!({"write": write}))
            .await
            .expect("scrubbed");
        assert_eq!(mcp, cli);
        assert_eq!(a.tree(), b.tree());
    }
}
