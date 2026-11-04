//! `--env-var KEY` and `--global-env-var KEY` read the value from the process environment.

use crate::common;

use common::{ROOT, apinata, write_tree};
use wiremock::matchers::{header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "s3cr3t-Zq81-distinct";
const WHO: &str = "info:\n  name: Who\n  type: http\nhttp:\n  method: GET\n  url: \"{{base}}/who\"\n  headers:\n    - name: X-Token\n      value: \"{{TOKEN}}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
const WHO_G: &str = "info:\n  name: Who\n  type: http\nhttp:\n  method: GET\n  url: \"{{base}}/who?key={{TOKEN}}\"\n  headers:\n    - name: X-Token\n      value: \"{{TOKEN}}\"\n";

async fn server(token: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/who"))
        .and(header("x-token", token))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bare_key_reads_the_value_from_the_process_environment() {
    let s = server(SECRET).await;
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let base = format!("base={}", s.uri());
    let out = apinata()
        .env("TOKEN", SECRET)
        .arg("run")
        .arg(dir.path())
        .args(["--no-keychain", "--env-var", &base, "--env-var", "TOKEN"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_equals_value_is_unchanged() {
    let s = server("plain").await;
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let base = format!("base={}", s.uri());
    let out = apinata()
        .env("TOKEN", SECRET)
        .arg("run")
        .arg(dir.path())
        .args([
            "--no-keychain",
            "--env-var",
            &base,
            "--env-var",
            "TOKEN=plain",
        ])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_set_but_empty_variable_is_the_empty_string() {
    let s = server("").await;
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let base = format!("base={}", s.uri());
    let out = apinata()
        .env("TOKEN", "")
        .arg("run")
        .arg(dir.path())
        .args(["--no-keychain", "--env-var", &base, "--env-var", "TOKEN"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_global_flag_has_the_same_bare_form() {
    let s = server(SECRET).await;
    let profile = tempfile::TempDir::new().expect("tempdir");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let base = format!("base={}", s.uri());
    let out = apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .env("TOKEN", SECRET)
        .arg("run")
        .arg(dir.path())
        .args([
            "--no-keychain",
            "--env-var",
            &base,
            "--global-env-var",
            "TOKEN",
        ])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
}

#[test]
fn an_unset_variable_is_a_usage_error_naming_it() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    for sub in ["run", "check", "mock", "mcp"] {
        let out = apinata()
            .env_remove("APINATA_UNSET_TOKEN_VAR")
            .arg(sub)
            .arg(dir.path())
            .args(["--env-var", "APINATA_UNSET_TOKEN_VAR"])
            .output()
            .expect("runs");
        assert_eq!(out.status.code(), Some(3), "{sub}: {}", text(&out));
        assert!(
            text(&out).contains("environment variable APINATA_UNSET_TOKEN_VAR is not set"),
            "{sub}: {}",
            text(&out)
        );
    }
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--global-env-var", "APINATA_UNSET_TOKEN_VAR"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
}

#[cfg(unix)]
#[test]
fn a_non_utf8_value_is_a_usage_error_without_the_value() {
    use std::os::unix::ffi::OsStrExt;
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let out = apinata()
        .env("TOKEN", std::ffi::OsStr::from_bytes(b"bad\xff\xfeval"))
        .arg("run")
        .arg(dir.path())
        .args(["--env-var", "TOKEN"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
    assert!(
        text(&out).contains("environment variable TOKEN"),
        "{}",
        text(&out)
    );
    assert!(!text(&out).contains("bad"), "{}", text(&out));
}

#[test]
fn an_empty_key_is_still_rejected() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--env-var", "=v"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3), "{}", text(&out));
}

#[test]
fn check_and_mock_accept_the_bare_form() {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO)]);
    let out = apinata()
        .env("TOKEN", SECRET)
        .arg("check")
        .arg(dir.path())
        .args(["--no-keychain", "--env-var", "TOKEN"])
        .output()
        .expect("runs");
    assert_ne!(out.status.code(), Some(3), "{}", text(&out));
    assert!(!text(&out).contains(SECRET), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undeclared_value_from_the_environment_never_reaches_output() {
    // The server echoes the token in the body, so the response view must redact it too.
    let s = MockServer::start().await;
    Mock::given(path("/who"))
        .respond_with(|r: &wiremock::Request| {
            let key = r.url.query().unwrap_or_default().to_owned();
            ResponseTemplate::new(200).set_body_string(key)
        })
        .mount(&s)
        .await;
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO_G)]);
    let report = dir.path().join("out.json");
    let base = format!("base={}", s.uri());
    for format in ["json", "text"] {
        let out = apinata()
            .env("TOKEN", SECRET)
            .arg("run")
            .arg(dir.path())
            .args([
                "--no-keychain",
                "--verbose",
                "--show-response",
                "--format",
                format,
            ])
            .args(["--env-var", &base, "--env-var", "TOKEN"])
            .arg("--reporter-json")
            .arg(&report)
            .output()
            .expect("runs");
        assert_eq!(out.status.code(), Some(0), "{}", text(&out));
        assert!(!text(&out).contains(SECRET), "{format}: {}", text(&out));
        let file = std::fs::read_to_string(&report).expect("report file");
        assert!(!file.contains(SECRET), "{format}: reporter file leaks");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_explicit_value_for_an_undeclared_key_is_unchanged_in_output() {
    // Only environment-sourced values gain redaction; KEY=VALUE keeps today's behaviour.
    let s = server("plain-visible").await;
    let dir = write_tree(&[("opencollection.yml", ROOT), ("who.yml", WHO_G)]);
    let base = format!("base={}", s.uri());
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args([
            "--no-keychain",
            "--env-var",
            &base,
            "--env-var",
            "TOKEN=plain-visible",
        ])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
}
