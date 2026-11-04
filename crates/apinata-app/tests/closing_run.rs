use crate::support;

use apinata_app::closing::{CloseScope, confirm_close_or_keep};
use apinata_app::runner_model::RunnerModel;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use std::time::Duration;
use support::*;
use wiremock::ResponseTemplate;

fn close_window(cx: &mut TestAppContext, s: &Started) -> bool {
    VisualTestContext::from_window(s.win, cx).simulate_close()
}

// Starts a runner pass against a server that answers after 30 seconds and waits until the
// run is under way.
fn run_in_flight(cx: &mut TestAppContext, s: &Started) -> Entity<RunnerModel> {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("run-start", cx))
        .unwrap();
    cx.run_until_parked();
    let model = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view")
        .read_with(cx, |v, _| v.model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_running()));
    model
}

fn slow_run(cx: &mut TestAppContext) -> (Started, Entity<RunnerModel>) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = run_in_flight(cx, &s);
    // The server and collection outlive the test through the started window's own handles.
    std::mem::forget((server, col));
    (s, model)
}

#[gpui_kit::test]
fn closing_with_a_run_in_flight_asks_first(cx: &mut TestAppContext) {
    let (s, model) = slow_run(cx);
    assert!(!close_window(cx, &s), "the close is held for the prompt");
    assert!(confirm_open(cx));
    assert!(
        !close_window(cx, &s),
        "a second request opens no second prompt"
    );
    answer_confirm(cx, "confirm-cancel");
    assert!(!confirm_open(cx));
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    assert!(
        model.read_with(cx, |m, _| m.is_running()),
        "the run goes on"
    );
}

#[gpui_kit::test]
fn stop_and_close_cancels_the_run_and_closes(cx: &mut TestAppContext) {
    let (s, model) = slow_run(cx);
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-stop");
    assert_eq!(cx.update(|cx| cx.windows().len()), 0);
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_running()));
}

#[gpui_kit::test]
fn an_idle_runner_closes_without_a_prompt(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection_for("http://127.0.0.1:9", "");
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx));
    assert!(close_window(cx, &s));
    assert!(!confirm_open(cx));
}

#[gpui_kit::test]
fn unsaved_changes_are_asked_before_the_run(cx: &mut TestAppContext) {
    let (s, model) = slow_run(cx);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/9"), cx)
    });
    cx.run_until_parked();
    assert!(!close_window(cx, &s));
    answer_confirm(cx, "confirm-discard");
    assert!(confirm_open(cx), "the run prompt follows the save prompt");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    assert!(model.read_with(cx, |m, _| m.is_running()));
}

#[gpui_kit::test]
fn cancel_on_the_run_prompt_reports_the_window_kept(cx: &mut TestAppContext) {
    let (s, _model) = slow_run(cx);
    let kept = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = kept.clone();
    let (ws, win) = (s.ws.clone(), s.win);
    cx.update_window(win, |_, window, cx| {
        confirm_close_or_keep(ws, CloseScope::Window, window, cx, move |_| flag.set(true));
    })
    .unwrap();
    assert!(confirm_open(cx));
    assert!(!kept.get(), "the prompt is still open");
    answer_confirm(cx, "confirm-cancel");
    assert!(kept.get());
}
