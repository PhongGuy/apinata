//! Executor errors and Debug output never reveal query values or cookies.

use crate::common;

use crate::common::dead_port;
use apinata_core::{ExecOptions, Executor};
use common::resolved_one;

#[tokio::test]
async fn a_failed_hop_redacts_its_url() {
    // A port that refuses every connection.
    let port = dead_port();
    let url = format!("http://127.0.0.1:{port}/x?api_key=SECRET");
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let err = exec
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect_err("refused");
    for shown in [err.to_string(), format!("{err:?}")] {
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(shown.contains("api_key=<redacted>"), "{shown}");
    }
}

#[test]
fn executor_debug_omits_the_cookie_jar() {
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let shown = format!("{exec:?}");
    assert!(shown.starts_with("Executor"), "{shown}");
    assert!(!shown.contains("jar"), "{shown}");
    assert!(shown.ends_with(".. }"), "{shown}");
}
