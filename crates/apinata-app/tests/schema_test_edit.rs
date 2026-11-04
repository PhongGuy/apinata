//! "Schema test" on the response body appends a generated test to the request's tests
//! script as one undoable, dirty, approved-on-save step.

use crate::support;

use apinata_app::request_editor::Section;
use apinata_app::script_section::ScriptSection;
use apinata_app::snapshot::ScriptCode;
use apinata_core::ScriptPhase;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

const JSON: &str = r#"{"user":{"name":"ann","tags":["a","b"]},"items":[{"id":1},{"id":2}]}"#;

#[gpui_kit::test]
fn a_click_appends_the_block_as_one_undoable_dirty_step(cx: &mut TestAppContext) {
    // A real send to a local server, so the response pane shows the body as in use.
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_body_raw(JSON, "application/json"),
    );
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    frame(cx, win);
    let tests = ed.read_with(cx, |e, _| e.tests_script().clone());
    let text = |cx: &mut TestAppContext| {
        tests.read_with(cx, |t, cx| {
            t.code(ScriptPhase::Tests)
                .unwrap()
                .read(cx)
                .value()
                .to_string()
        })
    };
    assert_eq!(text(cx), "");
    let click = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| w.click("body-schema-test", cx))
            .unwrap();
        cx.run_until_parked();
    };

    click(cx);
    let first = text(cx);
    assert!(first.contains("response matches schema"), "{first}");
    assert!(first.contains("\"required\""), "{first}");
    assert!(view_of(cx, &ws, &root, "a.yml").dirty);
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Section::Tests);
    let typed = ed.read_with(cx, |e, cx| e.typed_scripts(cx));
    assert_eq!(typed, std::slice::from_ref(&first));

    // One undo returns the script to its exact earlier text, so the click was one step.
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(text(cx), "");

    click(cx);
    click(cx);
    let both = text(cx);
    assert!(both.contains("test(\"response matches schema\""), "{both}");
    assert!(
        both.contains("test(\"response matches schema (2)\""),
        "{both}"
    );
}

// Opens a request, sends it to a local server and returns the pieces the tests below drive.
macro_rules! sent_editor {
    ($cx:ident) => {{
        http_ready($cx);
        let server = mock_server(
            $cx,
            ResponseTemplate::new(200).set_body_raw(JSON, "application/json"),
        );
        let col = collection_for(&server.uri(), "");
        let (_state, ws, root) = open_dir($cx, &col);
        let (win, ed) = editor_for($cx, &ws, &root, "a.yml", false);
        let model = ed.read_with($cx, |e, _| e.send_model().clone());
        let r = root.clone();
        assert!(model.update($cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
        finished($cx, &model);
        frame($cx, win);
        let tests = ed.read_with($cx, |e, _| e.tests_script().clone());
        (server, ws, root, win, ed, tests)
    }};
}

fn click(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.click("body-schema-test", cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn redo_of_the_appended_step_is_dirty_and_still_approved_on_save(cx: &mut TestAppContext) {
    let (_server, ws, root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    let block = view_of(cx, &ws, &root, "a.yml")
        .scripts
        .get(ScriptPhase::Tests)
        .to_string();
    assert!(block.contains("response matches schema"), "{block}");

    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(!view_of(cx, &ws, &root, "a.yml").dirty, "dirty after undo");
    assert!(ed.read_with(cx, |e, cx| e.typed_scripts(cx)).is_empty());

    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.redo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    let view = view_of(cx, &ws, &root, "a.yml");
    assert!(
        view.scripts
            .get(ScriptPhase::Tests)
            .contains("response matches schema")
    );
    assert!(view.dirty);
    let shown = tests.read_with(cx, |t, cx| {
        t.code(ScriptPhase::Tests)
            .unwrap()
            .read(cx)
            .value()
            .to_string()
    });
    let typed = ed.read_with(cx, |e, cx| e.typed_scripts(cx));
    assert_eq!(typed, vec![shown]);
}

// Types `text` at the end of the tests editor with real key input, as a user would.
fn type_in_tests(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, text: &str) {
    cx.update_window(win, |_, w, cx| {
        w.click("script-tests-code", cx);
        w.press(&secondary("end"), cx);
        w.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn shown_tests(cx: &mut TestAppContext, tests: &gpui_kit::Entity<ScriptSection>) -> String {
    tests.read_with(cx, |t, cx| {
        t.code(ScriptPhase::Tests)
            .unwrap()
            .read(cx)
            .value()
            .to_string()
    })
}

#[gpui_kit::test]
fn typing_after_the_click_then_undo_removes_only_the_typing(cx: &mut TestAppContext) {
    let (_server, ws, root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    let block = shown_tests(cx, &tests);
    type_in_tests(cx, win, "// mine");
    assert_eq!(shown_tests(cx, &tests), format!("{block}// mine"));

    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    // The typing alone is taken back; the appended block stays. The file keeps a script
    // without its edge whitespace, so the echo comes back trimmed.
    assert_eq!(shown_tests(cx, &tests), apinata_core::script_code(&block));

    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(shown_tests(cx, &tests), "");
    let view_text = view_of(cx, &ws, &root, "a.yml")
        .scripts
        .get(ScriptPhase::Tests)
        .to_string();
    assert_eq!(view_text, "");
}

#[gpui_kit::test]
fn redo_after_undoing_the_typing_brings_the_typing_back(cx: &mut TestAppContext) {
    let (_server, _ws, _root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    type_in_tests(cx, win, "// mine");
    let typed = shown_tests(cx, &tests);
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(!shown_tests(cx, &tests).ends_with("// mine"));
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.redo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(shown_tests(cx, &tests), apinata_core::script_code(&typed));
}

#[gpui_kit::test]
fn typing_without_an_appended_block_stays_with_the_editor_undo(cx: &mut TestAppContext) {
    let (_server, ws, root, win, ed, tests) = sent_editor!(cx);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Tests as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    type_in_tests(cx, win, "// mine");
    assert_eq!(shown_tests(cx, &tests), "// mine");
    // The request history holds no step for plain script typing, so its undo changes nothing.
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(shown_tests(cx, &tests), "// mine");
    let view = view_of(cx, &ws, &root, "a.yml");
    assert_eq!(view.scripts.get(ScriptPhase::Tests), "// mine");
    // The key goes to the code editor's own undo, which takes the typing back.
    cx.simulate_keystrokes(win, &secondary("z"));
    cx.run_until_parked();
    assert_eq!(shown_tests(cx, &tests), "");
}

#[gpui_kit::test]
fn text_typed_before_the_click_is_kept_and_the_block_follows_it(cx: &mut TestAppContext) {
    let (_server, ws, root, win, _ed, tests) = sent_editor!(cx);
    let code = tests.read_with(cx, |t, _| t.code(ScriptPhase::Tests).unwrap().clone());
    cx.update_window(win, |_, w, cx| {
        code.update(cx, |c, cx| c.set_value("// first".to_string(), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    click(cx, win);
    let text = view_of(cx, &ws, &root, "a.yml")
        .scripts
        .get(ScriptPhase::Tests)
        .to_string();
    assert!(text.starts_with("// first"), "{text}");
    let at = text.find("response matches schema").expect("block");
    assert!(at > "// first".len(), "{text}");
}

#[gpui_kit::test]
fn a_redo_approval_that_never_echoes_does_not_type_a_later_identical_load(cx: &mut TestAppContext) {
    let (_server, _ws, _root, win, ed, tests) = sent_editor!(cx);
    let load = |cx: &mut TestAppContext, text: &str| {
        let code = ScriptCode {
            tests: text.to_string(),
            ..ScriptCode::default()
        };
        cx.update_window(win, |_, w, cx| {
            tests.update(cx, |s, cx| s.load(&code, w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    };
    // The approval is armed for "// x", but the next load brings other text and consumes it.
    tests.update(cx, |s, _| s.expect_redo(ScriptPhase::Tests, "// x"));
    load(cx, "// other");
    load(cx, "// x");
    assert!(
        ed.read_with(cx, |e, cx| e.typed_scripts(cx)).is_empty(),
        "a stale approval typed a loaded text"
    );
}

#[gpui_kit::test]
fn a_matching_redo_load_is_typed_once_and_a_repeat_is_not(cx: &mut TestAppContext) {
    let (_server, _ws, _root, win, ed, tests) = sent_editor!(cx);
    let load = |cx: &mut TestAppContext, text: &str| {
        let code = ScriptCode {
            tests: text.to_string(),
            ..ScriptCode::default()
        };
        cx.update_window(win, |_, w, cx| {
            tests.update(cx, |s, cx| s.load(&code, w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    };
    tests.update(cx, |s, _| s.expect_redo(ScriptPhase::Tests, "// x"));
    load(cx, "// x");
    assert_eq!(ed.read_with(cx, |e, cx| e.typed_scripts(cx)), ["// x"]);
    // A later reload with other text and then the same text again comes from the file.
    load(cx, "// y");
    load(cx, "// x");
    assert!(ed.read_with(cx, |e, cx| e.typed_scripts(cx)).is_empty());
}

// Presses `keys` with focus in the tests editor, as a user does.
fn press_in_tests(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, keys: &str) {
    cx.update_window(win, |_, w, cx| {
        w.click("script-tests-code", cx);
        w.press(keys, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn redo_key() -> String {
    secondary("shift-z")
}

fn view_tests(
    cx: &mut TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
) -> String {
    view_of(cx, ws, root, "a.yml")
        .scripts
        .get(ScriptPhase::Tests)
        .to_string()
}

#[gpui_kit::test]
fn ctrl_z_in_the_editor_removes_the_block_in_one_press(cx: &mut TestAppContext) {
    let (_server, ws, root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    assert!(shown_tests(cx, &tests).contains("response matches schema"));
    press_in_tests(cx, win, &secondary("z"));
    assert_eq!(shown_tests(cx, &tests), "");
    assert_eq!(view_tests(cx, &ws, &root), "");
    assert!(!view_of(cx, &ws, &root, "a.yml").dirty);
    // No orphan step: a request-level undo afterwards changes nothing.
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(shown_tests(cx, &tests), "");
    assert_eq!(view_tests(cx, &ws, &root), "");
}

#[gpui_kit::test]
fn editor_redo_restores_the_block_byte_for_byte(cx: &mut TestAppContext) {
    let (_server, ws, root, win, _ed, tests) = sent_editor!(cx);
    click(cx, win);
    let block = shown_tests(cx, &tests);
    assert!(block.ends_with('\n'), "{block:?}");
    press_in_tests(cx, win, &secondary("z"));
    assert_eq!(shown_tests(cx, &tests), "");
    press_in_tests(cx, win, &redo_key());
    assert_eq!(shown_tests(cx, &tests), block);
    assert_eq!(
        view_tests(cx, &ws, &root),
        apinata_core::script_code(&block)
    );
    assert!(view_of(cx, &ws, &root, "a.yml").dirty);
}

#[gpui_kit::test]
fn editor_undo_after_typing_takes_the_typing_then_the_block(cx: &mut TestAppContext) {
    let (_server, ws, root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    let block = shown_tests(cx, &tests);
    type_in_tests(cx, win, "// mine");
    press_in_tests(cx, win, &secondary("z"));
    assert_eq!(shown_tests(cx, &tests), block);
    press_in_tests(cx, win, &secondary("z"));
    assert_eq!(shown_tests(cx, &tests), "");
    assert_eq!(view_tests(cx, &ws, &root), "");
    press_in_tests(cx, win, &redo_key());
    assert_eq!(shown_tests(cx, &tests), block);
    press_in_tests(cx, win, &redo_key());
    assert_eq!(shown_tests(cx, &tests), format!("{block}// mine"));
    // Approval survives the editor's own redo: a save would still be asked to approve it.
    let typed = ed.read_with(cx, |e, cx| e.typed_scripts(cx));
    assert_eq!(typed, vec![format!("{block}// mine")]);
}

#[gpui_kit::test]
fn request_level_undo_then_redo_keeps_the_trailing_newline(cx: &mut TestAppContext) {
    let (_server, _ws, _root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    let block = shown_tests(cx, &tests);
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(shown_tests(cx, &tests), "");
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.redo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(shown_tests(cx, &tests), block);
}

fn undo_request(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
) {
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.undo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
}

fn redo_request(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
) {
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.redo_edit(w, cx)))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn request_undo_then_editor_redo_and_the_reverse_stay_consistent(cx: &mut TestAppContext) {
    let (_server, ws, root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    let block = shown_tests(cx, &tests);
    undo_request(cx, win, &ed);
    assert_eq!(shown_tests(cx, &tests), "");
    // The editor has nothing to redo after the request-level undo reloaded its text.
    press_in_tests(cx, win, &redo_key());
    assert_eq!(shown_tests(cx, &tests), "");
    redo_request(cx, win, &ed);
    assert_eq!(shown_tests(cx, &tests), block);
    // A request-level redo reloads the editor, which drops its own history, so its undo
    // is a no-op and the request-level step stays the one way back.
    press_in_tests(cx, win, &secondary("z"));
    assert_eq!(shown_tests(cx, &tests), block);
    undo_request(cx, win, &ed);
    assert_eq!(shown_tests(cx, &tests), "");
    assert_eq!(view_tests(cx, &ws, &root), "");
}

#[gpui_kit::test]
fn editor_undo_then_request_redo_brings_the_block_back(cx: &mut TestAppContext) {
    let (_server, ws, root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    let block = shown_tests(cx, &tests);
    press_in_tests(cx, win, &secondary("z"));
    assert_eq!(shown_tests(cx, &tests), "");
    redo_request(cx, win, &ed);
    assert_eq!(shown_tests(cx, &tests), block);
    assert_eq!(
        view_tests(cx, &ws, &root),
        apinata_core::script_code(&block)
    );
    assert!(view_of(cx, &ws, &root, "a.yml").dirty);
}

#[gpui_kit::test]
fn two_blocks_are_removed_one_editor_undo_at_a_time(cx: &mut TestAppContext) {
    let (_server, ws, root, win, _ed, tests) = sent_editor!(cx);
    click(cx, win);
    let one = shown_tests(cx, &tests);
    click(cx, win);
    let two = shown_tests(cx, &tests);
    assert!(two.len() > one.len());
    press_in_tests(cx, win, &secondary("z"));
    assert_eq!(shown_tests(cx, &tests), one);
    press_in_tests(cx, win, &secondary("z"));
    assert_eq!(shown_tests(cx, &tests), "");
    assert_eq!(view_tests(cx, &ws, &root), "");
}

#[gpui_kit::test]
fn typing_in_another_phase_does_not_touch_the_tests_step(cx: &mut TestAppContext) {
    let (_server, ws, root, win, ed, tests) = sent_editor!(cx);
    click(cx, win);
    let block = shown_tests(cx, &tests);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Script as usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click("script-pre-request-code", cx);
        w.input("", cx);
        w.input("// pre", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        view_tests(cx, &ws, &root),
        apinata_core::script_code(&block)
    );
    // The tests step is still on the stack: one request-level undo removes the block.
    undo_request(cx, win, &ed);
    assert_eq!(shown_tests(cx, &tests), "");
}
