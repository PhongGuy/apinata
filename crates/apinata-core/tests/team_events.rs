//! The SSE notice client against a stub stream on 127.0.0.1.

use crate::common::sse_server::{Script, SlowServer, Step, slow_sse_server};
use crate::team_auth::TOKEN;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use apinata_core::team::{
    EventsConfig, EventsEnd, TeamAuth, TeamNotice, follow_events, reconnect_delay,
};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PATH: &str = "/api/v1/workspaces/t/w/events";

fn ref_event(name: &str, sha: &str) -> Step {
    Step::Chunk(format!(
        "event: ref\ndata: {{\"ref\":\"{name}\",\"sha\":\"{sha}\"}}\n\n"
    ))
}

fn config(server: &SlowServer, first_ms: u64, poll_ms: u64, idle_ms: u64) -> EventsConfig {
    EventsConfig::new(
        server.url(PATH),
        TeamAuth::new(server.url(""), TOKEN),
        "main",
    )
    .timings(
        Duration::from_millis(first_ms),
        Duration::from_millis(first_ms * 4),
        Duration::from_millis(poll_ms),
        Duration::from_millis(idle_ms),
    )
}

/// What the client reported within `ms`; it must still be running then.
async fn notices_within(config: EventsConfig, ms: u64) -> Vec<TeamNotice> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let ended = tokio::time::timeout(
        Duration::from_millis(ms),
        follow_events(config, move |n| sink.lock().unwrap().push(n)),
    )
    .await;
    assert!(ended.is_err(), "the client ended: {ended:?}");
    seen.lock().unwrap().clone()
}

fn status(code: &str) -> Script {
    Script::raw_head(
        format!("HTTP/1.1 {code}\r\ncontent-length: 0\r\n\r\n"),
        Vec::new(),
    )
}

#[tokio::test]
async fn branch_ref_events_arrive_and_other_refs_and_keep_alives_do_not() {
    let server = slow_sse_server(Script::sse(vec![
        ref_event("refs/heads/main", A),
        Step::Chunk(": keep-alive\n\n".into()),
        ref_event("refs/heads/other", B),
        ref_event("refs/heads/main", B),
    ]))
    .await;
    let got = notices_within(config(&server, 20, 1000, 5000), 300).await;
    assert_eq!(
        got,
        [
            TeamNotice::Connected,
            TeamNotice::Ref { sha: A.into() },
            TeamNotice::Ref { sha: B.into() }
        ]
    );
}

#[tokio::test]
async fn the_token_travels_only_in_the_bearer_header() {
    let server = slow_sse_server(Script::sse(Vec::new())).await;
    notices_within(config(&server, 20, 1000, 5000), 200).await;
    let head = server.heads()[0].to_ascii_lowercase();
    assert!(head.starts_with(&format!("get {PATH} http/1.1")), "{head}");
    assert!(
        head.contains(&format!(
            "authorization: bearer {}",
            TOKEN.to_ascii_lowercase()
        )),
        "{head}"
    );
    assert!(head.contains("accept: text/event-stream"), "{head}");
    assert_eq!(
        head.matches("plk_").count(),
        1,
        "only the header carries it"
    );
}

#[tokio::test]
async fn a_dropped_stream_reconnects() {
    let server = slow_sse_server(Script::sse(vec![
        ref_event("refs/heads/main", A),
        Step::Abort,
    ]))
    .await;
    let got = notices_within(config(&server, 20, 1000, 5000), 400).await;
    assert!(server.requests() >= 2, "{}", server.requests());
    assert_eq!(
        got[..4],
        [
            TeamNotice::Connected,
            TeamNotice::Ref { sha: A.into() },
            TeamNotice::Disconnected,
            TeamNotice::Connected
        ]
    );
}

#[tokio::test]
async fn while_down_it_polls_and_reports_one_disconnect() {
    let server = slow_sse_server(status("503 Service Unavailable")).await;
    let got = notices_within(config(&server, 200, 30, 5000), 300).await;
    assert!(
        got.iter().filter(|n| **n == TeamNotice::Poll).count() >= 3,
        "{got:?}"
    );
    assert!(!got.contains(&TeamNotice::Connected));
    assert!(
        got.iter()
            .filter(|n| **n == TeamNotice::Disconnected)
            .count()
            <= 1,
        "{got:?}"
    );
}

#[tokio::test]
async fn a_401_ends_signed_out_and_a_404_ends_gone() {
    for (code, want) in [
        ("401 Unauthorized", EventsEnd::SignedOut),
        ("404 Not Found", EventsEnd::Gone),
    ] {
        let server = slow_sse_server(status(code)).await;
        let ended = tokio::time::timeout(
            Duration::from_secs(2),
            follow_events(config(&server, 20, 1000, 5000), |_| {}),
        )
        .await;
        assert_eq!(ended.unwrap(), want, "{code}");
    }
}

#[tokio::test]
async fn a_stream_silent_past_the_idle_limit_reconnects() {
    let server = slow_sse_server(Script::sse(vec![ref_event("refs/heads/main", A)])).await;
    notices_within(config(&server, 20, 1000, 100), 500).await;
    assert!(server.requests() >= 2, "{}", server.requests());
}

#[test]
fn the_reconnect_delay_doubles_to_its_cap_with_jitter() {
    let (s, m) = (Duration::from_secs(1), Duration::from_secs(60));
    assert_eq!(reconnect_delay(0, s, m, 0.0), Duration::from_millis(500));
    assert_eq!(reconnect_delay(0, s, m, 1.0), Duration::from_secs(1));
    assert_eq!(reconnect_delay(3, s, m, 1.0), Duration::from_secs(8));
    assert_eq!(reconnect_delay(10, s, m, 1.0), Duration::from_secs(60));
    assert_eq!(
        reconnect_delay(u32::MAX, s, m, 0.0),
        Duration::from_secs(30)
    );
}

#[tokio::test]
async fn polls_keep_their_period_across_reconnect_waits() {
    // Every backoff wait is shorter than the poll period, so only a period kept across waits fires.
    let server = slow_sse_server(status("503 Service Unavailable")).await;
    let got = notices_within(config(&server, 25, 150, 5000), 700).await;
    assert!(
        got.iter().filter(|n| **n == TeamNotice::Poll).count() >= 3,
        "{got:?}"
    );
}

#[tokio::test]
async fn a_stream_that_closes_before_any_event_backs_off() {
    let server = slow_sse_server(Script::sse(vec![Step::Finish])).await;
    let config = EventsConfig::new(
        server.url(PATH),
        TeamAuth::new(server.url(""), TOKEN),
        "main",
    )
    .timings(
        Duration::from_millis(20),
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(5),
    );
    let got = notices_within(config, 500).await;
    assert!(server.requests() <= 8, "{}", server.requests());
    assert!(!got.contains(&TeamNotice::Connected), "{got:?}");
    assert!(
        got.iter()
            .filter(|n| **n == TeamNotice::Disconnected)
            .count()
            <= 1,
        "{got:?}"
    );
}

#[test]
fn a_roll_that_is_not_a_number_takes_the_full_delay() {
    let (s, m) = (Duration::from_secs(1), Duration::from_secs(60));
    assert_eq!(reconnect_delay(0, s, m, f64::NAN), Duration::from_secs(1));
}
