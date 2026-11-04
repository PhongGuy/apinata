//! The stream value types, the Response fields and the helper server itself.

use crate::common::sse_server::{Script, Step, event, slow_sse_server};
use apinata_core::sse::SseEvent;
use apinata_core::{Response, StreamBound, StreamEnd};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[test]
fn every_end_has_its_label() {
    let labels = [
        (StreamEnd::Closed, "closed"),
        (StreamEnd::Stopped, "stopped"),
        (StreamEnd::Bound(StreamBound::Time), "time_limit"),
        (StreamEnd::Bound(StreamBound::Events), "event_limit"),
        (StreamEnd::IdleTimeout, "idle_timeout"),
        (StreamEnd::BodyCap, "body_cap"),
        (StreamEnd::Error("x".into()), "error"),
    ];
    for (end, label) in labels {
        assert_eq!(end.label(), label);
    }
}

#[test]
fn a_response_carries_its_end_and_parses_its_events() {
    let plain = Response::new(200).with_text_body("hi");
    assert_eq!(plain.stream_end(), None);
    let streamed = Response::new(200)
        .with_text_body("data: a\n\ndata: b\n\n")
        .with_stream_end(StreamEnd::Stopped);
    assert_eq!(streamed.stream_end(), Some(&StreamEnd::Stopped));
    assert_eq!(
        streamed.events(),
        vec![SseEvent::new("a"), SseEvent::new("b")]
    );
}

#[tokio::test]
async fn the_helper_writes_chunks_with_pauses_and_counts_requests() {
    let s = slow_sse_server(Script::sse(vec![
        event("a"),
        Step::Pause(Duration::from_millis(50)),
        event("b"),
        Step::Finish,
    ]))
    .await;
    let mut sock = TcpStream::connect(("127.0.0.1", s.port())).await.unwrap();
    sock.write_all(b"GET / HTTP/1.1\r\nhost: x\r\n\r\n")
        .await
        .unwrap();
    let mut got = Vec::new();
    sock.read_to_end(&mut got).await.unwrap();
    let text = String::from_utf8_lossy(&got);
    assert!(text.contains("content-type: text/event-stream"), "{text}");
    assert!(
        text.contains("data: a") && text.contains("data: b"),
        "{text}"
    );
    assert!(text.ends_with("0\r\n\r\n"), "{text}");
    assert_eq!(s.requests(), 1);
}
