//! `--delay`, `--tags`, `--exclude-tags` and the data-file flags.

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
