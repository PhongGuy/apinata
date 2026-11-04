//! Script timeouts: NaN, Infinity and negatives mean no limit and a huge one is capped at a day;
//! the send still runs.

use crate::script_support;

use apinata_core::Outcome;
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tree(url: &str, code: &str) -> tempfile::TempDir {
    let text = scripted("R", url, &script("before-request", code), "");
    write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)])
}

async fn served(times: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(path("/x"))
        .respond_with(ResponseTemplate::new(200))
        .expect(times)
        .mount(&s)
        .await;
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_send_with_an_unusable_timeout_still_sends() {
    let s = served(3).await;
    let code = format!(
        r#"for (const timeout of [NaN, Infinity, -5]) {{
  const r = await bru.sendRequest({{ url: "{}/x", timeout }});
  console.log(String(timeout), r.status);
}}"#,
        s.uri()
    );
    let dir = tree(&format!("{}/r", s.uri()), &code);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(
        console(&result),
        ["log:NaN 200", "log:Infinity 200", "log:-5 200"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_timeout_is_capped_like_a_script_send() {
    // Infinity reaches the host as null, so it means none, as NaN and negatives do.
    let cases = [
        ("Infinity", "0"),
        ("NaN", "0"),
        ("-5", "0"),
        ("1e300", "86400000"),
        ("2500", "2500"),
    ];
    for (timeout, shown) in cases {
        let s = served(1).await;
        let code = format!("req.setTimeout({timeout}); console.log(req.getTimeout());");
        let dir = tree(&format!("{}/x", s.uri()), &code);
        let result = send(dir.path(), "r.yml", on()).await;
        assert_eq!(
            result.outcome,
            Outcome::Passed,
            "{timeout}: {:?}",
            result.error
        );
        assert_eq!(console(&result), [format!("log:{shown}")], "{timeout}");
    }
}
