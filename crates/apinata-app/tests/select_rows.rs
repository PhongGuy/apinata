//! Every select's dropdown rows carry `{select}-option-{key}` ids, and a click on one picks it.

use crate::support;

use apinata_app::auth_section::{AuthMode, AuthSection};
use apinata_app::body_section::{BodyMode, BodySection};
use apinata_app::generate_code::current;
use apinata_core::{AuthSpec, BodySpec, Target};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Focusable as _, TestAppContext};
use std::path::PathBuf;
use support::*;

// Opens the select `select`, checks each of `rows` is listed, then clicks `pick`.
fn open_and_pick(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    select: &str,
    rows: &[&str],
    pick: &str,
) {
    assert!(
        !present(cx, win, pick.to_owned()),
        "a closed {select} lists nothing"
    );
    cx.update_window(win, |_, w, cx| w.click(select.to_owned(), cx))
        .unwrap();
    frame(cx, win);
    for id in rows {
        assert!(present(cx, win, id.to_string()), "{id} is in the tree");
    }
    cx.update_window(win, |_, w, cx| w.click(pick.to_owned(), cx))
        .unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn method_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x.test/ok")),
    ]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = editor_of(cx, &s, tab);
    open_and_pick(
        cx,
        s.win,
        "method",
        &["method-option-GET", "method-option-DELETE"],
        "method-option-POST",
    );
    let method = editor.read_with(cx, |e, cx| e.method(cx));
    assert_eq!(method.as_deref(), Some("POST"));
}

#[gpui_kit::test]
fn auth_mode_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, AuthSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&AuthSpec::None, w, cx))
    })
    .unwrap();
    frame(cx, win);
    open_and_pick(
        cx,
        win,
        "auth-mode",
        &["auth-mode-option-none", "auth-mode-option-oauth2"],
        "auth-mode-option-bearer",
    );
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(AuthMode::Bearer));
}

#[gpui_kit::test]
fn body_mode_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| BodySection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&BodySpec::None, w, cx))
    })
    .unwrap();
    frame(cx, win);
    open_and_pick(
        cx,
        win,
        "body-mode",
        &["body-mode-option-none", "body-mode-option-xml"],
        "body-mode-option-json",
    );
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(BodyMode::Json));
}

#[gpui_kit::test]
fn codegen_target_rows_have_ids_and_a_click_picks_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x.test/ok")),
    ]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("generate code", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
    open_and_pick(
        cx,
        s.win,
        "generate-code-target",
        &[
            "generate-code-target-option-curl",
            "generate-code-target-option-go",
        ],
        "generate-code-target-option-httpie",
    );
    let target = cx.update(|cx| current(cx).map(|v| v.read(cx).target()));
    assert_eq!(target, Some(Target::Httpie));
}
