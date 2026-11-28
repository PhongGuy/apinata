//! `apinata run` reads only files inside the collection unless `--allow-outside-files` is given.

use crate::common;

use common::{ROOT, column, run_json, write_tree};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SENTINEL: &str = "outside-sentinel";

/// `<tmp>/coll` holds one request whose multipart part names `<tmp>/secret.txt`.
fn layout(url: &str) -> tempfile::TempDir {
    let request = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{url}\"\n  body:\n    type: multipart-form\n    data:\n      - name: f\n        type: file\n        value:\n          - \"../secret.txt\"\n"
    );
    write_tree(&[
        ("coll/opencollection.yml", ROOT),
        ("coll/r.yml", &request),
        ("secret.txt", SENTINEL),
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_outside_the_collection_is_refused_and_named_with_the_flag() {
    let s = MockServer::start().await;
    Mock::given(path("/up"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = layout(&format!("{}/up", s.uri()));
    let (_, report) = run_json(&dir.path().join("coll"), &["--no-scripts"]);
    assert_eq!(column(&report, "outcome"), ["errored"], "{report}");
    let error = &column(&report, "error")[0];
    assert!(error.contains("../secret.txt"), "{error}");
    let hint = &column(&report, "hint")[0];
    assert!(hint.contains("--allow-outside-files"), "{hint}");
    assert!(!report.to_string().contains(SENTINEL), "{report}");
    assert!(s.received_requests().await.unwrap_or_default().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_flag_reads_a_file_outside_the_collection() {
    let s = MockServer::start().await;
    Mock::given(path("/up"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let dir = layout(&format!("{}/up", s.uri()));
    let (_, report) = run_json(
        &dir.path().join("coll"),
        &["--no-scripts", "--allow-outside-files"],
    );
    assert_eq!(column(&report, "outcome"), ["passed"], "{report}");
    let sent = s.received_requests().await.unwrap_or_default();
    assert!(String::from_utf8_lossy(&sent[0].body).contains(SENTINEL));
}
