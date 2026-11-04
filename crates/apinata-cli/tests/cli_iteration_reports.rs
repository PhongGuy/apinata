//! The text, JUnit and HTML reports label each iteration of a data-file run.

use crate::common;

use common::{ROOT, apinata, check, get, write_tree};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_report_labels_the_iterations() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
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
    let out_dir = write_tree(&[("rows.csv", "id\n1\n2\n")]);
    let (junit, html) = (out_dir.path().join("r.xml"), out_dir.path().join("r.html"));
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--csv-file-path")
        .arg(out_dir.path().join("rows.csv"))
        .arg("--reporter-junit")
        .arg(&junit)
        .arg("--reporter-html")
        .arg(&html)
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let lines: Vec<&str> = text.lines().collect();
    let at = |want: &str| {
        lines
            .iter()
            .position(|l| *l == want)
            .unwrap_or_else(|| panic!("{want}: {text}"))
    };
    let headlines: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].starts_with("a ("))
        .collect();
    assert_eq!(headlines.len(), 2, "{text}");
    assert!(at("Iteration 1") < headlines[0] && headlines[0] < at("Iteration 2"));
    assert!(at("Iteration 2") < headlines[1]);
    let xml = std::fs::read_to_string(&junit).expect("junit");
    assert!(xml.contains("name=\"A (iteration 1)\""), "{xml}");
    assert!(xml.contains("name=\"A (iteration 2)\""), "{xml}");
    let page = std::fs::read_to_string(&html).expect("html");
    let (one, two) = (
        page.find("<h3>Iteration 1</h3>"),
        page.find("<h3>Iteration 2</h3>"),
    );
    assert!(one.is_some() && one < two, "{page}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_without_data_has_no_iteration_labels() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
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
    let out = apinata().arg("run").arg(dir.path()).output().expect("runs");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Iteration"));
}

async fn ok_collection() -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get(
                "A",
                1,
                &format!("{}/n/{{{{iteration}}}}", s.uri()),
                &check("res.status", "eq", "200"),
            ),
        ),
    ]);
    (s, dir)
}

async fn paths(s: &MockServer) -> Vec<String> {
    let got = s.received_requests().await.expect("recorded");
    got.iter().map(|r| r.url.path().to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iterations_repeats_the_run_and_labels_each_pass() {
    let (s, dir) = ok_collection().await;
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .args(["--iterations", "3"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("Iteration 3"), "{text}");
    assert_eq!(paths(&s).await, ["/n/0", "/n/1", "/n/2"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iterations_caps_the_rows_of_a_data_file() {
    let (s, dir) = ok_collection().await;
    let data = write_tree(&[("rows.csv", "id\n1\n2\n3\n")]);
    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--csv-file-path")
        .arg(data.path().join("rows.csv"))
        .args(["--iterations", "2"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(paths(&s).await, ["/n/0", "/n/1"]);
}

#[test]
fn iterations_zero_is_a_usage_error() {
    let out = apinata()
        .args(["run", ".", "--iterations", "0"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--iterations"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_run_sees_iteration_zero_of_one() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get(
                "A",
                1,
                &format!("{}/i/{{{{iteration}}}}/of/{{{{iterationCount}}}}", s.uri()),
                &check("res.status", "eq", "200"),
            ),
        ),
    ]);
    let out = apinata().arg("run").arg(dir.path()).output().expect("runs");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(!text.contains("Iteration"), "{text}");
    assert_eq!(paths(&s).await, ["/i/0/of/1"]);
}
