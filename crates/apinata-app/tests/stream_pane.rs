use crate::support;
use crate::support::sse_server::{Script, Step, event};
use crate::support::*;
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};

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
fn a_streaming_body_shows_as_live_rows_then_the_normal_view(cx: &mut TestAppContext) {
    let (win, pane, model, root, _server, _col) =
        open_pane(cx, Script::sse(vec![event("one"), event("two")]));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 2))
    });
    assert!(present(cx, win, "response-streaming"));
    assert!(present(cx, win, "live-body"));
    assert!(!present(cx, win, "response-spinner"));
    assert!(!pane.read_with(cx, |p, cx| p.body_shown(cx)));
    assert_eq!(
        pane.read_with(cx, |p, cx| p.live_rows(cx)),
        ["data: one", "", "data: two", ""]
    );
    model.update(cx, |m, cx| m.stop(cx));
    finished(cx, &model);
    assert!(present(cx, win, "response-body"));
    assert!(!present(cx, win, "live-body"));
    assert!(pane.read_with(cx, |p, cx| p.body_shown(cx)));
    assert!(pane.read_with(cx, |p, cx| p.live_rows(cx)).is_empty());
    let tabs = pane.read_with(cx, |p, cx| p.shown_tabs(cx));
    assert!(tabs.contains(&ResponseTab::Headers), "{tabs:?}");
    assert_eq!(pane.read_with(cx, |p, _| p.tab()), ResponseTab::Body);
}

#[gpui_kit::test]
fn the_live_body_is_the_last_2000_lines(cx: &mut TestAppContext) {
    let steps: Vec<Step> = (0..2500)
        .map(|i| Step::Chunk(format!("line {i}\n")))
        .collect();
    let (win, pane, model, root, _server, _col) =
        open_pane(cx, Script::with_type("text/plain", steps));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| {
            m.live().is_some_and(|l| l.skipped_lines() == 500)
        })
    });
    assert!(present(cx, win, "live-body"));
    let rows = pane.read_with(cx, |p, cx| p.live_rows(cx));
    assert_eq!(
        (rows.len(), rows[0].as_str(), rows[1999].as_str()),
        (2000, "line 500", "line 2499")
    );
    assert_eq!(pane.read_with(cx, |p, cx| p.live_skipped(cx)), 500);
    assert!(present(cx, win, "live-note"));
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}

#[gpui_kit::test]
fn a_tab_chosen_before_the_stream_does_not_break_the_live_view(cx: &mut TestAppContext) {
    let (win, pane, model, root, _server, _col) = open_pane(
        cx,
        Script::with_type("text/plain", vec![Step::Chunk("hi\n".into())]),
    );
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    assert!(present(cx, win, "live-body"));
    assert!(!present(cx, win, "live-note"));
    assert_eq!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx)),
        [ResponseTab::Body]
    );
    assert!(present(cx, win, "response-tab-strip"));
    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}
