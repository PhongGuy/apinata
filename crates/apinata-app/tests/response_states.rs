//! The pane's idle, sending and error states, their buttons, and a spinner that holds
//! still under reduced motion.

use crate::support;

use apinata_app::response_pane::ResponsePane;
use apinata_app::response_pane::{ErrorKind, ErrorTone, ErrorView};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

fn slow() -> ResponseTemplate {
    ResponseTemplate::new(200).set_delay(Duration::from_secs(30))
}

#[gpui_kit::test]
fn sending_shows_a_still_spinner_and_cancel_shows_retry(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, slow());
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    support::init(cx);
    let m = model.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    assert!(present(cx, win, "response-idle"));
    assert!(model.read_with(cx, |m, _| m.elapsed().is_none()));
    cx.update_window(win, |_, w, _| {
        assert!(
            w.within("response-idle")
                .try_find(("empty-action", 0usize))
                .is_some()
        )
    })
    .unwrap();
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    assert!(present(cx, win, "response-spinner"));
    // The timer starts when the plan is handed to the run, not at the click.
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.elapsed().is_some()));
    assert!(present(cx, win, "response-elapsed"));
    let frames = cx
        .update_window(win, |_, w, cx| w.simulate_next_frame(cx))
        .unwrap();
    assert_eq!(frames, 0, "nothing animates under reduced motion");
    cx.update_window(win, |_, w, cx| w.click("response-cancel", cx))
        .unwrap();
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert!(present(cx, win, "response-error"));
    assert!(present(cx, win, "response-retry"));
    assert!(model.read_with(cx, |m, _| m.elapsed().is_none()));
}

#[gpui_kit::test]
fn retry_and_the_idle_send_reach_the_editor(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, slow());
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("response-cancel", cx)
    })
    .unwrap();
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("response-retry", cx)
    })
    .unwrap();
    wait_real(cx, |cx| served(cx, &server).len() == 2);
    assert!(present(cx, win, "response-sending"));
    model.update(cx, |m, cx| m.cancel(cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
}

#[gpui_kit::test]
fn a_cancel_mid_flight_is_neutral_and_has_no_tabs(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, slow());
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    support::init(cx);
    let m = model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    model.update(cx, |m, cx| m.cancel(cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert_eq!(
        view.read_with(cx, |p, cx| p.error_view(cx)),
        Some(ErrorView {
            kind: ErrorKind::Cancelled,
            title: "Cancelled",
            sentence: None,
            message: None,
            tone: ErrorTone::Neutral,
            retry: true,
            add_variable: None,
            use_environments: Vec::new(),
        })
    );
    assert!(present(cx, win, "response-error"));
    assert!(present(cx, win, "response-retry"));
    assert!(!present(cx, win, ("response-tab", 0usize)));
}

#[gpui_kit::test]
fn a_cancelled_done_result_is_neutral_and_has_no_tabs(cx: &mut TestAppContext) {
    // A cancel that lands before the run polls ends as Done with Skipped and no response.
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let mut skipped = (*s.result).clone();
    skipped.outcome = apinata_core::Outcome::Skipped;
    skipped.response = None;
    s.model
        .update(cx, |m, cx| m.show(std::sync::Arc::new(skipped), cx));
    support::init(cx);
    let m = s.model.clone();
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
        .expect("an error");
    assert_eq!((shown.title, shown.tone), ("Cancelled", ErrorTone::Neutral));
    assert!(present(cx, win, "response-error"));
    assert!(!present(cx, win, ("response-tab", 0usize)));
}

#[gpui_kit::test]
fn an_undefined_variable_offers_no_retry_and_no_tabs(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{nope}}")),
    ]);
    http_ready(cx);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
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
        .expect("an error");
    assert_eq!(
        (shown.title, shown.tone, shown.retry),
        ("Undefined variable", ErrorTone::Danger, false)
    );
    assert!(shown.message.unwrap().contains("nope"));
    assert!(present(cx, win, "response-error"));
    assert!(!present(cx, win, "response-retry"));
    assert!(!present(cx, win, ("response-tab", 0usize)));
}
