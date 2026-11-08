//! A subscription's transcript saved through `StreamOptions::body_file`.

use crate::common::ws_server::{Step, WsScript, raw_server, sub, tick, ws_exec, ws_server};
use apinata_core::HttpVersionPolicy::Auto;
use apinata_core::{CancelHandle, ExecError, StreamEnd, StreamEvent, StreamLimits, StreamOptions};
use std::time::{Duration, Instant};

const TRANSCRIPT: &str = "event: next\ndata: {\"data\":{\"tick\":1}}\n\nevent: complete\ndata:\n\n";

#[tokio::test]
async fn the_transcript_is_written_to_the_body_file() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::Complete])).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.txt");
    let opts = StreamOptions::new().body_file(&path);
    let res = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/")), &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(res.body_file(), Some(path.as_path()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), TRANSCRIPT);
    assert_eq!(res.body(), TRANSCRIPT.as_bytes());
}

#[tokio::test]
async fn a_stopped_run_keeps_what_arrived() {
    let s = ws_server(WsScript::new(vec![tick(1)])).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.txt");
    let cancel = CancelHandle::new();
    let stopper = cancel.clone();
    let opts = StreamOptions::new().body_file(&path).sink(move |e| {
        if matches!(e, StreamEvent::Chunk(_)) {
            stopper.stop();
        }
    });
    let res = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/")), &cancel, opts)
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Stopped));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "event: next\ndata: {\"data\":{\"tick\":1}}\n\n"
    );
}

#[tokio::test]
async fn a_refused_handshake_writes_no_file() {
    let port = raw_server("HTTP/1.1 404 Not Found\r\ncontent-length: 4\r\n\r\nnope").await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.txt");
    let url = format!("http://127.0.0.1:{port}/graphql");
    let res = ws_exec(Auto)
        .send_streaming(
            &sub(&url),
            &CancelHandle::new(),
            StreamOptions::new().body_file(&path),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
    assert_eq!(res.body_file(), None);
    assert!(!path.exists());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "no temp file left"
    );
}

#[tokio::test]
async fn an_unwritable_body_file_fails_before_the_run_starts() {
    // The server never completes, so only an early failure ends this send quickly.
    let s = ws_server(WsScript::new(vec![tick(1)])).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing").join("out.txt");
    let opts = StreamOptions::new()
        .limits(StreamLimits::new().max_duration(Some(Duration::from_secs(30))))
        .body_file(&path);
    let started = Instant::now();
    let err = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/")), &CancelHandle::new(), opts)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ExecError::Transport { message, .. } if message.starts_with("could not write ")),
        "{err}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(
        !s.seen.lines().iter().any(|l| l.starts_with("recv: ")),
        "nothing is sent on the socket: {:?}",
        s.seen.lines()
    );
}
