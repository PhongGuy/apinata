//! `run --env-file PATH`: an environment read from a file outside the collection, in either
//! kind, under `--env-var`, and never written into the collection.

use crate::common;

use common::{apinata, write_tree};
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PM_SECRET: &str = "pm-s3cret-Zq81";
const PRESET_ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nextensions:\n  bruno:\n    presets:\n      defaultEnvironment: dev\n";
const WHO: &str = "info:\n  name: Who\n  type: http\nhttp:\n  method: GET\n  url: \"{{base}}/who?key={{token}}\"\n  headers:\n    - name: X-Who\n      value: \"{{who}}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

/// A server that answers 200 only to `X-Who: who`.
async fn server(who: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/who"))
        .and(header("x-who", who))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

/// A collection whose preset default environment `dev` sends `X-Who: dev`.
fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", PRESET_ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: base\n    value: http://127.0.0.1:9\n  - name: who\n    value: dev\n  - name: token\n    value: t\n",
        ),
        ("who.yml", WHO),
    ])
}

fn postman(base: &str) -> String {
    format!(
        r#"{{"name": "CI", "values": [
  {{"key": "base", "value": "{base}", "enabled": true}},
  {{"key": "who", "value": "postman", "enabled": true}},
  {{"key": "token", "value": "{PM_SECRET}", "type": "secret", "enabled": true}}
]}}"#
    )
}

fn yaml(base: &str) -> String {
    format!(
        "variables:\n  - name: base\n    value: {base}\n  - name: who\n    value: yaml\n  - name: token\n    value: t\n"
    )
}

fn text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// `apinata run COLLECTION --no-keychain ARGS`.
fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    apinata()
        .arg("run")
        .arg(dir)
        .arg("--no-keychain")
        .args(args)
        .output()
        .expect("runs")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_postman_export_supplies_the_variables() {
    let s = server("postman").await;
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("ci.json");
    std::fs::write(&file, postman(&s.uri())).unwrap();
    let out = run(dir.path(), &["--env-file", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let envs: Vec<_> = std::fs::read_dir(dir.path().join("environments"))
        .unwrap()
        .collect();
    assert_eq!(envs.len(), 1, "the file is not written into the collection");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_collection_file_supplies_the_variables() {
    let s = server("yaml").await;
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("ci.yml");
    std::fs::write(&file, yaml(&s.uri())).unwrap();
    let out = run(dir.path(), &["--env-file", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_var_beats_the_file() {
    let s = server("flag").await;
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("ci.yml");
    std::fs::write(&file, yaml(&s.uri())).unwrap();
    let out = run(
        dir.path(),
        &[
            "--env-file",
            file.to_str().unwrap(),
            "--env-var",
            "who=flag",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_postman_secret_is_redacted_in_verbose_output() {
    let s = server("postman").await;
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("ci.json");
    std::fs::write(&file, postman(&s.uri())).unwrap();
    let out = run(
        dir.path(),
        &["--env-file", file.to_str().unwrap(), "--verbose"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let all = text(&out);
    assert!(!all.contains(PM_SECRET), "{all}");
    assert!(all.contains("key=[redacted]"), "{all}");
}

#[test]
fn env_and_env_file_together_are_a_usage_error() {
    let dir = collection();
    let out = run(dir.path(), &["--env", "dev", "--env-file", "ci.json"]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
    assert!(text(&out).contains("cannot be used with"), "{}", text(&out));
}

#[test]
fn a_missing_file_is_a_usage_error_naming_it() {
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("absent.json");
    let out = run(dir.path(), &["--env-file", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
    let all = text(&out);
    assert!(all.contains("could not read"), "{all}");
    assert!(all.contains("absent.json"), "{all}");
}

#[test]
fn a_file_of_another_shape_is_a_usage_error() {
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("other.json");
    std::fs::write(&file, "{\"hello\": 1}").unwrap();
    let out = run(dir.path(), &["--env-file", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
    assert!(
        text(&out).contains("is not an environment file"),
        "{}",
        text(&out)
    );
}

#[test]
fn no_env_and_env_file_together_are_a_usage_error() {
    let dir = collection();
    let out = run(dir.path(), &["--no-env", "--env-file", "ci.json"]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
    assert!(text(&out).contains("cannot be used with"), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_env_var_over_a_postman_secret_stays_redacted() {
    let s = server("postman").await;
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("ci.json");
    std::fs::write(&file, postman(&s.uri())).unwrap();
    let out = run(
        dir.path(),
        &[
            "--env-file",
            file.to_str().unwrap(),
            "--env-var",
            "token=0verride-Zq81",
            "--verbose",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let all = text(&out);
    assert!(!all.contains("0verride-Zq81"), "{all}");
    assert!(all.contains("key=[redacted]"), "{all}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_json_report_leaves_out_a_postman_secret() {
    let s = server("postman").await;
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("ci.json");
    let report = outside.path().join("report.json");
    std::fs::write(&file, postman(&s.uri())).unwrap();
    // A failing assertion whose expected value is the secret puts it in the report's verdict.
    std::fs::write(
        dir.path().join("who.yml"),
        WHO.replace("value: \"200\"", "value: \"{{token}}\""),
    )
    .unwrap();
    let out = run(
        dir.path(),
        &[
            "--env-file",
            file.to_str().unwrap(),
            "--reporter-json",
            report.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let json = std::fs::read_to_string(&report).unwrap();
    assert!(json.contains("res.status"), "{json}");
    assert!(!json.contains(PM_SECRET), "{json}");
}

#[test]
fn a_postman_secret_without_a_value_points_at_env_var_not_the_app() {
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("ci.json");
    std::fs::write(
        &file,
        r#"{"name": "CI", "values": [
  {"key": "base", "value": "http://127.0.0.1:9"},
  {"key": "who", "value": "x"},
  {"key": "token", "value": "", "type": "secret"}
]}"#,
    )
    .unwrap();
    let out = run(dir.path(), &["--env-file", file.to_str().unwrap()]);
    let all = text(&out);
    assert_ne!(out.status.code(), Some(0), "{all}");
    assert!(
        all.contains("pass --env-var token=VALUE or give it a value in the file"),
        "{all}"
    );
    assert!(!all.contains("editor"), "{all}");
}

#[test]
fn a_hoppscotch_export_is_a_usage_error() {
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("hopp.json");
    std::fs::write(
        &file,
        r#"{"v": 2, "name": "H", "variables": [{"key": "base", "initialValue": "x", "currentValue": "x", "secret": false}]}"#,
    )
    .unwrap();
    let out = run(dir.path(), &["--env-file", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
    assert!(
        text(&out).contains("format is not supported"),
        "{}",
        text(&out)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_renamed_postman_key_is_a_warning() {
    let s = server("postman").await;
    let dir = collection();
    let outside = tempfile::TempDir::new().unwrap();
    let file = outside.path().join("ci.json");
    let body = postman(&s.uri()).replace(
        r#""values": ["#,
        r#""values": [{"key": "my key", "value": "v"},"#,
    );
    std::fs::write(&file, body).unwrap();
    let out = run(dir.path(), &["--env-file", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning: ")
            && stderr.contains("the variable `my key` was renamed to `my_key`"),
        "{stderr}"
    );
}
