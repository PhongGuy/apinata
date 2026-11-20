//! A running request cancels from the keyboard: the send chord again, or Escape when nothing
//! nearer takes it.

use crate::support::sse_server::{Script, event};
use crate::support::*;

use apinata_app::commands::{Scope, registry};
use apinata_app::palette::{Availability, palette_items_for};
use apinata_app::request_editor::{CancelRequest, RUNNING_KEY_CONTEXT, RequestEditor, SendRequest};
use apinata_app::shortcuts::HOST_OS;
use apinata_core::{Outcome, StreamEnd};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    Action as _, AnyWindowHandle, AppContext as _, AsKeystroke as _, Entity, Focusable as _,
    KeyContext, TestAppContext,
};
use std::time::Duration;
use wiremock::ResponseTemplate;

struct Running {
    server: wiremock::MockServer,
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
}

// A request whose server holds the answer back, sent and seen arriving at the server.
fn running(cx: &mut TestAppContext) -> Running {
    http_ready(cx);
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(30));
    let server = mock_server(cx, slow);
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    frame(cx, win);
    Running {
        server,
        _col: col,
        _state: state,
        win,
        ed,
    }
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    frame(cx, win);
}

fn sending(cx: &mut TestAppContext, ed: &Entity<RequestEditor>) -> bool {
    ed.read_with(cx, |e, cx| e.send_model().read(cx).is_sending())
}

fn assert_cancelled(cx: &mut TestAppContext, r: &Running) {
    let model = r.ed.read_with(cx, |e, _| e.send_model().clone());
    assert_eq!(finished(cx, &model).outcome, Outcome::Cancelled);
    frame(cx, r.win);
    assert!(present(cx, r.win, "send"), "the slot is back to Send");
    assert_eq!(served(cx, &r.server).len(), 1, "nothing was sent again");
}

#[gpui_kit::test]
fn the_send_chord_cancels_a_running_request(cx: &mut TestAppContext) {
    let r = running(cx);
    cx.update_window(r.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    press(cx, r.win, &key("enter"));
    assert_cancelled(cx, &r);
}

#[gpui_kit::test]
fn escape_in_the_url_cancels_a_running_request(cx: &mut TestAppContext) {
    let r = running(cx);
    cx.update_window(r.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    press(cx, r.win, "escape");
    assert_cancelled(cx, &r);
}

#[gpui_kit::test]
fn escape_on_the_cancel_button_cancels(cx: &mut TestAppContext) {
    let r = running(cx);
    cx.update_window(r.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    tab_to(cx, r.win, "cancel-button");
    press(cx, r.win, "escape");
    assert_cancelled(cx, &r);
}

// An open dropdown takes the first Escape to close itself; only the next one cancels.
#[gpui_kit::test]
fn escape_closes_an_open_method_list_before_it_cancels(cx: &mut TestAppContext) {
    let r = running(cx);
    let method = r.ed.read_with(cx, |e, cx| e.method_focus(cx));
    cx.update_window(r.win, |_, w, cx| method.focus(w, cx))
        .unwrap();
    press(cx, r.win, "space");
    press(cx, r.win, "escape");
    assert!(sending(cx, &r.ed), "the list took the first Escape");
    press(cx, r.win, "escape");
    assert_cancelled(cx, &r);
}

// With nothing running, Escape in the URL is not taken: the next send goes out as usual.
#[gpui_kit::test]
fn escape_while_idle_leaves_the_request_alone(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| w.click("url", cx))
        .unwrap();
    press(cx, win, "escape");
    assert!(!sending(cx, &ed));
    press(cx, win, &key("enter"));
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    assert_eq!(finished(cx, &model).outcome, Outcome::Passed);
}

// While the body streams the slot shows Stop, and the keys do what it does: end the body and
// keep what arrived.
#[gpui_kit::test]
fn the_keys_stop_a_streaming_response(cx: &mut TestAppContext) {
    for k in [key("enter"), "escape".to_string()] {
        http_ready(cx);
        let server = sse(cx, Script::sse(vec![event("one")]));
        let col = sse_collection(&server);
        let (_state, ws, root) = open_dir(cx, &col);
        let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
        cx.update_window(win, |_, w, cx| w.click("send", cx))
            .unwrap();
        let model = ed.read_with(cx, |e, _| e.send_model().clone());
        wait_real(cx, |cx| model.read_with(cx, |m, _| m.is_streaming()));
        frame(cx, win);
        cx.update_window(win, |_, w, cx| w.click("url", cx))
            .unwrap();
        press(cx, win, &k);
        let result = finished(cx, &model);
        assert_eq!(
            result.response.as_ref().unwrap().stream_end(),
            Some(&StreamEnd::Stopped),
            "{k}"
        );
    }
}

#[gpui_kit::test]
fn the_cancel_tooltip_names_escape(cx: &mut TestAppContext) {
    let r = running(cx);
    // The context the tooltip looks its chord up in; the kit shows that binding's first key.
    let context = KeyContext::parse(RUNNING_KEY_CONTEXT).expect("a key context");
    let keys = cx
        .update_window(r.win, |_, w, _| {
            w.highest_precedence_binding_for_action_in_context(&CancelRequest, context)
                .and_then(|b| {
                    b.keystrokes()
                        .first()
                        .map(|k| k.as_keystroke().unparse(HOST_OS))
                })
        })
        .unwrap();
    assert_eq!(keys.as_deref(), Some("escape"));
    press(cx, r.win, "escape");
}

#[test]
fn cancel_request_is_a_command_on_escape_while_a_request_runs() {
    let cmds = registry();
    let c = cmds
        .iter()
        .find(|c| c.id == "request.cancel")
        .expect("registered");
    assert_eq!(c.label, "Cancel request");
    assert_eq!(c.default_chords, ["escape"]);
    assert_eq!(c.scope, Scope::Editor("RunningRequest"));
    assert!(c.in_palette && !c.opens_dialog);
}

#[test]
fn the_palette_offers_cancel_only_while_a_request_runs() {
    let listed = |a: Availability| {
        palette_items_for(a)
            .iter()
            .any(|e| e.action.name() == CancelRequest.name())
    };
    assert!(listed(Availability::all()));
    assert!(!listed(Availability {
        cancel: false,
        ..Availability::all()
    }));
}

// From the palette, with focus outside the editor, the shell cancels the active tab's request.
#[gpui_kit::test]
fn the_palette_cancels_the_active_request(cx: &mut TestAppContext) {
    http_ready(cx);
    let slow = ResponseTemplate::new(200).set_delay(Duration::from_secs(30));
    let server = mock_server(cx, slow);
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).editor(tab).cloned())
        .expect("a request editor");
    cx.update_window(s.win, |_, w, cx| {
        ed.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("cancel request", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    assert_eq!(finished(cx, &model).outcome, Outcome::Cancelled);
}
