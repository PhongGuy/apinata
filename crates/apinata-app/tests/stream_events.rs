use crate::support;
use crate::support::sse_server::{Script, Step, event};
use crate::support::*;
use apinata_app::live_stream::{
    EventRow, LiveStream, PREVIEW_CHARS, event_rows, preview, retry_label, time_label,
};
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_core::sse::SseEvent;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn preview_keeps_the_first_line_and_marks_the_cut() {
    assert_eq!(preview("a"), "a");
    assert_eq!(preview("a\nb"), "a\u{2026}");
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

// A multi-line event reads as its first line, on one row no taller than a list row.
#[gpui_kit::test]
fn a_multi_line_event_shows_only_its_first_line(cx: &mut TestAppContext) {
    let (win, pane, model, root, _server, _col) = open_pane(
        cx,
        Script::sse(vec![Step::Chunk("data: first\ndata: second\n\n".into())]),
    );
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 1))
    });
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    let rows = pane.read_with(cx, |p, cx| p.event_rows(cx));
    assert_eq!(rows[0].preview, "first\u{2026}");
    let (row, line) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let row = w.find(("event-row", 0usize)).bounds();
            (
                row,
                apinata_app::style::resolve(apinata_app::style::ROW_HEIGHT, w),
            )
        })
        .unwrap();
    assert!(
        row.size.height <= line + gpui_kit::px(0.5),
        "{row:?} beside {line:?}"
    );
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}

#[test]
fn rows_name_the_event_and_keep_its_id() {
    let events = vec![
        SseEvent::new("hi").with_event("tick").with_id("7"),
        SseEvent::new("plain"),
        SseEvent::new("slow").with_retry(3000),
    ];
    assert_eq!(
        event_rows(&events),
        [
            EventRow {
                name: "tick".into(),
                id: Some("7".into()),
                retry: None,
                preview: "hi".into(),
                time: None,
            },
            EventRow {
                name: "message".into(),
                id: None,
                retry: None,
                preview: "plain".into(),
                time: None,
            },
            EventRow {
                name: "message".into(),
                id: None,
                retry: Some(3000),
                preview: "slow".into(),
                time: None,
            },
        ]
    );
}

#[test]
fn retry_shows_in_milliseconds_only_when_sent() {
    assert_eq!(retry_label(Some(3000)).as_deref(), Some("retry 3000 ms"));
    assert_eq!(retry_label(None), None);
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
    // A kept response has no arrival times, so its rows show blank ones.
    assert!(
        pane.read_with(cx, |p, cx| p.event_rows(cx))
            .iter()
            .all(|r| r.time.is_none())
    );
}

// One event whose data runs past the preview and onto a second line.
fn long_event(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<ResponsePane>,
    gpui_kit::Entity<apinata_app::send_model::SendModel>,
    support::sse_server::SlowServer,
    tempfile::TempDir,
) {
    let (win, pane, model, root, server, col) = open_pane(
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
    (win, pane, model, server, col)
}

#[gpui_kit::test]
fn the_copy_icon_copies_the_whole_data(cx: &mut TestAppContext) {
    let big = format!("{}\nsecond line", "z".repeat(PREVIEW_CHARS + 20));
    let (win, pane, model, _server, _col) = long_event(cx);
    assert!(present(cx, win, ("event-copy", 0usize)));
    cx.update_window(win, |_, w, cx| w.click(("event-copy", 0usize), cx))
        .unwrap();
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(big));
    // The icon sits on the row, yet copying picks nothing.
    assert_eq!(pane.read_with(cx, |p, _| p.selected_event()), None);
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}

// A row click picks the event for the detail panel, which shows its whole data; the
// clipboard stays untouched.
#[gpui_kit::test]
fn clicking_a_row_selects_it_and_does_not_copy(cx: &mut TestAppContext) {
    let big = format!("{}\nsecond line", "z".repeat(PREVIEW_CHARS + 20));
    let (win, pane, model, _server, _col) = long_event(cx);
    assert!(!present(cx, win, "event-detail"));
    cx.update_window(win, |_, w, cx| w.click(("event-row", 0usize), cx))
        .unwrap();
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), None);
    assert_eq!(pane.read_with(cx, |p, _| p.selected_event()), Some(0));
    assert!(present(cx, win, "event-detail"));
    let data = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("event-detail-data").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(data.as_deref(), Some(big.as_str()));
    // The selection carries into the finished view of the same send.
    model.update(cx, |m, cx| m.stop(cx));
    finished(cx, &model);
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, cx| p.event_rows(cx).len() == 1)
    });
    assert!(present(cx, win, "event-detail"));
    // Clicking the picked row again closes the panel.
    cx.update_window(win, |_, w, cx| w.click(("event-row", 0usize), cx))
        .unwrap();
    assert_eq!(pane.read_with(cx, |p, _| p.selected_event()), None);
    assert!(!present(cx, win, "event-detail"));
}

#[test]
fn a_live_stream_stamps_each_event_with_its_arrival() {
    let mut live = LiveStream::new(200, vec![], true);
    let start = live.started();
    live.push_at(b"data: a\n\ndata: b", start + Duration::from_millis(1500));
    live.push_at(b"\n\n", start + Duration::from_millis(2250));
    let times = live.times();
    assert_eq!(times.get(0), Some(Duration::from_millis(1500)));
    assert_eq!(times.get(1), Some(Duration::from_millis(2250)));
    assert_eq!(times.get(2), None);
}

#[test]
fn time_label_reads_as_an_offset_from_the_response_start() {
    assert_eq!(time_label(None), "");
    assert_eq!(time_label(Some(Duration::from_millis(1500))), "+1.500s");
    assert_eq!(time_label(Some(Duration::from_millis(62_345))), "+1:02.345");
    assert_eq!(time_label(Some(Duration::from_secs(3723))), "+1:02:03");
}

// Each row shows when its event arrived, live and after the end of the same send.
#[gpui_kit::test]
fn rows_show_the_time_offset(cx: &mut TestAppContext) {
    let (win, pane, model, root, _server, _col) =
        open_pane(cx, Script::sse(vec![event("one"), event("two")]));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 2))
    });
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    let rows = pane.read_with(cx, |p, cx| p.event_rows(cx));
    assert!(rows.iter().all(|r| r.time.is_some()), "{rows:?}");
    let label = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("event-time", 0usize)).label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label, Some(time_label(rows[0].time)));
    model.update(cx, |m, cx| m.stop(cx));
    finished(cx, &model);
    // Once the parse of the stored body lands, the live arrivals still mark its rows.
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, cx| p.event_rows(cx).len() == 2)
    });
    cx.run_until_parked();
    let after = pane.read_with(cx, |p, cx| p.event_rows(cx));
    assert_eq!(
        after.iter().map(|r| r.time).collect::<Vec<_>>(),
        rows.iter().map(|r| r.time).collect::<Vec<_>>()
    );
}

// Typing in the filter narrows the list once typing settles, matching the name or the data.
#[gpui_kit::test]
fn the_filter_narrows_rows(cx: &mut TestAppContext) {
    let steps = vec![
        event("one"),
        Step::Chunk("event: tick\nid: 2\ndata: two\n\n".into()),
        event("three"),
    ];
    let (win, pane, model, root, _server, _col) = open_pane(cx, Script::sse(steps));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 3))
    });
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    assert!(present(cx, win, "events-filter"));
    replace_input(cx, win, None, "events-filter", "TICK");
    assert_eq!(pane.read_with(cx, |p, cx| p.event_rows(cx).len()), 3);
    cx.executor().advance_clock(apinata_app::body_view::SETTLE);
    cx.run_until_parked();
    let rows = pane.read_with(cx, |p, cx| p.event_rows(cx));
    assert_eq!(
        rows.iter().map(|r| r.preview.as_str()).collect::<Vec<_>>(),
        ["two"]
    );
    assert!(present(cx, win, ("event-row", 0usize)));
    assert!(!present(cx, win, ("event-row", 1usize)));
    replace_input(cx, win, None, "events-filter", "thr");
    cx.executor().advance_clock(apinata_app::body_view::SETTLE);
    cx.run_until_parked();
    let rows = pane.read_with(cx, |p, cx| p.event_rows(cx));
    assert_eq!(
        rows.iter().map(|r| r.preview.as_str()).collect::<Vec<_>>(),
        ["three"]
    );
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
