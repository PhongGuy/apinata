//! A body the server cuts short fails the send, keeping the head that arrived and the counts.

use crate::common::body_server::{Reply, Tail, body_server};
use crate::common::resolved_one;
use apinata_core::{CancelHandle, ExecError, ExecOptions, Executor, StreamOptions};

fn exec() -> Executor {
    Executor::new(ExecOptions::new()).unwrap()
}

/// A 201 with a trace header that promises 100 bytes, sends 10 and closes.
fn cut_reply() -> Reply {
    Reply {
        head: "HTTP/1.1 201 Created\r\nx-trace: t-42\r\ncontent-length: 100\r\n\r\n".to_owned(),
        tail: Tail::Close,
        ..Reply::plain(vec![b'a'; 10])
    }
}

fn assert_cut(err: &ExecError, expected: Option<u64>) {
    let ExecError::BodyCut { cut, .. } = err else {
        panic!("{err:?}");
    };
    assert_eq!(cut.status, 201);
    assert!(
        (cut.headers.iter()).any(|(k, v)| k == "x-trace" && v == "t-42"),
        "{:?}",
        cut.headers
    );
    assert_eq!((cut.received, cut.expected), (10, expected));
}

#[tokio::test]
async fn a_plain_send_keeps_the_head_and_both_counts() {
    let s = body_server(vec![("/", cut_reply())]).await;
    let err = exec()
        .send(&resolved_one("GET", &s.url("/"), "", ""))
        .await
        .unwrap_err();
    assert_cut(&err, Some(100));
    let text = err.to_string();
    assert!(text.contains("10 of 100 bytes"), "{text}");
}

#[tokio::test]
async fn a_streaming_send_keeps_the_head_and_both_counts() {
    let s = body_server(vec![("/", cut_reply())]).await;
    let err = exec()
        .send_streaming(
            &resolved_one("GET", &s.url("/"), "", ""),
            &CancelHandle::new(),
            StreamOptions::new(),
        )
        .await
        .unwrap_err();
    assert_cut(&err, Some(100));
}

#[tokio::test]
async fn a_cut_chunked_body_has_no_expected_count() {
    // The capitalised header keeps the test server from framing, so the chunk is sent raw
    // and the connection closes before the last chunk.
    let reply = Reply {
        head: "HTTP/1.1 201 Created\r\nx-trace: t-42\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_owned(),
        tail: Tail::Close,
        ..Reply::plain(b"a\r\naaaaaaaaaa\r\n".to_vec())
    };
    let s = body_server(vec![("/", reply)]).await;
    let err = exec()
        .send(&resolved_one("GET", &s.url("/"), "", ""))
        .await
        .unwrap_err();
    assert_cut(&err, None);
    let text = err.to_string();
    assert!(text.contains("after 10 bytes"), "{text}");
}

#[test]
fn the_constructor_carries_every_field() {
    let e = ExecError::body_cut(
        "http://a.test/",
        200,
        vec![("x".to_owned(), "y".to_owned())],
        3,
        Some(9),
        "closed",
    );
    let ExecError::BodyCut {
        url, cut, message, ..
    } = &e
    else {
        panic!("{e:?}");
    };
    assert_eq!(
        (url.as_str(), message.as_str()),
        ("http://a.test/", "closed")
    );
    assert_eq!(
        (cut.status, cut.headers.len(), cut.received, cut.expected),
        (200, 1, 3, Some(9))
    );
}
