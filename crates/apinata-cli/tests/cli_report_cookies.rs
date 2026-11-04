//! The JSON report lists each Set-Cookie verdict of a request, by name and never by value.

use crate::cli_verbose_cookies::{SCRIPT_VALUES, VALUES, chain_dir, pre_script_runtime};
use crate::common;

use common::{ROOT, apinata, run_json, write_tree};
use serde_json::{Value, json};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The rows the scripted chain on `port` reports: the pre-request script's, then each hop's.
fn expected(port: u16) -> Value {
    let start = format!("app.test:{port}/start");
    let next = format!("app.test:{port}/next");
    json!([
        {"script": true, "phase": "pre-request", "name": "pre", "stored": true},
        {"script": true, "phase": "pre-request", "name": "evil", "domain": "evil.example", "stored": false,
         "reason": "Domain does not match host"},
        {"hop": start, "name": "kept", "path": "/", "stored": true},
        {"hop": start, "name": "sec", "path": "/", "stored": false,
         "reason": "Secure cookie over http"},
        {"hop": next, "name": "next", "path": "/", "stored": true},
    ])
}

/// Asserts no cookie value of the chain or its script appears in `text`.
fn no_value_in(text: &str) {
    for value in VALUES.iter().chain(&SCRIPT_VALUES) {
        assert!(!text.contains(value), "{value} leaked: {text}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn format_json_lists_script_and_hop_cookies_with_their_verdicts() {
    let (_mock, dir, port) = chain_dir(&pre_script_runtime()).await;
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["results"][0]["cookies"], expected(port), "{report}");
    no_value_in(&report.to_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reporter_json_file_lists_the_same_cookies() {
    let (_mock, dir, port) = chain_dir(&pre_script_runtime()).await;
    let out_dir = tempfile::TempDir::new().expect("tempdir");
    let file = out_dir.path().join("report.json");
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--reporter-json")
        .arg(&file)
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = std::fs::read_to_string(&file).expect("report file");
    let report: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(report["results"][0]["cookies"], expected(port), "{report}");
    no_value_in(&text);
    no_value_in(&String::from_utf8_lossy(&out.stdout));
    no_value_in(&String::from_utf8_lossy(&out.stderr));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_without_cookies_has_no_cookies_key() {
    let mock = MockServer::start().await;
    Mock::given(path("/plain"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/plain\"\n",
        mock.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["results"][0].get("cookies"), None, "{report}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_cookie_name_is_redacted_and_an_unparsed_one_has_no_name() {
    let mock = MockServer::start().await;
    Mock::given(path("/one"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "SECRET-Name7=SecretVal-Q6x; Path=/")
                .append_header("set-cookie", "NamelessVal-Z9; Path=/"),
        )
        .mount(&mock)
        .await;
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/one?k={{{{key}}}}\"\n",
        mock.uri()
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: key\n",
        ),
        ("r.yml", &request),
    ]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--env", "dev", "--env-var"])
        .arg("key=SECRET-Name7")
        .output()
        .expect("runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let report: Value = serde_json::from_str(&stdout).expect("json");
    let rows = &report["results"][0]["cookies"];
    assert_eq!(rows[0]["name"], "[redacted]", "{report}");
    assert_eq!(rows[1].get("name"), None, "{report}");
    assert_eq!(rows[1]["reason"], "Could not be parsed", "{report}");
    for leak in ["SECRET-Name7", "SecretVal-Q6x", "NamelessVal-Z9"] {
        assert!(!stdout.contains(leak), "{leak} leaked: {stdout}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_post_response_script_cookie_follows_the_hop_rows() {
    let code = "await bru.cookies.add({ key: \"post\", value: \"PostVal-Q8x\" });";
    let runtime = format!(
        "runtime:\n  scripts:\n    - type: after-response\n      code: |-\n        {code}\n"
    );
    let (_mock, dir, port) = chain_dir(&runtime).await;
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report}");
    let rows = report["results"][0]["cookies"].as_array().expect("cookies");
    let from: Vec<String> = rows
        .iter()
        .map(|r| match r.get("hop") {
            Some(hop) => format!("{}:{}", hop.as_str().expect("hop"), r["name"]),
            None => format!("script:{}", r["name"]),
        })
        .collect();
    assert_eq!(
        from,
        [
            format!("app.test:{port}/start:\"kept\""),
            format!("app.test:{port}/start:\"sec\""),
            format!("app.test:{port}/next:\"next\""),
            "script:\"post\"".to_owned(),
        ],
        "{report}"
    );
    assert_eq!(rows[3]["script"], true, "{report}");
    assert_eq!(rows[3]["phase"], "post-response", "{report}");
    assert!(!report.to_string().contains("PostVal-Q8x"), "{report}");
}

#[test]
fn an_errored_result_keeps_its_pre_request_script_cookie_row() {
    let dir = crate::cli_verbose_cookies::failed_dir();
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 2, "{report}");
    assert_eq!(
        report["results"][0]["cookies"],
        json!([{"script": true, "phase": "pre-request", "name": "pre", "stored": true}]),
        "{report}"
    );
    assert!(
        !report
            .to_string()
            .contains(crate::cli_verbose_cookies::FAILED_VALUE),
        "{report}"
    );
}

/// The HTML and JUnit reports of one run of `dir`, written to files, and the run's output.
fn html_and_junit(dir: &std::path::Path) -> (String, String, std::process::Output) {
    let out_dir = tempfile::TempDir::new().expect("tempdir");
    let (html, junit) = (out_dir.path().join("r.html"), out_dir.path().join("r.xml"));
    let out = apinata()
        .arg("run")
        .arg(dir)
        .arg("--reporter-html")
        .arg(&html)
        .arg("--reporter-junit")
        .arg(&junit)
        .output()
        .expect("runs");
    let read = |p: &std::path::Path| std::fs::read_to_string(p).expect("report file");
    (read(&html), read(&junit), out)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_html_report_lists_each_cookie_by_origin_name_and_verdict() {
    let (_mock, dir, port) = chain_dir(&pre_script_runtime()).await;
    let (html, _, out) = html_and_junit(dir.path());
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(html.contains("<h3>Cookies</h3>"), "{html}");
    let start = format!("app.test:{port}/start");
    for (from, name, verdict) in [
        ("pre-request script", "pre", "Stored"),
        ("pre-request script", "evil", "Domain does not match host"),
        (start.as_str(), "kept", "Stored"),
        (start.as_str(), "sec", "Secure cookie over http"),
    ] {
        let row = format!("<tr><td>{from}</td><td>{name}</td><td>{verdict}</td></tr>");
        assert!(html.contains(&row), "{row} missing: {html}");
    }
    no_value_in(&html);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_junit_report_notes_each_cookie_line_in_system_out() {
    let (_mock, dir, port) = chain_dir(&pre_script_runtime()).await;
    let (_, junit, out) = html_and_junit(dir.path());
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let note = format!(
        "<system-out>set-cookie pre: Stored (pre-request script)\n\
         set-cookie evil: Domain does not match host (pre-request script)\n\
         set-cookie kept: Stored (app.test:{port}/start)\n\
         set-cookie sec: Secure cookie over http (app.test:{port}/start)\n\
         set-cookie next: Stored (app.test:{port}/next)</system-out>"
    );
    assert!(junit.contains(&note), "{junit}");
    no_value_in(&junit);
}

#[test]
fn an_errored_request_notes_its_script_cookie_in_both_reports() {
    let dir = crate::cli_verbose_cookies::failed_dir();
    let (html, junit, _) = html_and_junit(dir.path());
    let row = "<tr><td>pre-request script</td><td>pre</td><td>Stored</td></tr>";
    assert!(html.contains(row), "{html}");
    assert!(
        junit.contains("set-cookie pre: Stored (pre-request script)</system-out>"),
        "{junit}"
    );
    for text in [&html, &junit] {
        assert!(
            !text.contains(crate::cli_verbose_cookies::FAILED_VALUE),
            "{text}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_without_cookies_has_no_cookie_section_in_either_report() {
    let mock = MockServer::start().await;
    Mock::given(path("/plain"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/plain\"\n",
        mock.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &request)]);
    let (html, junit, _) = html_and_junit(dir.path());
    assert!(!html.contains("<h3>Cookies</h3>"), "{html}");
    assert!(!junit.contains("set-cookie"), "{junit}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_deleted_cookie_never_shows_its_value() {
    let code = "await bru.cookies.add({ key: \"gone\", value: \"GoneVal-Q7z\" }); await bru.cookies.remove(\"gone\");";
    let runtime = format!(
        "runtime:\n  scripts:\n    - type: after-response\n      code: |-\n        {code}\n"
    );
    let (_mock, dir, _) = chain_dir(&runtime).await;
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--verbose")
        .output()
        .expect("runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("  set-cookie gone: Removed (post-response script)"),
        "{stderr}"
    );
    assert!(!stderr.contains("GoneVal-Q7z"), "{stderr}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("GoneVal-Q7z"));
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report}");
    let rows = report["results"][0]["cookies"].as_array().expect("cookies");
    assert_eq!(rows.last().expect("a row")["removed"], true, "{report}");
    assert!(!report.to_string().contains("GoneVal-Q7z"), "{report}");
}
