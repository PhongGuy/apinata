use crate::support::sse_server::{Script, event};
use crate::support::*;
use apinata_app::commands::registry;
use apinata_app::send_model::SendState;
use apinata_app::tab_area::StopActive;
use apinata_core::{Outcome, StreamEnd};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Action as _, AppContext as _, TestAppContext};
use std::time::Duration;

#[gpui_kit::test]
fn send_becomes_cancel_before_headers_and_stop_after(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(
        cx,
        Script::sse(vec![event("one")]).head_delay(Duration::from_millis(600)),
    );
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    assert!(present(cx, win, "cancel"));
    assert!(!present(cx, win, "stop"));
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    assert!(present(cx, win, "stop"));
    assert!(!present(cx, win, "cancel"));
    assert!(!present(cx, win, "send"));
    // The slot keeps its width, so the URL does not jump.
    cx.update_window(win, |_, w, _| {
        let slot = w.find("send-slot").bounds();
        let stop = w.find("stop").bounds();
        assert!(stop.left() >= slot.left() && stop.right() <= slot.right());
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| w.click("stop-button", cx))
        .unwrap();
    let result = finished(cx, &model);
    assert_eq!(
        result.response.as_ref().unwrap().stream_end(),
        Some(&StreamEnd::Stopped)
    );
    assert!(present(cx, win, "send"));
}

#[gpui_kit::test]
fn send_and_stop_share_one_width_across_a_stream(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![event("one")]));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    // The clickable buttons themselves, not the wrappers around them.
    let send = cx
        .update_window(win, |_, w, _| w.find("send-button").bounds())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    let (stop, spinner) = cx
        .update_window(win, |_, w, _| {
            (
                w.find("stop-button").bounds(),
                w.find("stop-spinner").bounds(),
            )
        })
        .unwrap();
    assert_eq!(stop.size.width, send.size.width);
    assert_eq!(stop.left(), send.left());
    // The spinner is drawn inside the Stop button, not beside it.
    assert!(spinner.left() >= stop.left() && spinner.right() <= stop.right());
    assert!(spinner.top() >= stop.top() && spinner.bottom() <= stop.bottom());
    cx.update_window(win, |_, w, cx| w.click("stop-button", cx))
        .unwrap();
    finished(cx, &model);
    let again = cx
        .update_window(win, |_, w, _| w.find("send-button").bounds())
        .unwrap();
    assert_eq!(again.size.width, send.size.width);
    assert_eq!(again.left(), send.left());
}

#[gpui_kit::test]
fn cancel_shares_the_send_width_and_left_edge(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(
        cx,
        Script::sse(vec![event("one")]).head_delay(Duration::from_secs(30)),
    );
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let send = cx
        .update_window(win, |_, w, _| w.find("send-button").bounds())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_sending()));
    let (cancel, spinner) = cx
        .update_window(win, |_, w, _| {
            (
                w.find("cancel-button").bounds(),
                w.find("cancel-spinner").bounds(),
            )
        })
        .unwrap();
    assert_eq!(cancel.size.width, send.size.width);
    assert_eq!(cancel.left(), send.left());
    assert!(spinner.left() >= cancel.left() && spinner.right() <= cancel.right());
    assert!(spinner.top() >= cancel.top() && spinner.bottom() <= cancel.bottom());
    // Headers are held for 30 s, so the send is still pending; cancel it so it does not linger.
    cx.update_window(win, |_, w, cx| w.click("cancel-button", cx))
        .unwrap();
    finished(cx, &model);
}

#[gpui_kit::test]
fn cancel_before_headers_keeps_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![]).head_delay(Duration::from_secs(5)));
    let col = sse_collection(&server);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |_| server.requests() == 1);
    assert!(present(cx, win, "cancel"));
    cx.update_window(win, |_, w, cx| w.click("cancel", cx))
        .unwrap();
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Cancelled);
    assert!(result.response.is_none());
}

#[test]
fn request_stop_is_registered_unbound_and_listed_after_send() {
    let cmds = registry();
    let ix = cmds
        .iter()
        .position(|c| c.id == "request.stop")
        .expect("registered");
    assert_eq!(cmds[ix - 1].id, "request.send");
    let c = &cmds[ix];
    assert_eq!(c.label, "Stop response");
    assert!(c.default_chords.is_empty());
    assert!(c.in_palette);
    assert_eq!(c.targets[0].action.name(), "apinata::StopActive");
}

#[gpui_kit::test]
fn the_stop_command_acts_only_while_a_response_is_open(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = sse(cx, Script::sse(vec![event("one")]));
    let col = sse_collection(&server);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let ed = area.read_with(cx, |a, _| a.editor(tab).cloned().expect("an editor"));
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    let stop = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_action(StopActive.boxed_clone(), cx)
        })
        .unwrap();
        cx.run_until_parked();
    };
    stop(cx);
    assert!(
        matches!(
            model.read_with(cx, |m, _| m.state().clone()),
            SendState::Idle
        ),
        "idle: nothing to stop"
    );
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
    stop(cx);
    let result = finished(cx, &model);
    assert_eq!(
        result.response.as_ref().unwrap().stream_end(),
        Some(&StreamEnd::Stopped)
    );
}
