//! A URL padded with spaces is sent trimmed, and an empty one is named as such.

use crate::common;

use common::{ROOT, apinata, check, column, get, run_json, write_tree};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_padded_url_is_sent_and_an_empty_one_is_reported_by_name() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let padded = format!("  {}/ok \\n", s.uri());
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &get("padded", 1, &padded, &check("res.status", "eq", "200")),
        ),
        ("b.yml", &get("nourl", 2, "  ", "    []\n")),
    ]);
    let (code, report) = run_json(dir.path(), &["--no-scripts"]);
    assert_eq!(code, 2, "{report}");
    assert_eq!(column(&report, "outcome"), ["passed", "errored"]);
    let text = report["results"][1]["error"].as_str().unwrap_or_default();
    assert!(text.contains("the URL is empty"), "{report}");

    let out = apinata()
        .arg("run")
        .arg(dir.path())
        .arg("--no-scripts")
        .output()
        .expect("runs");
    let both =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(both.contains("b (the URL is empty)"), "{both}");
}
