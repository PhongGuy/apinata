//! A GraphQL request whose response carries `errors` is named on stderr with their summary;
//! an HTTP request with the same body is not, and `--quiet` silences the line.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::path::Path;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const BODY: &str = r#"{"errors":[{"message":"Not authorised","path":["me"]}],"data":null}"#;
const LINE: &str = "warning: g.yml: 1 GraphQL error: Not authorised (at me)";

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/gql"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(BODY, "application/json"))
        .mount(&s)
        .await;
    s
}

fn collection(s: &MockServer) -> tempfile::TempDir {
    let url = format!("{}/gql", s.uri());
    let g = format!(
        "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"{url}\"\n  body:\n    query: \"{{ me }}\"\n"
    );
    let h = format!(
        "info:\n  name: h\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: \"{url}\"\n"
    );
    write_tree(&[("opencollection.yml", ROOT), ("g.yml", &g), ("h.yml", &h)])
}

fn stderr(dir: &Path, extra: &[&str]) -> String {
    let out = apinata()
        .arg("run")
        .arg(dir)
        .args(extra)
        .output()
        .expect("runs");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_graphql_request_is_named_with_its_errors() {
    let s = server().await;
    let dir = collection(&s);
    let err = stderr(dir.path(), &[]);
    let lines: Vec<&str> = err.lines().filter(|l| l.contains("GraphQL")).collect();
    assert_eq!(lines, vec![LINE], "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_hides_the_graphql_errors_line() {
    let s = server().await;
    let dir = collection(&s);
    let err = stderr(dir.path(), &["--quiet"]);
    assert!(!err.contains("GraphQL"), "{err}");
}
