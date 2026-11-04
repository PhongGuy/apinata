//! `--workspace` on `apinata run`, `mcp` and `mock`: workspace environments for `--global-env`.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WORKSPACE: &str =
    "opencollection: 1.0.0\ninfo:\n  name: W\n  type: workspace\ncollections: []\n";
const WHO: &str = "info:\n  name: Who\n  type: http\nhttp:\n  method: GET\n  url: \"{{gbase}}/who\"\n  headers:\n    - name: X-Token\n      value: \"{{tok}}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

/// A server that answers 200 on `/who` only for the token `want`.
async fn who_server(want: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/who"))
        .and(header("x-token", want))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn env_file(name: &str, base: &str, token: &str) -> String {
    format!(
        "name: {name}\nvariables:\n  - name: gbase\n    value: {base}\n  - name: tok\n    value: {token}\n"
    )
}

fn run_with(
    profile: &std::path::Path,
    workspace: &std::path::Path,
    extra: &[&str],
) -> std::process::Output {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    apinata()
        .env("APINATA_PROFILE_DIR", profile)
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--workspace"])
        .arg(workspace)
        .args(extra)
        .output()
        .expect("runs")
}

fn code(out: &std::process::Output) -> Option<i32> {
    out.status.code()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_label_selects_a_workspace_environment_without_a_profile_dir() {
    let s = who_server("t1").await;
    let ws = write_tree(&[
        ("workspace.yml", WORKSPACE),
        (
            "environments/shared.yml",
            &env_file("shared", &s.uri(), "t1"),
        ),
    ]);
    let missing = ws.path().join("no-profile");
    let out = run_with(&missing, ws.path(), &["--global-env", "shared (workspace)"]);
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_name_falls_back_to_the_workspace_when_the_profile_lacks_it() {
    let s = who_server("t1").await;
    let ws = write_tree(&[
        ("workspace.yml", WORKSPACE),
        (
            "environments/shared.yml",
            &env_file("shared", &s.uri(), "t1"),
        ),
    ]);
    let profile = tempfile::TempDir::new().expect("tempdir");
    let out = run_with(profile.path(), ws.path(), &["--global-env", "shared"]);
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_label_beats_a_clashing_profile_name_and_the_plain_name_prefers_the_profile() {
    let good = who_server("t1").await;
    let bad = who_server("never").await;
    let ws = write_tree(&[
        ("workspace.yml", WORKSPACE),
        (
            "environments/shared.yml",
            &env_file("shared", &good.uri(), "t1"),
        ),
    ]);
    let profile = write_tree(&[(
        "global-environments/shared.yml",
        &env_file("shared", &bad.uri(), "t1"),
    )]);
    let out = run_with(
        profile.path(),
        ws.path(),
        &["--global-env", "shared (workspace)"],
    );
    assert_eq!(
        code(&out),
        Some(0),
        "label: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let out = run_with(profile.path(), ws.path(), &["--global-env", "shared"]);
    assert_ne!(
        code(&out),
        Some(0),
        "plain name must pick the profile's base url"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_workspace_secret_is_supplied_by_global_env_var() {
    let s = who_server("t1").await;
    let text = format!(
        "name: shared\nvariables:\n  - name: gbase\n    value: {}\n  - secret: true\n    name: tok\n",
        s.uri()
    );
    let ws = write_tree(&[
        ("workspace.yml", WORKSPACE),
        ("environments/shared.yml", &text),
    ]);
    let profile = tempfile::TempDir::new().expect("tempdir");
    let out = run_with(
        profile.path(),
        ws.path(),
        &[
            "--global-env",
            "shared (workspace)",
            "--global-env-var",
            "tok=t1",
        ],
    );
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    // The secret value stays out of the report.
    assert!(!String::from_utf8_lossy(&out.stdout).contains("tok=t1"));
}

#[test]
fn an_unknown_name_is_the_usual_not_found_failure() {
    let ws = write_tree(&[("workspace.yml", WORKSPACE)]);
    let profile = tempfile::TempDir::new().expect("tempdir");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .arg("--workspace")
        .arg(ws.path())
        .args(["--global-env", "nope (workspace)"])
        .assert()
        .code(3)
        .stderr(contains(
            "global environment \"nope (workspace)\" not found; no global environments",
        ));
}

#[test]
fn a_folder_without_workspace_yml_is_an_error_naming_it() {
    let ws = tempfile::TempDir::new().expect("tempdir");
    let profile = tempfile::TempDir::new().expect("tempdir");
    let out = run_with(profile.path(), ws.path(), &["--global-env", "shared"]);
    assert_ne!(code(&out), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a workspace"));
}

#[test]
fn a_malformed_workspace_yml_is_an_error() {
    let ws = write_tree(&[("workspace.yml", "- just\n- a list\n")]);
    let profile = tempfile::TempDir::new().expect("tempdir");
    let out = run_with(profile.path(), ws.path(), &["--global-env", "shared"]);
    assert_ne!(code(&out), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("workspace.yml"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_leaves_workspace_files_byte_identical() {
    let s = who_server("t1").await;
    let env = env_file("shared", &s.uri(), "t1");
    let ws = write_tree(&[
        ("workspace.yml", WORKSPACE),
        ("environments/shared.yml", &env),
    ]);
    let profile = tempfile::TempDir::new().expect("tempdir");
    let out = run_with(
        profile.path(),
        ws.path(),
        &["--global-env", "shared (workspace)"],
    );
    assert_eq!(code(&out), Some(0));
    assert_eq!(
        std::fs::read_to_string(ws.path().join("workspace.yml")).unwrap(),
        WORKSPACE
    );
    assert_eq!(
        std::fs::read_to_string(ws.path().join("environments/shared.yml")).unwrap(),
        env
    );
}

#[test]
fn the_three_commands_document_workspace() {
    for command in ["run", "mcp", "mock"] {
        let out = apinata().args([command, "--help"]).output().expect("runs");
        let help = String::from_utf8(out.stdout).expect("utf-8");
        assert!(help.contains("--workspace <DIR>"), "{command}: {help}");
        assert!(!help.contains("Bruno"), "{command}");
    }
}
