//! The Script and Tests code editors: load without echo, emit per phase, reach the draft.

use crate::support;

use apinata_app::command::Edit;
use apinata_app::request_editor::Section;
use apinata_app::script_section::ScriptSection;
use apinata_app::snapshot::ScriptCode;
use apinata_core::ScriptPhase::{self, PostResponse, PreRequest, Tests};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

fn scripted() -> tempfile::TempDir {
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"bru.setVar('a', 1)\"\n    - type: tests\n      code: \"test('t', function() {{}})\"\n",
        request("a", "http://x")
    );
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

fn open_section(cx: &mut TestAppContext, win: AnyWindowHandle, s: Section) {
    cx.update_window(win, |_, w, cx| {
        w.click(("section", s as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
}

#[gpui_kit::test]
fn loading_emits_no_edit(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse], w, cx)
    });
    let s = host.read_with(cx, |h, _| h.view.clone());
    let code = ScriptCode {
        pre_request: "a()".into(),
        post_response: "b()".into(),
        tests: "c()".into(),
    };
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&code, w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
    assert_eq!(s.read_with(cx, |s, cx| s.texts(cx)), ["a()", "b()"]);
    assert!(s.read_with(cx, |s, _| s.code(Tests).is_none()));
}

#[gpui_kit::test]
fn typing_emits_the_phase_and_text(cx: &mut TestAppContext) {
    let (win, host) = host_section(cx, |w, cx| {
        ScriptSection::new(&[PreRequest, PostResponse], w, cx)
    });
    cx.update_window(win, |_, w, cx| w.click(("script-tab", 1usize), cx))
        .unwrap();
    frame(cx, win);
    replace_code(cx, win, None, "script-post-response-code", "let b = 2");
    cx.run_until_parked();
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Script(ScriptPhase::PostResponse, text)) => assert_eq!(text, "let b = 2"),
        other => panic!("no post-response edit: {other:?}"),
    }
}

#[gpui_kit::test]
fn typed_edge_whitespace_survives_the_trimmed_echo(cx: &mut TestAppContext) {
    let col = scripted();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    open_section(cx, win, Section::Script);
    replace_code(
        cx,
        win,
        Some("script-section"),
        "script-pre-request-code",
        "foo();",
    );
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        view_of(cx, &ws, &root, "a.yml").scripts.pre_request,
        "foo();"
    );
    let pre = ed
        .read_with(cx, |e, cx| e.script().read(cx).code(PreRequest).cloned())
        .unwrap();
    assert_eq!(
        pre.read_with(cx, |c, _| (c.value().to_string(), c.cursor())),
        ("foo();\n".to_string(), "foo();\n".len())
    );
}

#[gpui_kit::test]
fn the_request_editor_shows_and_edits_scripts(cx: &mut TestAppContext) {
    let col = scripted();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let view = view_of(cx, &ws, &root, "a.yml");
    assert_eq!(view.scripts.pre_request, "bru.setVar('a', 1)");
    assert_eq!(view.scripts.tests, "test('t', function() {})");
    open_section(cx, win, Section::Script);
    let pre = ed
        .read_with(cx, |e, cx| e.script().read(cx).code(PreRequest).cloned())
        .unwrap();
    assert_eq!(
        pre.read_with(cx, |c, _| c.value().to_string()),
        "bru.setVar('a', 1)"
    );
    cx.update_window(win, |_, w, cx| w.click(("script-tab", 1usize), cx))
        .unwrap();
    frame(cx, win);
    replace_code(
        cx,
        win,
        Some("script-section"),
        "script-post-response-code",
        "let b = 2",
    );
    cx.run_until_parked();
    let view = view_of(cx, &ws, &root, "a.yml");
    assert_eq!(view.scripts.post_response, "let b = 2");
    assert!(view.dirty);
    open_section(cx, win, Section::Tests);
    replace_code(
        cx,
        win,
        Some("tests-section"),
        "script-tests-code",
        "let c = 3",
    );
    cx.run_until_parked();
    assert_eq!(view_of(cx, &ws, &root, "a.yml").scripts.tests, "let c = 3");
}

#[gpui_kit::test]
fn each_section_opens_at_its_own_scroll_offset(cx: &mut TestAppContext) {
    let col = scripted();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    // Short enough that both the Script and the Tests section overflow the body.
    gpui_kit::VisualTestContext::from_window(win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(420.)));
    frame(cx, win);
    // The top edge of `id` relative to the section body's, negative once scrolled past.
    let offset = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(win, |_, w, _| {
            w.find(id).bounds().top() - w.find("section-body").bounds().top()
        })
        .unwrap()
    };
    open_section(cx, win, Section::Script);
    let at_rest = offset(cx, "script-section");
    cx.update_window(win, |_, w, cx| {
        let down =
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-1000.)));
        w.scroll("section-body", down, cx);
        w.render_frame(cx)
    })
    .unwrap();
    assert!(
        offset(cx, "script-section") < at_rest,
        "the Script section scrolled"
    );
    open_section(cx, win, Section::Tests);
    frame(cx, win);
    assert_eq!(offset(cx, "tests-section"), at_rest);
}
