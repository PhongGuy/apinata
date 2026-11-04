//! `apinata run` says why a variable is undefined and how to supply it, in text and JSON.

use crate::common;
use common::{ROOT, write_tree};
use serde_json::Value;

const DEV: &str =
    "name: dev\nvariables:\n  - name: base\n    value: b\n  - secret: true\n    name: tok\n";
const TOK_HINT: &str = "tok is a secret in environment dev with no value; pass --env-var tok=VALUE or set it in the app's environment editor";
const BASE_HINT: &str = "base is defined in environment dev; pass --env dev";

/// The binary with a throwaway HOME and XDG dirs, so nothing reads the real keyring or config.
fn apinata() -> (assert_cmd::Command, tempfile::TempDir) {
    let home = tempfile::TempDir::new().expect("home");
    let mut cmd = common::apinata();
    cmd.env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env_remove("XDG_RUNTIME_DIR");
    (cmd, home)
}

fn req(var: &str) -> String {
    format!(
        "info:\n  name: {var}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:1/{{{{{var}}}}}\"\n"
    )
}

fn collection(var: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &req(var)),
    ])
}

fn run(var: &str, args: &[&str]) -> String {
    let dir = collection(var);
    let (mut cmd, _home) = apinata();
    let out = cmd
        .arg("run")
        .arg("--no-keychain")
        .arg(dir.path())
        .args(args)
        .output()
        .expect("runs");
    assert!(!out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn text_shows_the_secret_hint_under_the_failing_row() {
    let text = run("tok", &["--env", "dev"]);
    assert!(text.contains(&format!("\n   hint: {TOK_HINT}\n")), "{text}");
}

#[test]
fn text_shows_the_environment_hint_when_none_is_selected() {
    let text = run("base", &[]);
    assert!(
        text.contains(&format!("\n   hint: {BASE_HINT}\n")),
        "{text}"
    );
}

#[test]
fn json_carries_the_hint_key() {
    let json: Value =
        serde_json::from_str(&run("tok", &["--env", "dev", "--format", "json"])).unwrap();
    assert_eq!(json["results"][0]["hint"], TOK_HINT, "{json}");
    let json: Value = serde_json::from_str(&run("base", &["--format", "json"])).unwrap();
    assert_eq!(json["results"][0]["hint"], BASE_HINT, "{json}");
}

#[test]
fn json_leaves_the_key_out_when_there_is_no_hint() {
    let json: Value =
        serde_json::from_str(&run("nowhere", &["--env", "dev", "--format", "json"])).unwrap();
    assert!(json["results"][0].get("hint").is_none(), "{json}");
}
