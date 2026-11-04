//! `--global-env` and `--global-env-var` on `apinata run` and `apinata mcp`.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WHO: &str = "info:\n  name: Who\n  type: http\nhttp:\n  method: GET\n  url: \"{{gbase}}/who\"\n  headers:\n    - name: X-Token\n      value: \"{{tok}}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

async fn who_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/who"))
        .and(header("x-token", "t1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_env_selects_a_profile_global_and_global_env_var_supplies_its_secret() {
    let s = who_server().await;
    let global = format!(
        "name: shared\nvariables:\n  - name: gbase\n    value: {}\n  - secret: true\n    name: tok\n",
        s.uri()
    );
    let profile = write_tree(&[("global-environments/shared.yml", &global)]);
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let out = apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .args([
            "--format",
            "json",
            "--global-env",
            "shared",
            "--global-env-var",
            "tok=t1",
        ])
        .output()
        .expect("runs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_env_var_alone_needs_no_global_file() {
    let s = who_server().await;
    let profile = tempfile::TempDir::new().expect("tempdir");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let base = format!("gbase={}", s.uri());
    let out = apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .args([
            "--format",
            "json",
            "--global-env-var",
            &base,
            "--global-env-var",
            "tok=t1",
        ])
        .output()
        .expect("runs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(!profile.path().join("global-environments").exists());
}

#[test]
fn an_unknown_global_environment_is_a_usage_failure() {
    let profile = tempfile::TempDir::new().expect("tempdir");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .args(["--global-env", "nope"])
        .assert()
        .code(3)
        .stderr(contains(
            "global environment \"nope\" not found; no global environments",
        ));
}

#[test]
fn both_commands_document_the_global_flags() {
    for command in ["run", "mcp"] {
        let out = apinata().args([command, "--help"]).output().expect("runs");
        let help = String::from_utf8(out.stdout).expect("utf-8");
        assert!(help.contains("--global-env <"), "{command}: {help}");
        assert!(
            help.contains("--global-env-var <KEY[=VALUE]>"),
            "{command}: {help}"
        );
    }
}
