//! Without `--env`, commands use the collection's preset default environment while it exists;
//! `--env` wins and `--no-env` uses none.

use crate::common;

use common::{apinata, check, column, get, run_json, write_tree};
use predicates::str::contains;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const PRESET_ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nextensions:\n  bruno:\n    presets:\n      defaultEnvironment: dev\n";

fn env(name: &str, base: &str) -> String {
    format!("name: {name}\nvariables:\n  - name: base\n    value: \"{base}\"\n")
}

async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// A preset collection: `dev` points at `/ok` (200) and `prod` at `/missing` (404).
fn collection(s: &MockServer, root: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", root),
        (
            "environments/dev.yml",
            &env("dev", &format!("{}/ok", s.uri())),
        ),
        (
            "environments/prod.yml",
            &env("prod", &format!("{}/missing", s.uri())),
        ),
        (
            "a.yml",
            &get("A", 1, "{{base}}", &check("res.status", "eq", "200")),
        ),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_without_env_uses_the_preset() {
    let s = server().await;
    let dir = collection(&s, PRESET_ROOT);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(column(&report, "outcome"), ["passed"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_wins_over_the_preset() {
    let s = server().await;
    let dir = collection(&s, PRESET_ROOT);
    let (code, report) = run_json(dir.path(), &["--env", "prod"]);
    assert_eq!(code, 1, "{report}");
    assert_eq!(column(&report, "outcome"), ["failed"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_env_runs_without_the_preset() {
    let s = server().await;
    let dir = collection(&s, PRESET_ROOT);
    let (code, report) = run_json(dir.path(), &["--no-env"]);
    assert_ne!(code, 0, "{report}");
    let hint = report["results"][0]["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("pass --env"), "{report}");
}

#[test]
fn no_env_conflicts_with_env() {
    let dir = write_tree(&[("opencollection.yml", PRESET_ROOT)]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--env", "dev", "--no-env"])
        .assert()
        .failure()
        .stderr(contains("cannot be used with"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_preset_naming_a_missing_environment_warns_and_runs_with_none() {
    let s = server().await;
    let root = PRESET_ROOT.replace("dev\n", "gone\n");
    let dir = write_tree(&[
        ("opencollection.yml", &root),
        (
            "a.yml",
            &get(
                "A",
                1,
                &format!("{}/ok", s.uri()),
                &check("res.status", "eq", "200"),
            ),
        ),
    ]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .assert()
        .code(0)
        .stderr(contains(
            "warning: default environment \"gone\" not found; using no environment",
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gen_uses_the_preset_too() {
    let s = server().await;
    let dir = collection(&s, PRESET_ROOT);
    apinata()
        .args(["gen", "--target", "curl"])
        .arg(dir.path().join("a.yml"))
        .assert()
        .code(0)
        .stdout(contains(format!("{}/ok", s.uri())));
}
