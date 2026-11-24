//! One Scripts tab holds every script slot of a request: Pre-request on one sub-tab, and the
//! post-response script with the tests under it on the other.

use crate::support;

use apinata_app::request_editor::Section;
use apinata_app::script_section::{ScriptErrorNote, ScriptSection};
use apinata_app::snapshot::ScriptCode;
use apinata_core::ScriptPhase::{PostResponse, PreRequest, Tests};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

const TESTS_ONLY: &str =
    "runtime:\n  scripts:\n    - type: tests\n      code: \"test('ok', function () {})\"\n";

fn all_phases(
    cx: &mut TestAppContext,
    code: ScriptCode,
) -> (AnyWindowHandle, Entity<ScriptSection>) {
    let (win, host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse, Tests], w, cx)
    });
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&code, w, cx)))
        .unwrap();
    frame(cx, win);
    (win, s)
}

fn code(pre: &str, post: &str, tests: &str) -> ScriptCode {
    ScriptCode {
        pre_request: pre.into(),
        post_response: post.into(),
        tests: tests.into(),
    }
}

fn click_tab(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) {
    cx.update_window(win, |_, w, cx| w.click(("script-tab", ix), cx))
        .unwrap();
    frame(cx, win);
}

#[test]
fn the_strip_has_one_scripts_tab_and_no_tests_tab() {
    let labels: Vec<&str> = Section::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(
        labels,
        [
            "Params",
            "Body",
            "Headers",
            "Auth",
            "Vars",
            "Scripts",
            "Assertions",
            "Docs",
            "Settings"
        ]
    );
}

#[gpui_kit::test]
fn post_response_holds_the_tests_editor_under_its_script(cx: &mut TestAppContext) {
    let (win, s) = all_phases(cx, code("a()", "", ""));
    assert_eq!(s.read_with(cx, |s, _| s.shown()), Some(PreRequest));
    assert!(present(cx, win, "script-pre-request-code"));
    assert!(!present(cx, win, "script-tests-code"));
    assert!(!present(cx, win, ("script-tab", 2usize)), "two sub-tabs");
    click_tab(cx, win, 1);
    assert_eq!(s.read_with(cx, |s, _| s.shown()), Some(PostResponse));
    assert!(present(cx, win, "script-post-response-code"));
    assert!(present(cx, win, "script-tests-code"));
    assert!(!present(cx, win, "script-pre-request-code"));
}

#[gpui_kit::test]
fn tests_alone_open_on_post_response_with_its_dot(cx: &mut TestAppContext) {
    let (win, s) = all_phases(cx, code("", "", "test('a', () => {})"));
    assert_eq!(s.read_with(cx, |s, _| s.shown()), Some(PostResponse));
    assert!(present(cx, win, ("script-dot", 1usize)));
    assert!(!present(cx, win, ("script-dot", 0usize)));
}

#[gpui_kit::test]
fn a_snippet_on_post_response_goes_to_the_editor_whose_row_was_pressed(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx)
    })
    .unwrap();
    frame(cx, win);
    click_tab(cx, win, 1);
    // The same snippet is offered for both editors here; each row is its own scope.
    cx.update_window(win, |_, w, cx| {
        w.within("script-tests").click("snippet-var-from-body", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let texts = ed.read_with(cx, |e, cx| {
        let s = e.script().read(cx);
        let text = |p| s.code(p).unwrap().read(cx).value().to_string();
        (text(PostResponse), text(Tests))
    });
    assert_eq!(texts.0, "");
    assert!(texts.1.contains("bru.setVar(\"id\""), "{texts:?}");
}

#[gpui_kit::test]
fn a_tests_error_is_marked_on_post_response_and_go_to_line_reaches_it(cx: &mut TestAppContext) {
    let (win, s) = all_phases(cx, code("a()", "", "let x = 1;\nthrow 1;"));
    let note = ScriptErrorNote {
        phase: Tests,
        text: "Test script error, this request's tests, line 2: Error: y".into(),
        line: Some(2),
    };
    s.update(cx, |s, cx| s.set_errors(&[note], cx));
    frame(cx, win);
    assert!(!present(cx, win, "script-error"), "pre-request shows none");
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.go_to_error(Tests, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(s.read_with(cx, |s, _| s.shown()), Some(PostResponse));
    let banner = cx
        .update_window(win, |_, w, _| {
            w.within("script-tests").try_find("script-error").is_some()
        })
        .unwrap();
    assert!(banner, "the banner sits over the tests editor");
    let row = s.read_with(cx, |s, cx| {
        s.code(Tests).unwrap().read(cx).cursor_position().line
    });
    assert_eq!(row, 1);
}

#[gpui_kit::test]
fn a_request_with_only_tests_marks_the_scripts_tab(cx: &mut TestAppContext) {
    let a = format!("{}{TESTS_ONLY}", request("a", "http://x"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    assert!(ed.read_with(cx, |e, cx| e.section_marked(Section::Script, cx)));
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx)
    })
    .unwrap();
    frame(cx, win);
    assert!(present(cx, win, "script-tests-code"));
}
