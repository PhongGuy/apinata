//! Report files written by `apinata run --reporter-*`.

use crate::common;

use common::{ROOT, apinata, check, get, write_tree};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A collection whose first request passes and whose second fails; `/ok` must be hit
/// `hits` times.
async fn pass_then_fail(hits: u64) -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":7}"))
        .expect(hits)
        .mount(&s)
        .await;
    let ok = format!("{}/ok", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "pass.yml",
            &get("Pass", 1, &ok, &check("res.status", "eq", "200")),
        ),
        (
            "fail.yml",
            &get("Fail", 2, &ok, &check("res.body.id", "eq", "8")),
        ),
        (
            "last.yml",
            &get("Last", 3, &ok, &check("res.status", "eq", "200")),
        ),
    ]);
    (s, dir)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_report_file_matches_stdout() {
    let (_s, dir) = pass_then_fail(3).await;
    let out_dir = tempfile::TempDir::new().unwrap();
    let file = out_dir.path().join("report.json");
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--reporter-json"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(std::fs::read(&file).unwrap(), out.stdout);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_output_directory_exits_3_before_sending() {
    let (s, dir) = pass_then_fail(0).await;
    let out_dir = tempfile::TempDir::new().unwrap();
    let file = out_dir.path().join("nope").join("report.xml");
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--reporter-junit")
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("output directory"), "{stderr}");
    assert!(stderr.contains("does not exist"), "{stderr}");
    assert!(!file.exists());
    s.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unwritable_report_path_exits_3() {
    let (_s, dir) = pass_then_fail(3).await;
    let out_dir = tempfile::TempDir::new().unwrap();
    // The path is itself a directory, so its parent exists but the write fails.
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--reporter-json")
        .arg(out_dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot write report"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn junit_report_written_after_bail() {
    let (_s, dir) = pass_then_fail(2).await;
    let out_dir = tempfile::TempDir::new().unwrap();
    let file = out_dir.path().join("junit.xml");
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--bail", "--reporter-junit"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let xml = std::fs::read_to_string(&file).unwrap();
    assert!(
        xml.starts_with(
            "<?xml version=\"1.0\"?>\n<testsuites>\n  <testsuite name=\"Pass\" file=\"pass.yml\""
        ),
        "{xml}"
    );
    assert!(xml.contains(r#"<failure type="failure""#), "{xml}");
    assert!(
        xml.contains(r#"<skipped message="Request skipped due to bail"/>"#),
        "{xml}"
    );
    assert_eq!(xml.matches("<testsuite ").count(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_and_junit_reports_combine() {
    let (_s, dir) = pass_then_fail(3).await;
    let out_dir = tempfile::TempDir::new().unwrap();
    let json = out_dir.path().join("r.json");
    let xml = out_dir.path().join("r.xml");
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--reporter-json")
        .arg(&json)
        .arg("--reporter-junit")
        .arg(&xml)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(report["summary"]["requests"], 3);
    let text = std::fs::read_to_string(&xml).unwrap();
    assert!(text.contains("hostname=\""), "{text}");
    assert!(!text.contains("hostname=\"\""), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_three_reports_combine() {
    let (_s, dir) = pass_then_fail(3).await;
    let out_dir = tempfile::TempDir::new().unwrap();
    let [json, xml, html] = ["r.json", "r.xml", "r.html"].map(|f| out_dir.path().join(f));
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--reporter-json")
        .arg(&json)
        .arg("--reporter-junit")
        .arg(&xml)
        .arg("--reporter-html")
        .arg(&html)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(json.is_file() && xml.is_file());
    let page = std::fs::read_to_string(&html).unwrap();
    assert!(page.starts_with("<!doctype html>"));
    assert!(page.contains("Fail") && page.contains("fail.yml"));
}
