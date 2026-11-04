use crate::support;
use crate::support::sse_server::{Script, Step, event};
use crate::support::*;
use apinata_app::live_stream::end_label;
use apinata_app::response_pane::ResponsePane;
use apinata_core::{StreamBound, StreamEnd};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};

#[test]
fn every_end_has_a_short_label() {
    assert_eq!(end_label(&StreamEnd::Closed), "closed");
    assert_eq!(end_label(&StreamEnd::Stopped), "stopped");
    assert_eq!(end_label(&StreamEnd::Bound(StreamBound::Time)), "limit");
    assert_eq!(end_label(&StreamEnd::Bound(StreamBound::Events)), "limit");
    assert_eq!(end_label(&StreamEnd::IdleTimeout), "idle timeout");
    assert_eq!(end_label(&StreamEnd::BodyCap), "size limit");
    assert_eq!(end_label(&StreamEnd::Error("x".into())), "error");
}

fn open_pane(
    cx: &mut TestAppContext,
    script: Script,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<ResponsePane>,
    gpui_kit::Entity<apinata_app::send_model::SendModel>,
    std::path::PathBuf,
    support::sse_server::SlowServer,
    tempfile::TempDir,
) {
    http_ready(cx);
    let server = sse(cx, script);
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    support::init(cx);
    let m = model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    (win, pane, model, root, server, col)
}

#[gpui_kit::test]
fn the_status_row_shows_streaming_with_a_timer_then_the_reason(cx: &mut TestAppContext) {
    let (win, pane, model, root, _server, _col) = open_pane(cx, Script::sse(vec![event("one")]));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    assert_eq!(
        pane.read_with(cx, |p, cx| p.stream_status(cx)).as_deref(),
        Some("streaming")
    );
    assert!(present(cx, win, "response-stream-state"));
    assert!(present(cx, win, "response-elapsed"));
    assert!(present(cx, win, "response-status"));
    assert!(!present(cx, win, "response-stream-end"));
    model.update(cx, |m, cx| m.stop(cx));
    finished(cx, &model);
    assert_eq!(
        pane.read_with(cx, |p, cx| p.stream_status(cx)).as_deref(),
        Some("stopped")
    );
    assert!(present(cx, win, "response-stream-end"));
    assert!(!present(cx, win, "response-stream-state"));
}

#[gpui_kit::test]
fn a_server_that_finishes_reads_closed(cx: &mut TestAppContext) {
    let (win, pane, model, root, _server, _col) =
        open_pane(cx, Script::sse(vec![event("one"), Step::Finish]));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    assert_eq!(
        pane.read_with(cx, |p, cx| p.stream_status(cx)).as_deref(),
        Some("closed")
    );
    assert!(present(cx, win, "response-stream-end"));
}

#[gpui_kit::test]
fn an_ordinary_response_has_no_stream_status(cx: &mut TestAppContext) {
    let (_win, pane, model, root, _server, _col) = open_pane(
        cx,
        Script::with_type("text/plain", vec![Step::Chunk("hi".into()), Step::Finish]),
    );
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    assert_eq!(pane.read_with(cx, |p, cx| p.stream_status(cx)), None);
}
