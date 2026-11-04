//! ctrl+enter sends the active tab's request wherever focus is in the shell.

use crate::support;

use apinata_app::tabs::TabKind;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use support::*;

fn focus_shell(cx: &mut TestAppContext, s: &Started) {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
}

#[gpui_kit::test]
fn ctrl_enter_sends_from_the_sidebar_tree(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    cx.simulate_keystrokes(s.win, &key("enter"));
    wait_real(cx, |cx| !served(cx, &server).is_empty());
    assert_eq!(served(cx, &server), ["/ok"]);
}

#[gpui_kit::test]
fn ctrl_enter_and_the_palette_send_from_the_shell_root(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("enter"));
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.input("send request", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    wait_real(cx, |cx| served(cx, &server).len() == 2);
}

#[gpui_kit::test]
fn without_a_request_tab_ctrl_enter_sends_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Collection, cx)
    });
    frame(cx, s.win);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("enter"));
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert!(served(cx, &server).is_empty());
}

// A request tab that is open but not active must not send.
#[gpui_kit::test]
fn ctrl_enter_sends_nothing_when_a_settings_tab_is_active(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Collection, cx)
    });
    frame(cx, s.win);
    focus_shell(cx, &s);
    cx.simulate_keystrokes(s.win, &key("enter"));
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert!(served(cx, &server).is_empty());
}
