//! The Script section shows one phase at a time under a Pre-request / Post-response switch.

use crate::support;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_app::script_section::{ScriptSection, default_side};
use apinata_app::snapshot::ScriptCode;
use apinata_core::ScriptPhase::{PostResponse, PreRequest, Tests};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

fn loaded(
    cx: &mut TestAppContext,
    pre: &str,
    post: &str,
) -> (AnyWindowHandle, Entity<ScriptSection>) {
    let (win, host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse], w, cx)
    });
    let s = host.read_with(cx, |h, _| h.view.clone());
    let code = ScriptCode {
        pre_request: pre.into(),
        post_response: post.into(),
        tests: String::new(),
    };
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&code, w, cx)))
        .unwrap();
    frame(cx, win);
    (win, s)
}

fn click_tab(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) {
    cx.update_window(win, |_, w, cx| w.click(("script-tab", ix), cx))
        .unwrap();
    frame(cx, win);
}

#[test]
fn the_side_with_the_only_script_opens() {
    assert_eq!(default_side("", ""), PreRequest);
    assert_eq!(default_side("a()", ""), PreRequest);
    assert_eq!(default_side("a()", "b()"), PreRequest);
    assert_eq!(default_side("", "b()"), PostResponse);
    assert_eq!(default_side(" \n", "b()"), PostResponse);
}

#[gpui_kit::test]
fn one_editor_shows_under_the_switch(cx: &mut TestAppContext) {
    let (win, s) = loaded(cx, "", "b()");
    assert_eq!(s.read_with(cx, |s, _| s.shown()), Some(PostResponse));
    assert!(present(cx, win, "script-phase"));
    assert!(present(cx, win, "script-post-response-code"));
    assert!(!present(cx, win, "script-pre-request-code"));
    click_tab(cx, win, 0);
    assert_eq!(s.read_with(cx, |s, _| s.shown()), Some(PreRequest));
    assert!(present(cx, win, "script-pre-request-code"));
    assert!(!present(cx, win, "script-post-response-code"));
}

#[gpui_kit::test]
fn a_dot_marks_each_side_with_a_script(cx: &mut TestAppContext) {
    let (win, _s) = loaded(cx, "a()", "  ");
    assert!(present(cx, win, ("script-dot", 0usize)));
    assert!(
        !present(cx, win, ("script-dot", 1usize)),
        "whitespace is no script"
    );
}

#[gpui_kit::test]
fn the_tests_section_has_no_switch(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| ScriptSection::new(&[Tests], w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    assert_eq!(s.read_with(cx, |s, _| s.shown()), None);
    assert!(!present(cx, win, "script-phase"));
    assert!(present(cx, win, "script-tests-code"));
}

#[gpui_kit::test]
fn switching_keeps_each_editors_caret(cx: &mut TestAppContext) {
    let (win, s) = loaded(cx, "a()", "b()");
    replace_code(
        cx,
        win,
        Some("script-section"),
        "script-pre-request-code",
        "foo();",
    );
    let pre = s.read_with(cx, |s, _| s.code(PreRequest).cloned()).unwrap();
    let before = pre.read_with(cx, |c, _| c.cursor());
    click_tab(cx, win, 1);
    click_tab(cx, win, 0);
    assert_eq!(pre.read_with(cx, |c, _| c.cursor()), before);
}

#[gpui_kit::test]
fn the_chosen_side_is_kept_for_the_request(cx: &mut TestAppContext) {
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('a', 1)\"\n",
        request("a", "http://x")
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx)
    })
    .unwrap();
    frame(cx, win);
    let shown = |cx: &mut TestAppContext, ed: &Entity<RequestEditor>| {
        ed.read_with(cx, |e, cx| e.script().read(cx).shown())
    };
    assert_eq!(shown(cx, &ed), Some(PreRequest));
    click_tab(cx, win, 1);
    assert_eq!(
        ws.read_with(cx, |w, _| w.script_side(&root, &ip("a.yml"))),
        Some(PostResponse)
    );
    // A fresh editor for the same request, as when its tab is reopened.
    let tab = ws.read_with(cx, |w, _| w.tabs().active().expect("a tab").id);
    support::init(cx);
    let shared = ws.clone();
    let (_win2, ed2) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| RequestEditor::new(shared, tab, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(shown(cx, &ed2), Some(PostResponse));
}

#[gpui_kit::test]
fn closing_the_collection_forgets_its_chosen_sides(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, _| {
        w.set_script_side(&root, ip("a.yml"), PostResponse)
    });
    ws.update(cx, |w, cx| w.close(&root, cx));
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(cx, |w, _| w.script_side(&root, &ip("a.yml"))),
        None
    );
}
