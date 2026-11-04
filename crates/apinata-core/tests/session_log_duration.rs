//! The Network log duration is the request timing users see, not the work done after the body.

use crate::common::body_server::{Reply, body_server, pattern};
use crate::common::resolved_one;
use crate::session_log_support::logged;
use apinata_core::ExecOptions;
use std::time::Duration;

const MIB: usize = 1024 * 1024;

#[tokio::test]
async fn a_large_binary_body_does_not_lengthen_the_logged_duration() {
    // Bytes that are not UTF-8 make the text decode of the kept body slow, which must not show.
    let reply = Reply::with_headers(
        pattern(3 * MIB),
        "content-type: application/octet-stream\r\n",
    );
    let s = body_server(vec![("/", reply)]).await;
    let (exec, log) = logged(ExecOptions::new().max_body_bytes(Some(MIB as u64)));
    let res = exec
        .send(&resolved_one("GET", &s.url("/"), "", ""))
        .await
        .expect("sent");
    assert!(res.truncated());
    let entry = log.network_since(0).entries.remove(0);
    assert!(entry.duration >= entry.hops[0].elapsed);
    assert!(
        entry.duration <= res.response_time() + Duration::from_millis(100),
        "logged {:?} against a response time of {:?}",
        entry.duration,
        res.response_time()
    );
}
