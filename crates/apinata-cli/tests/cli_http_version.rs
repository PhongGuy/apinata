//! `--http-version`: parsing, its reach into `run`, and the verbose line's version suffix.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn request(url: &str) -> String {
    format!("info:\n  name: A\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n")
}

/// The `--verbose` lines of one `run` of a collection whose one request GETs `url`.
fn verbose_lines(url: &str, extra: &[&str]) -> Vec<String> {
    let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request(url))]);
    let profile = tempfile::TempDir::new().expect("tempdir");
    let out = apinata()
        .env("APINATA_PROFILE_DIR", profile.path())
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--verbose"])
        .args(extra)
        .output()
        .expect("runs");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{stderr}");
    stderr
        .lines()
        .filter(|l| l.starts_with('['))
        .map(str::to_owned)
        .collect()
}

async fn ok_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prior_knowledge_marks_the_verbose_line_with_http2() {
    let s = ok_server().await;
    let url = format!("{}/ok", s.uri());
    let lines = verbose_lines(&url, &["--http-version", "2-prior-knowledge"]);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].ends_with("ms HTTP/2"), "{}", lines[0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http1_exchanges_keep_the_old_line() {
    let s = ok_server().await;
    let url = format!("{}/ok", s.uri());
    for extra in [
        &[][..],
        &["--http-version", "1.1"][..],
        &["--http-version", "auto"][..],
    ] {
        let lines = verbose_lines(&url, extra);
        assert_eq!(lines.len(), 1, "{extra:?}: {lines:?}");
        assert!(lines[0].ends_with("ms"), "{extra:?}: {}", lines[0]);
        assert!(!lines[0].contains("HTTP/"), "{extra:?}: {}", lines[0]);
    }
}

#[test]
fn an_unknown_version_is_a_usage_error_listing_the_three() {
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--http-version", "3"])
        .assert()
        .code(3)
        .stderr(contains("possible values: 1.1, auto, 2-prior-knowledge"));
}

#[test]
fn run_and_mcp_both_take_the_flag() {
    for sub in ["run", "mcp"] {
        apinata()
            .args([sub, "--help"])
            .assert()
            .code(0)
            .stdout(contains("--http-version"));
    }
}
