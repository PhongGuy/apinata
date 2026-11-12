//! Outcomes and exit codes 0, 1 and 2, the highest winning, against a mock server.

use crate::common;

use common::{ROOT, apinata, check, column, get, run_json, write_tree};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":7}"))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn passing_and_skipped_assertions_exit_0() {
    let s = server().await;
    let ok = format!("{}/ok", s.uri());
    let asserts = check("res.status", "eq", "200") + &check("res.url", "isDefined", "");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &get("A", 1, &ok, &asserts)),
    ]);
    // Scripts off, so the JavaScript left side `res.url` is skipped.
    let (code, report) = run_json(dir.path(), &["--no-scripts"]);
    assert_eq!(code, 0);
    assert_eq!(column(&report, "outcome"), ["passed"]);
    assert_eq!(report["results"][0]["status"], 200);
    assert_eq!(report["summary"]["skipped_assertions"], 1);
    assert_eq!(report["summary"]["unverified"], 0);
    assert_eq!(report["summary"]["no_tests"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_whose_only_check_was_skipped_is_unverified_and_still_exits_0() {
    let s = server().await;
    let ok = format!("{}/ok", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("A", 1, &ok, &check("res.url", "isDefined", "")),
        ),
        ("b.yml", &get("B", 2, &ok, "    []\n")),
    ]);
    let (code, report) = run_json(dir.path(), &["--no-scripts"]);
    assert_eq!(code, 0);
    assert_eq!(report["summary"]["passed"], 2);
    assert_eq!(report["summary"]["unverified"], 1);
    assert_eq!(report["summary"]["no_tests"], 1);
    assert_eq!(report["summary"]["skipped"], 0);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--no-scripts")
        .assert()
        .code(0);
    let text = String::from_utf8(out.get_output().stdout.clone()).expect("utf-8");
    assert!(
        text.contains(
            "Requests: 2 (1 passed (1 unverified), 0 failed, 1 without tests, 0 errored, 0 not sent)"
        ),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_assertion_exits_1() {
    let s = server().await;
    let ok = format!("{}/ok", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("A", 1, &ok, &check("res.status", "eq", "200")),
        ),
        ("b.yml", &get("B", 2, &ok, &check("res.body.id", "eq", "8"))),
    ]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 1);
    assert_eq!(column(&report, "outcome"), ["passed", "failed"]);
    let row = &report["results"][1]["assertions"][0];
    assert_eq!(
        (row["verdict"].as_str(), row["actual"].as_str()),
        (Some("fail"), Some("7"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsent_requests_exit_2_and_load_errors_exit_3_over_failures() {
    let s = server().await;
    let ok = format!("{}/ok", s.uri());
    let failing = get("B", 2, &ok, &check("res.status", "eq", "500"));
    let undefined = get("C", 3, "{{nowhere}}/x", "    []\n");
    let refused = get("D", 4, "http://127.0.0.1:1/", "    []\n");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("b.yml", &failing),
        ("c.yml", &undefined),
        ("d.yml", &refused),
    ]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 2);
    assert_eq!(column(&report, "outcome"), ["failed", "errored", "errored"]);
    assert_eq!(
        column(&report, "error")[1],
        "variable `nowhere` is not defined"
    );
    assert!(column(&report, "error")[2].contains("127.0.0.1:1"));
    assert_eq!(report["results"][1]["status"], serde_json::Value::Null);

    let broken = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("A", 1, &ok, &check("res.status", "eq", "200")),
        ),
        ("broken.yml", "info: [unclosed\n"),
    ]);
    let (code, report) = run_json(broken.path(), &[]);
    assert_eq!(code, 3);
    assert_eq!(column(&report, "outcome"), ["passed"]);
    assert_eq!(report["load_errors"][0]["path"], "broken.yml");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn environment_warnings_go_to_stderr_and_keep_json_clean() {
    let s = server().await;
    let ok = format!("{}/ok", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nextends: nope\nvariables: []\n",
        ),
        (
            "a.yml",
            &get("A", 1, &ok, &check("res.status", "eq", "200")),
        ),
    ]);
    let out = common::apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--env", "dev"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning: environment `dev` extends `nope`, which does not exist"),
        "{stderr}"
    );
    serde_json::from_slice::<serde_json::Value>(&out.stdout).expect("stdout is only the report");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_with_a_custom_method_sends_that_verb() {
    let s = MockServer::start().await;
    Mock::given(wiremock::matchers::method("PROPFIND"))
        .and(path("/dav"))
        .respond_with(ResponseTemplate::new(207))
        .expect(1)
        .mount(&s)
        .await;
    let text = format!(
        "info:\n  name: Dav\n  type: http\n  seq: 1\nhttp:\n  method: PROPFIND\n  url: \"{}/dav\"\n",
        s.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("dav.yml", &text)]);
    let (code, report) = run_json(dir.path(), &[]);
    assert_eq!(code, 0, "{report:#}");
    assert_eq!(column(&report, "outcome"), ["passed"]);
}
