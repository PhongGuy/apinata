//! A request whose URL query and params block disagree is named on stderr once per process,
//! the URL's query is what goes out, and `--quiet` silences the line.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::path::Path;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const A_LINE: &str = "warning: a.yml: URL query and params disagree on z; the URL's query is sent";

fn request(name: &str, seq: u32, url: &str, z: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n  params:\n    - name: z\n      value: \"{z}\"\n      type: query\n"
    )
}

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn collection(s: &MockServer) -> tempfile::TempDir {
    let url = format!("{}/ok?z=6", s.uri());
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1, &url, "3")),
        ("b.yml", &request("b", 2, &url, "6")),
    ])
}

fn run(dir: &Path, before: &[&str], after: &[&str]) -> (i32, String) {
    let out = apinata()
        .args(before)
        .arg("run")
        .arg(dir)
        .args(after)
        .output()
        .expect("runs");
    (
        out.status.code().expect("exit code"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disagreeing_request_is_named_once_and_sends_the_url_query() {
    let s = server().await;
    let dir = collection(&s);
    let (code, err) = run(dir.path(), &[], &["--iterations", "2"]);
    assert_eq!(code, 0, "{err}");
    let warnings: Vec<&str> = err.lines().filter(|l| l.contains("disagree")).collect();
    assert_eq!(warnings, vec![A_LINE], "{err}");
    let queries: Vec<String> = s
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.query().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(queries, vec!["z=6"; 4]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_hides_the_disagreement_line() {
    let s = server().await;
    let dir = collection(&s);
    let (code, err) = run(dir.path(), &[], &["--quiet"]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("disagree"), "{err}");
    let (code, err) = run(dir.path(), &["--quiet"], &[]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("disagree"), "{err}");
}
