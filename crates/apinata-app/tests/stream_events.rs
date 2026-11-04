use crate::support;
use crate::support::sse_server::{Script, Step, event};
use crate::support::*;
use apinata_app::live_stream::{EventRow, PREVIEW_CHARS, event_rows, preview};
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_core::sse::SseEvent;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use std::sync::Arc;

#[test]
fn preview_keeps_three_lines_and_marks_the_cut() {
    assert_eq!(preview("a\nb"), "a\nb");
    assert_eq!(preview("a\nb\nc\nd\ne"), "a\nb\nc\u{2026}");
    let long = "x".repeat(PREVIEW_CHARS + 50);
    assert_eq!(
        preview(&long),
        format!("{}\u{2026}", "x".repeat(PREVIEW_CHARS))
    );
    assert_eq!(
        preview(&"y".repeat(PREVIEW_CHARS)),
        "y".repeat(PREVIEW_CHARS)
    );
}

#[test]
fn rows_name_the_event_and_keep_its_id() {
    let events = vec![
        SseEvent::new("hi").with_event("tick").with_id("7"),
        SseEvent::new("plain"),
    ];
    assert_eq!(
        event_rows(&events),
        [
            EventRow {
                name: "tick".into(),
                id: Some("7".into()),
                preview: "hi".into()
            },
            EventRow {
                name: "message".into(),
                id: None,
                preview: "plain".into()
            },
        ]
    );
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
fn an_event_stream_lists_events_live_and_after_the_end(cx: &mut TestAppContext) {
    let steps = vec![
        event("one"),
        Step::Chunk("event: tick\nid: 2\ndata: two\n\n".into()),
    ];
    let (win, pane, model, root, _server, _col) = open_pane(cx, Script::sse(steps));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 2))
    });
    assert_eq!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx))[..2],
        [ResponseTab::Body, ResponseTab::Events]
    );
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    assert!(present(cx, win, "response-events"));
    let rows = pane.read_with(cx, |p, cx| p.event_rows(cx));
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (
            rows[1].name.as_str(),
            rows[1].id.as_deref(),
            rows[1].preview.as_str()
        ),
        ("tick", Some("2"), "two")
    );
    model.update(cx, |m, cx| m.stop(cx));
    finished(cx, &model);
    assert!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events)
    );
    // The live events carry over, so the list and badge never blank before the parse lands.
    assert_eq!(pane.read_with(cx, |p, cx| p.event_rows(cx).len()), 2);
    assert_eq!(
        pane.read_with(cx, |p, cx| p.tab_count(ResponseTab::Events, cx)),
        Some(2)
    );
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, cx| p.event_rows(cx).len() == 2)
    });
}

#[gpui_kit::test]
fn a_plain_stream_has_no_events_tab(cx: &mut TestAppContext) {
    let (win, pane, model, root, _server, _col) = open_pane(
        cx,
        Script::with_type("text/plain", vec![Step::Chunk("hi\n".into())]),
    );
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    assert!(
        !pane
            .read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events)
    );
    // Events picked earlier falls back to the Body view, and the strip leads with Body.
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    assert!(present(cx, win, "live-body"));
    assert_eq!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx))[0],
        ResponseTab::Body
    );
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}

#[gpui_kit::test]
fn a_kept_event_stream_response_shows_events_too(cx: &mut TestAppContext) {
    let (_win, pane, model, _root, _server, _col) = open_pane(cx, Script::sse(vec![]));
    let response = Response::new(200)
        .with_header("Content-Type", "text/event-stream; charset=utf-8")
        .with_text_body("data: x\n\ndata: y\n\n");
    let result =
        RequestResult::from_parts(ip("a.yml"), "A", Outcome::Passed, Some(response), vec![]);
    model.update(cx, |m, cx| m.show(Arc::new(result), cx));
    assert!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events)
    );
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, cx| p.event_rows(cx).len() == 2)
    });
}

#[gpui_kit::test]
fn clicking_an_event_copies_its_whole_data(cx: &mut TestAppContext) {
    let big = format!("{}\nsecond line", "z".repeat(PREVIEW_CHARS + 20));
    let (win, pane, model, root, _server, _col) = open_pane(
        cx,
        Script::sse(vec![Step::Chunk(format!(
            "data: {}\ndata: second line\n\n",
            "z".repeat(PREVIEW_CHARS + 20)
        ))]),
    );
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 1))
    });
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    assert!(present(cx, win, ("event-row", 0usize)));
    cx.update_window(win, |_, w, cx| w.click(("event-row", 0usize), cx))
        .unwrap();
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(big));
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}

#[gpui_kit::test]
fn dropped_events_show_a_muted_note(cx: &mut TestAppContext) {
    let burst: String = (0..10_050).map(|_| "data: x\n\n").collect();
    let (win, pane, model, root, _server, _col) =
        open_pane(cx, Script::sse(vec![Step::Chunk(burst)]));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| {
            m.live().is_some_and(|l| l.dropped_events() == 50)
        })
    });
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    assert_eq!(pane.read_with(cx, |p, cx| p.events_dropped(cx)), 50);
    assert!(present(cx, win, "events-note"));
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}
