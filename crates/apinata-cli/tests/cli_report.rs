//! The JSON report's stable shape (an insta snapshot) and the text rendering.

use crate::common;

use common::{ROOT, apinata, check, get, run_json, unary, write_tree};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A collection with a passing request (a warning and a skipped assertion), a failing one,
/// one that cannot be resolved and a file that does not parse.
async fn mixed() -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":7}"))
        .mount(&s)
        .await;
    let ok = format!("{}/ok", s.uri());
    let passing = get(
        "Pass",
        1,
        &ok,
        &(check("res.status", "eq", "200") + &unary("res.url", "isDefined")),
    ) + "  scripts:\n    - type: tests\n      code: x\n";
    let failing = get(
        "Fail",
        2,
        &ok,
        &(check("res.body.id", "eq", "8") + &check("res.body.id", "eq", "{{missing}}")),
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("pass.yml", &passing),
        ("fail.yml", &failing),
        ("err.yml", &get("Err", 3, "{{nowhere}}/x", "    []\n")),
        ("broken.yml", "info: [unclosed\n"),
    ]);
    (s, dir)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_report_shape_is_stable() {
    let (_s, dir) = mixed().await;
    let (code, mut report) = run_json(dir.path(), &["--no-scripts"]);
    assert_eq!(code, 3);
    for result in report["results"].as_array_mut().expect("results") {
        if result["duration_ms"].is_u64() {
            result["duration_ms"] = 0.into();
        }
    }
    insta::assert_json_snapshot!(report, @r#"
    {
      "apinata": 1,
      "load_errors": [
        {
          "path": "broken.yml",
          "message": "did not find expected ',' or ']' at line 2 column 1, while parsing a flow sequence at line 1 column 7"
        }
      ],
      "results": [
        {
          "path": "pass.yml",
          "name": "Pass",
          "outcome": "passed",
          "status": 200,
          "duration_ms": 0,
          "error": null,
          "warnings": [
            "scripts not executed"
          ],
          "assertions": [
            {
              "expression": "res.status",
              "operator": "eq",
              "value": "200",
              "verdict": "pass",
              "detail": null
            },
            {
              "expression": "res.url",
              "operator": "isDefined",
              "value": null,
              "verdict": "skipped",
              "detail": "left side `res.url` needs a JavaScript runtime"
            }
          ],
          "post_response_vars": [],
          "tests": [
            {
              "phase": "tests",
              "name": "this request's tests",
              "status": "skipped",
              "message": "scripts are off"
            }
          ],
          "console": []
        },
        {
          "path": "fail.yml",
          "name": "Fail",
          "outcome": "failed",
          "status": 200,
          "duration_ms": 0,
          "error": null,
          "warnings": [],
          "assertions": [
            {
              "expression": "res.body.id",
              "operator": "eq",
              "value": "8",
              "verdict": "fail",
              "actual": "7",
              "detail": null
            },
            {
              "expression": "res.body.id",
              "operator": "eq",
              "value": "{{missing}}",
              "verdict": "fail",
              "actual": "7",
              "detail": "variable `{{missing}}` is undefined"
            }
          ],
          "post_response_vars": [],
          "tests": [],
          "console": []
        },
        {
          "path": "err.yml",
          "name": "Err",
          "outcome": "errored",
          "status": null,
          "duration_ms": null,
          "error": "variable `nowhere` is not defined",
          "warnings": [],
          "assertions": [],
          "post_response_vars": [],
          "tests": [],
          "console": []
        }
      ],
      "summary": {
        "requests": 3,
        "passed": 1,
        "unverified": 0,
        "no_tests": 0,
        "failed": 1,
        "errored": 1,
        "skipped": 0,
        "skipped_assertions": 1,
        "passed_tests": 0,
        "failed_tests": 0,
        "skipped_tests": 1
      }
    }
    "#);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_output_lists_requests_assertions_and_totals() {
    let (_s, dir) = mixed().await;
    let assert = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--no-scripts")
        .assert()
        .code(3);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    for line in [
        "broken.yml (load error: did not find expected",
        "pass (200 OK) - ",
        "   ! scripts not executed",
        "   ✓ res.status: eq 200",
        "   - res.url: isDefined (skipped: left side `res.url` needs a JavaScript runtime)",
        "   ✕ res.body.id: eq 8 (actual 7)",
        "   ✕ res.body.id: eq {{missing}} (actual 7; variable `{{missing}}` is undefined)",
        "err (variable `nowhere` is not defined)",
        "Requests: 3 (1 passed, 1 failed, 1 errored, 0 not sent)",
        "Assertions: 1 passed, 2 failed, 1 skipped",
    ] {
        assert!(stdout.contains(line), "missing {line:?} in:\n{stdout}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_supplied_by_env_var_never_reaches_any_output() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let env = "name: dev\nvariables:\n  - secret: true\n    name: key\n";
    // The secret rides in the query, a header and an assertion's expected value.
    let with_key = |name: &str, seq: u32, url: &str| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}?key={{{{key}}}}\"\n  headers:\n    - name: X-Key\n      value: \"{{{{key}}}}\"\nruntime:\n  assertions:\n{}",
            check("res.status", "eq", "{{key}}")
        )
    };
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", env),
        ("fail.yml", &with_key("Fail", 1, &format!("{}/ok", s.uri()))),
        ("err.yml", &with_key("Err", 2, "http://127.0.0.1:1/refused")),
    ]);
    for format in ["json", "text"] {
        let out = apinata()
            .arg("run")
            .arg(dir.path())
            .args(["--env", "dev", "--env-var", "key=SUPERSECRET"])
            .args(["--format", format])
            .output()
            .expect("runs");
        assert_eq!(out.status.code(), Some(2));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stdout.contains("fail"), "{stdout}");
        assert!(!stdout.contains("SUPERSECRET"), "{format} stdout: {stdout}");
        assert!(!stderr.contains("SUPERSECRET"), "{format} stderr: {stderr}");
    }
    // The secret was really sent, so its absence above is redaction and not an unused variable.
    let sent = s.received_requests().await.expect("recorded");
    assert!(sent.iter().all(|r| r.headers["x-key"] == "SUPERSECRET"));
    assert_eq!(sent.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_status_without_a_reason_phrase_has_no_stray_space() {
    let s = MockServer::start().await;
    Mock::given(path("/odd"))
        .respond_with(ResponseTemplate::new(599))
        .mount(&s)
        .await;
    let url = format!("{}/odd", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "odd.yml",
            &get("Odd", 1, &url, &check("res.status", "eq", "599")),
        ),
    ]);
    apinata()
        .arg("run")
        .arg(dir.path())
        .assert()
        .code(0)
        .stdout(predicates::str::contains("odd (599) - "));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn path_params_dropping_url_credentials_warn_without_showing_them() {
    let s = MockServer::start().await;
    Mock::given(path("/users/7"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let url = s.uri().replacen("http://", "http://acct-7781:pw-55xq@", 1);
    let file = format!(
        "info:\n  name: User\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}/users/:id\"\n  params:\n    - name: id\n      value: \"7\"\n      type: path\n"
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("user.yml", &file)]);
    for extra in [&[][..], &["--verbose"][..]] {
        let out = apinata()
            .arg("run")
            .arg(dir.path())
            .args(extra)
            .output()
            .expect("runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stdout.contains(
                "   ! credentials in the URL were not sent because the request has path params"
            ),
            "{extra:?}: {stdout}"
        );
        for secret in ["acct-7781", "pw-55xq"] {
            assert!(!stdout.contains(secret), "{extra:?} stdout: {stdout}");
            assert!(!stderr.contains(secret), "{extra:?} stderr: {stderr}");
        }
    }
}
