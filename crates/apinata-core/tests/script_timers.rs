//! bru.sleep and setTimeout wait on the host without blocking, under the phase limit.

use crate::script_support;

use apinata_core::Outcome;
use script_support::*;
use std::time::{Duration, Instant};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn tree(code: &str) -> (MockServer, tempfile::TempDir) {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let text = scripted(
        "R",
        &format!("{}/r", s.uri()),
        &script("before-request", code),
        "",
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    (s, dir)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sleep_and_set_timeout_wait_without_blocking() {
    let code = r#"const t = Date.now();
const v = await bru.sleep("50");
const order = [];
const p = setTimeout((...a) => order.push("timer" + a.length), 30, 1, 2);
order.push("sync");
const done = await p;
console.log(v, Date.now() - t >= 75, order.join(","), done, typeof setTimeout(() => {}, 0).then);"#;
    let (_s, dir) = tree(code).await;
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.error);
    assert_eq!(
        console(&result),
        ["log:slept true sync,timer0 undefined function"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_sleep_stops_at_the_time_limit() {
    let (_s, dir) = tree("await bru.sleep(1e9);").await;
    let start = Instant::now();
    let result = send(dir.path(), "r.yml", limited(200)).await;
    assert!(
        start.elapsed() < Duration::from_millis(400),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(result.outcome, Outcome::Errored);
    assert!(script_error(&result).contains("script exceeded the 0.2 s limit"));
}
