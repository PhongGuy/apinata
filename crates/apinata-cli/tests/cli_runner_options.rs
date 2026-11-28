//! `--delay`, `--tags`, `--exclude-tags`, `--tests-only`, `--parallel` and the data-file flags.

use crate::common;

use common::{ROOT, apinata, column, run_json, write_tree};
use std::time::{Duration, Instant};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn ok_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn tagged(name: &str, seq: u32, url: &str, tags: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\n{tags}http:\n  method: GET\n  url: \"{url}\"\n"
    )
}

async fn served(s: &MockServer) -> Vec<String> {
    let got = s.received_requests().await.expect("recorded");
    got.iter().map(|r| r.url.path().to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tags_and_exclude_tags_filter_by_effective_tags() {
    let s = ok_server().await;
    let u = s.uri();
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "api/folder.yml",
            "info:\n  name: api\n  type: folder\n  tags:\n    - api\n",
        ),
        (
            "api/a.yml",
            &tagged("A", 1, &format!("{u}/a"), "  tags:\n    - smoke\n"),
        ),
        ("api/b.yml", &tagged("B", 2, &format!("{u}/b"), "")),
        (
            "c.yml",
            &tagged(
                "C",
                1,
                &format!("{u}/c"),
                "  tags:\n    - smoke\n    - slow\n",
            ),
        ),
    ]);
    let (code, report) = run_json(
        dir.path(),
        &["--tags", "smoke, api", "--exclude-tags", "slow"],
    );
    assert_eq!(code, 0);
    assert_eq!(column(&report, "path"), ["api/a.yml", "api/b.yml"]);
    assert!(report["results"][0].get("iteration_index").is_none());
    // A filter that matches nothing is a usage error (see cli_exit_codes).
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--tags", "nothing-has-this"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3));
    assert!(served(&s).await.len() == 2, "nothing more was sent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn csv_rows_iterate_and_results_carry_their_iteration_index() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &tagged("A", 1, &format!("{}/u/{{{{id}}}}", s.uri()), ""),
        ),
    ]);
    let data = write_tree(&[("rows.csv", "id\n7\n8\n")]);
    let path = data.path().join("rows.csv");
    let (code, report) = run_json(
        dir.path(),
        &["--csv-file-path", path.to_str().expect("utf-8")],
    );
    assert_eq!(code, 0);
    assert_eq!(served(&s).await, ["/u/7", "/u/8"]);
    assert_eq!(report["results"][0]["iteration_index"], 0);
    assert_eq!(report["results"][1]["iteration_index"], 1);
    assert_eq!(report["summary"]["requests"], 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_relative_json_file_path_resolves_from_the_working_directory() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &tagged("A", 1, &format!("{}/u/{{{{id}}}}", s.uri()), ""),
        ),
    ]);
    let data = write_tree(&[("rows.json", "[{\"id\": 1}, {\"id\": \"two\"}]")]);
    let out = apinata()
        .current_dir(data.path())
        .arg("run")
        .arg(dir.path())
        .args(["--format", "json", "--json-file-path", "rows.json"])
        .output()
        .expect("runs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(served(&s).await, ["/u/1", "/u/two"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_or_missing_data_file_exits_3_before_sending() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &tagged("A", 1, &format!("{}/a", s.uri()), "")),
    ]);
    let data = write_tree(&[("bad.csv", "a,b\n1\n")]);
    for file in [data.path().join("bad.csv"), data.path().join("missing.csv")] {
        let out = apinata()
            .arg("run")
            .arg(dir.path())
            .arg("--csv-file-path")
            .arg(&file)
            .output()
            .expect("runs");
        assert_eq!(out.status.code(), Some(3));
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(err.contains("cannot read data file"), "{err}");
    }
    assert!(served(&s).await.is_empty());
}

#[test]
fn the_two_data_flags_conflict() {
    let out = apinata()
        .args([
            "run",
            ".",
            "--csv-file-path",
            "a.csv",
            "--json-file-path",
            "a.json",
        ])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("cannot be used with"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delay_spaces_the_requests() {
    let s = ok_server().await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &tagged("A", 1, &format!("{}/a", s.uri()), "")),
        ("b.yml", &tagged("B", 2, &format!("{}/b", s.uri()), "")),
    ]);
    let t = Instant::now();
    let (code, _) = run_json(dir.path(), &["--delay", "400"]);
    assert_eq!(code, 0);
    assert!(t.elapsed() >= Duration::from_millis(400));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tests_only_sends_only_requests_with_checks_and_counts_the_rest() {
    let s = ok_server().await;
    let u = s.uri();
    let checked = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &format!("{}{checked}", tagged("A", 1, &format!("{u}/a"), "")),
        ),
        ("b.yml", &tagged("B", 2, &format!("{u}/b"), "")),
        ("c.yml", &tagged("C", 3, &format!("{u}/c"), "")),
    ]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--tests-only", "--format", "json"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(column(&report, "path"), ["a.yml"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Skipped 2 requests without tests"),
        "stderr: {err}"
    );
    assert_eq!(served(&s).await, ["/a"]);
    // A target with no tests at all is a usage error, as an empty tag filter is.
    let out = apinata()
        .arg("run")
        .arg(dir.path().join("b.yml"))
        .arg("--tests-only")
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("no requests with tests to run (1 skipped)"),
        "stderr: {err}"
    );
    assert_eq!(served(&s).await.len(), 1, "nothing more was sent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_sends_every_request_and_reports_in_plan_order() {
    let s = ok_server().await;
    let u = s.uri();
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &tagged("A", 1, &format!("{u}/a"), "")),
        ("b.yml", &tagged("B", 2, &format!("{u}/b"), "")),
        ("c.yml", &tagged("C", 3, &format!("{u}/c"), "")),
    ]);
    let (code, report) = run_json(dir.path(), &["--parallel", "3"]);
    assert_eq!(code, 0);
    assert_eq!(column(&report, "path"), ["a.yml", "b.yml", "c.yml"]);
    let mut got = served(&s).await;
    got.sort();
    assert_eq!(got, ["/a", "/b", "/c"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_refuses_a_request_that_writes_shared_variables() {
    let s = ok_server().await;
    let setter = format!(
        "{}runtime:\n  scripts:\n    - type: tests\n      code: |-\n        bru.setVar(\"t\", 1);\n",
        tagged("B", 2, &format!("{}/b", s.uri()), "")
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &tagged("A", 1, &format!("{}/a", s.uri()), "")),
        ("b.yml", &setter),
    ]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--parallel", "2"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("b.yml cannot run in parallel: its tests script calls setVar"),
        "stderr: {err}"
    );
    assert!(served(&s).await.is_empty());
    // With scripts off the call never runs, so the run may go ahead.
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--parallel", "2", "--no-scripts"])
        .output()
        .expect("runs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn parallel_takes_one_to_sixty_four_and_conflicts_with_delay() {
    for n in ["0", "65"] {
        let out = apinata()
            .args(["run", ".", "--parallel", n])
            .output()
            .expect("runs");
        assert_eq!(out.status.code(), Some(3), "{n}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("--parallel"), "{n}: {err}");
    }
    let out = apinata()
        .args(["run", ".", "--parallel", "2", "--delay", "10"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("cannot be used with"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_verbose_lines_follow_run_order() {
    use wiremock::matchers::path;
    let s = MockServer::start().await;
    // The first request answers last, so completion order is the reverse of run order.
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(300)))
        .mount(&s)
        .await;
    Mock::given(path("/fast"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let u = s.uri();
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &tagged("A", 1, &format!("{u}/slow"), "")),
        ("b.yml", &tagged("B", 2, &format!("{u}/fast"), "")),
    ]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--parallel", "2", "--verbose"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
    let err = String::from_utf8_lossy(&out.stderr);
    let at = |needle: &str| {
        err.lines()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no {needle} line: {err}"))
    };
    assert!(at("/slow 200") < at("/fast 200"), "{err}");
}
