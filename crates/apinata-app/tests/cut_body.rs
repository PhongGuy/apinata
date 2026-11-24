//! A body the server cuts short still shows the status and headers that arrived, with a
//! notice of how many bytes came, and the send still counts as failed.

use crate::support;

use std::io::{Read as _, Write as _};

use apinata_app::response_pane::{ErrorKind, ResponsePane};
use apinata_core::Outcome;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use support::*;

/// A 127.0.0.1 server that answers every request with a 201 promising 100 bytes, sends 10
/// and closes; its base URL.
fn cut_server() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for mut sock in listener.incoming().flatten() {
            let mut head = [0u8; 4096];
            let _ = sock.read(&mut head);
            let _ = sock.write_all(
                b"HTTP/1.1 201 Created\r\nx-trace: t-42\r\ncontent-length: 100\r\n\r\naaaaaaaaaa",
            );
        }
    });
    format!("http://127.0.0.1:{port}")
}

#[gpui_kit::test]
fn a_cut_body_keeps_the_head_with_a_notice_in_the_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection_for(&cut_server(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Errored);
    let head = result.response.as_ref().expect("the head is shown");
    assert_eq!(head.status(), 201);
    assert!(
        (head.headers().iter()).any(|(k, v)| k == "x-trace" && v == "t-42"),
        "{:?}",
        head.headers()
    );
    assert!(head.body().is_empty());

    support::init(cx);
    let m = model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let shown = view
        .read_with(cx, |p, cx| p.error_view(cx))
        .expect("a notice");
    assert_eq!(
        (shown.kind, shown.title),
        (ErrorKind::BodyCut, "Body cut short")
    );
    let sentence = shown.sentence.unwrap_or_default();
    assert!(sentence.contains("10 of 100 bytes"), "{sentence}");
    assert!(present(cx, win, "response-status"));
    assert!(present(cx, win, "response-error"));
}
