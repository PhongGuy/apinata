//! A script error from the last run, marked in the editor of the script that failed: a
//! banner above it with Go to line, and the failing line underlined.

use crate::support;

use apinata_app::request_editor::Section;
use apinata_app::script_section::{ScriptErrorNote, ScriptSection, script_error_notes};
use apinata_core::ScriptPhase::{PostResponse, PreRequest, Tests};
use apinata_core::{Outcome, RequestResult, RunError, TestOutcome, Verdict};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::sync::Arc;
use support::*;

fn result() -> RequestResult {
    RequestResult::from_parts(ip("a.yml"), "A", Outcome::Errored, None, Vec::new())
}

fn script_error(phase: apinata_core::ScriptPhase, name: &str, message: &str) -> TestOutcome {
    let mut t = TestOutcome::new(phase, name, Verdict::fail("", Some(message.into())));
    t.script_error = true;
    t
}

fn note(phase: apinata_core::ScriptPhase, text: &str, line: Option<u32>) -> ScriptErrorNote {
    ScriptErrorNote {
        phase,
        text: text.into(),
        line,
    }
}

#[test]
fn a_pre_request_error_names_its_phase_and_line() {
    let mut r = result();
    r.error = Some(RunError::Script {
        phase: PreRequest,
        message: "this request's script, line 2: Error: stop".into(),
    });
    assert_eq!(
        script_error_notes(&r),
        [note(
            PreRequest,
            "Pre-request script error, this request's script, line 2: Error: stop",
            Some(2)
        )]
    );
}

#[test]
fn later_phase_errors_come_from_their_entries_and_tests_are_not_errors() {
    let mut r = result();
    r.tests = vec![
        script_error(
            PostResponse,
            "Post-response script error",
            "folder f script, line 4: Error: x",
        ),
        TestOutcome::new(Tests, "t", Verdict::fail("", Some("no".into()))),
        script_error(
            Tests,
            "Test script error",
            "this request's tests, line 3: Error: y",
        ),
    ];
    assert_eq!(
        script_error_notes(&r),
        [
            note(
                PostResponse,
                "Post-response script error, folder f script, line 4: Error: x",
                None
            ),
            note(
                Tests,
                "Test script error, this request's tests, line 3: Error: y",
                Some(3)
            ),
        ]
    );
}

fn section(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ScriptSection>) {
    let (win, host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse], w, cx)
    });
    let s = host.read_with(cx, |h, _| h.view.clone());
    let code = apinata_app::snapshot::ScriptCode {
        pre_request: "let a = 1;\nlet b = 2;\nthrow new Error('stop');".into(),
        post_response: String::new(),
        tests: String::new(),
    };
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&code, w, cx)))
        .unwrap();
    frame(cx, win);
    (win, s)
}

fn banner_label(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, _| {
        w.try_find("script-error")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn underlined(cx: &mut TestAppContext, s: &Entity<ScriptSection>) -> usize {
    s.read_with(cx, |s, cx| {
        let code = s.code(PreRequest).unwrap().read(cx);
        code.diagnostics().map_or(0, |d| d.len())
    })
}

#[gpui_kit::test]
fn the_failing_phase_shows_a_banner_and_underlines_its_line(cx: &mut TestAppContext) {
    let (win, s) = section(cx);
    let text = "Pre-request script error, this request's script, line 3: Error: stop";
    s.update(cx, |s, cx| {
        s.set_errors(&[note(PreRequest, text, Some(3))], cx)
    });
    frame(cx, win);
    assert_eq!(banner_label(cx, win).as_deref(), Some(text));
    assert_eq!(underlined(cx, &s), 1);
    // The other phase's editor shows nothing.
    cx.update_window(win, |_, w, cx| w.click(("script-tab", 1usize), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(banner_label(cx, win), None);
}

#[gpui_kit::test]
fn go_to_line_puts_the_caret_on_the_failing_line(cx: &mut TestAppContext) {
    let (win, s) = section(cx);
    let text = "Pre-request script error, this request's script, line 3: Error: stop";
    s.update(cx, |s, cx| {
        s.set_errors(&[note(PreRequest, text, Some(3))], cx)
    });
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("script-error-go-to-line", cx))
        .unwrap();
    frame(cx, win);
    let caret = s.read_with(cx, |s, cx| {
        let code = s.code(PreRequest).unwrap().read(cx);
        code.cursor_position().line
    });
    assert_eq!(caret, 2, "zero-based line of the third line");
}

#[gpui_kit::test]
fn an_error_without_a_line_of_this_script_offers_no_go_to_line(cx: &mut TestAppContext) {
    let (win, s) = section(cx);
    let text = "Pre-request script error, folder f script, line 3: Error: stop";
    s.update(cx, |s, cx| {
        s.set_errors(&[note(PreRequest, text, None)], cx)
    });
    frame(cx, win);
    assert_eq!(banner_label(cx, win).as_deref(), Some(text));
    let go = cx
        .update_window(win, |_, w, _| {
            w.try_find("script-error-go-to-line").is_some()
        })
        .unwrap();
    assert!(!go);
    assert_eq!(underlined(cx, &s), 0);
}

#[gpui_kit::test]
fn typing_in_the_editor_drops_the_underline_but_keeps_the_banner(cx: &mut TestAppContext) {
    let (win, s) = section(cx);
    let text = "Pre-request script error, this request's script, line 3: Error: stop";
    s.update(cx, |s, cx| {
        s.set_errors(&[note(PreRequest, text, Some(3))], cx)
    });
    frame(cx, win);
    replace_code(cx, win, None, "script-pre-request-code", "let c = 3");
    frame(cx, win);
    assert_eq!(underlined(cx, &s), 0);
    assert_eq!(banner_label(cx, win).as_deref(), Some(text));
}

#[gpui_kit::test]
fn the_request_editor_marks_the_error_of_the_result_it_shows(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    let mut r = result();
    r.error = Some(RunError::Script {
        phase: PreRequest,
        message: "this request's script: Error: stop".into(),
    });
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.show(Arc::new(r), cx));
    frame(cx, win);
    assert_eq!(
        banner_label(cx, win).as_deref(),
        Some("Pre-request script error, this request's script: Error: stop")
    );
    // A later result without an error clears it.
    model.update(cx, |m, cx| m.show(Arc::new(result()), cx));
    frame(cx, win);
    assert_eq!(banner_label(cx, win), None);
}
