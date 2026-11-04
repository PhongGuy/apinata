//! Ctrl+F finds in the response body and still reaches the sidebar filter and Preferences.

use crate::support;

use apinata_app::body_view::{BodyView, Shown};
use apinata_app::request_editor::SendRequest;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext,
    WindowOptions,
};
use support::*;
use wiremock::ResponseTemplate;

struct Responded {
    s: Started,
    body: Entity<BodyView>,
    _server: wiremock::MockServer,
    _col: tempfile::TempDir,
}

fn responded(cx: &mut TestAppContext) -> Responded {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_body_raw("{\"a\":\"x\"}", "application/json"),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).editor(tab).cloned())
        .unwrap();
    let body = editor.read_with(cx, |e, cx| e.response().read(cx).body().clone());
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    wait_real(cx, |cx| {
        body.read_with(cx, |b, _| b.shown()) == Shown::Editor
    });
    frame(cx, s.win);
    Responded {
        s,
        body,
        _server: server,
        _col: col,
    }
}

fn focus_body(cx: &mut TestAppContext, r: &Responded) {
    let ed = r.body.read_with(cx, |b, _| b.editor().clone());
    cx.update_window(r.s.win, |_, w, cx| {
        ed.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
}

fn press_in_body(cx: &mut TestAppContext, r: &Responded, ks: &str) {
    focus_body(cx, r);
    let mut v = VisualTestContext::from_window(r.s.win, cx);
    v.simulate_keystrokes(ks);
    v.run_until_parked();
    frame(cx, r.s.win);
}

fn find_open(cx: &mut TestAppContext, body: &Entity<BodyView>) -> bool {
    body.read_with(cx, |b, _| b.find_open())
}

fn focused(cx: &mut TestAppContext, win: AnyWindowHandle, h: &gpui_kit::FocusHandle) -> bool {
    cx.update_window(win, |_, w, _| h.is_focused(w)).unwrap()
}

fn filter_focused(cx: &mut TestAppContext, r: &Responded) -> bool {
    let sidebar = r.s.shell.read_with(cx, |s, _| s.sidebar().clone());
    let h = sidebar.read_with(cx, |s, cx| s.filter().focus_handle(cx));
    focused(cx, r.s.win, &h)
}

#[gpui_kit::test]
fn ctrl_f_in_the_body_opens_the_find_bar_on_its_field(cx: &mut TestAppContext) {
    let r = responded(cx);
    assert!(!find_open(cx, &r.body));
    let replace = if cfg!(target_os = "macos") {
        "cmd-shift-f"
    } else {
        "ctrl-h"
    };
    press_in_body(cx, &r, replace);
    let ed = r.body.read_with(cx, |b, _| b.editor().clone());
    assert!(
        !ed.read_with(cx, |e, _| e.search_session().open),
        "no kit panel"
    );
    press_in_body(cx, &r, &secondary("f"));
    assert!(find_open(cx, &r.body));
    assert!(present(cx, r.s.win, "body-find"));
    let field = r
        .body
        .read_with(cx, |b, cx| b.find_input().focus_handle(cx));
    assert!(focused(cx, r.s.win, &field));
    assert!(
        !ed.read_with(cx, |e, _| e.search_session().open),
        "no kit panel"
    );
    assert!(!filter_focused(cx, &r));
}

#[gpui_kit::test]
fn escape_closes_the_find_bar_and_refocuses_the_body(cx: &mut TestAppContext) {
    let r = responded(cx);
    press_in_body(cx, &r, &secondary("f"));
    cx.simulate_keystrokes(r.s.win, "escape");
    frame(cx, r.s.win);
    assert!(!find_open(cx, &r.body));
    assert!(!present(cx, r.s.win, "body-find"));
    let ed = r
        .body
        .read_with(cx, |b, cx| b.editor().read(cx).focus_handle(cx));
    assert!(focused(cx, r.s.win, &ed));
    press_in_body(cx, &r, &secondary("f"));
    cx.update_window(r.s.win, |_, w, cx| w.click("body-find-close", cx))
        .unwrap();
    frame(cx, r.s.win);
    assert!(!find_open(cx, &r.body));
}

#[gpui_kit::test]
fn ctrl_f_in_the_sidebar_still_focuses_its_filter(cx: &mut TestAppContext) {
    let r = responded(cx);
    let sidebar = r.s.shell.read_with(cx, |s, _| s.sidebar().clone());
    let tree = sidebar.read_with(cx, |s, _| s.tree().clone());
    cx.update_window(r.s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    cx.simulate_keystrokes(r.s.win, &secondary("f"));
    frame(cx, r.s.win);
    assert!(filter_focused(cx, &r));
    assert!(!find_open(cx, &r.body));
}

#[gpui_kit::test]
fn ctrl_f_in_preferences_still_focuses_its_search(cx: &mut TestAppContext) {
    let r = responded(cx);
    focus_body(cx, &r);
    let view = open_prefs(cx, &r.s);
    cx.simulate_keystrokes(r.s.win, &secondary("f"));
    frame(cx, r.s.win);
    let search = view.read_with(cx, |v, cx| v.search().focus_handle(cx));
    assert!(focused(cx, r.s.win, &search));
    assert!(!find_open(cx, &r.body));
}

#[gpui_kit::test]
fn a_rebound_find_leaves_ctrl_f_to_the_shell(cx: &mut TestAppContext) {
    let r = responded(cx);
    edit_keys(cx, |o| {
        o.insert(
            "view.find_in_response".into(),
            vec!["secondary-shift-f".into()],
        );
    });
    press_in_body(cx, &r, &secondary("f"));
    assert!(!find_open(cx, &r.body));
    assert!(filter_focused(cx, &r));
    press_in_body(cx, &r, &secondary("shift-f"));
    assert!(find_open(cx, &r.body));
}

#[gpui_kit::test]
fn ctrl_f_reaches_long_body_rows_after_a_click(cx: &mut TestAppContext) {
    let body = vec!["line"; 30_000].join("\n");
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string(body), "");
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Lines(30_000));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-lines-frame", cx))
        .unwrap();
    cx.simulate_keystrokes(win, &secondary("f"));
    frame(cx, win);
    assert!(find_open(cx, &view));
}

fn next_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-g"
    } else {
        "f3"
    }
}

fn prev_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-g"
    } else {
        "shift-f3"
    }
}

fn query(cx: &mut TestAppContext, r: &Responded, q: &str) {
    cx.update_window(r.s.win, |_, w, cx| {
        r.body.update(cx, |v, cx| {
            v.open_find(w, cx);
            v.set_find_query(q, w, cx);
        })
    })
    .unwrap();
    frame(cx, r.s.win);
}

fn label(cx: &mut TestAppContext, body: &Entity<BodyView>) -> String {
    body.read_with(cx, |b, cx| b.find_label(cx))
}

#[gpui_kit::test]
fn the_find_keys_step_matches_from_the_body_and_from_the_field(cx: &mut TestAppContext) {
    let r = responded(cx);
    query(cx, &r, "\"");
    let first = label(cx, &r.body);
    assert!(first.ends_with("of 4"), "{first}");
    press_in_body(cx, &r, next_key());
    let second = label(cx, &r.body);
    assert_ne!(second, first);
    press_in_body(cx, &r, prev_key());
    assert_eq!(label(cx, &r.body), first);
    // The field has focus after typing; the keys act there too.
    let field = r.body.read_with(cx, |b, _| b.find_input().clone());
    cx.update_window(r.s.win, |_, w, cx| field.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    let mut v = VisualTestContext::from_window(r.s.win, cx);
    v.simulate_keystrokes(next_key());
    v.run_until_parked();
    assert_eq!(label(cx, &r.body), second);
}

#[gpui_kit::test]
fn the_find_keys_do_nothing_without_a_query(cx: &mut TestAppContext) {
    let r = responded(cx);
    press_in_body(cx, &r, next_key());
    assert!(!find_open(cx, &r.body));
    query(cx, &r, "");
    press_in_body(cx, &r, next_key());
    assert_eq!(label(cx, &r.body), "");
}

#[gpui_kit::test]
fn the_find_keys_step_rows_after_a_click(cx: &mut TestAppContext) {
    let body = vec!["line"; 30_000].join("\n");
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string(body), "");
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.open_find(w, cx);
            v.set_find_query("line", w, cx);
        })
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("body-lines-frame", cx))
        .unwrap();
    let first = label(cx, &view);
    cx.simulate_keystrokes(win, next_key());
    frame(cx, win);
    let second = label(cx, &view);
    assert_ne!(second, first);
    cx.simulate_keystrokes(win, prev_key());
    frame(cx, win);
    assert_eq!(label(cx, &view), first);
}
