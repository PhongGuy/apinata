//! Inserting a snippet at the caret of a script editor is one dirty, undoable, approved step.

use crate::support;

use apinata_app::request_editor::{RequestEditor, Section};
use apinata_core::ScriptPhase;
use apinata_core::{Snippet, script_snippets};
use gpui_kit::component::input::EditorState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use support::*;

struct Rig {
    // Kept alive so the collection directory outlives the test.
    _dir: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<RequestEditor>,
    code: Entity<EditorState>,
}

fn rig(cx: &mut TestAppContext) -> Rig {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &dir);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    let code = ed.read_with(cx, |e, cx| {
        e.tests_script()
            .read(cx)
            .code(ScriptPhase::Tests)
            .unwrap()
            .clone()
    });
    Rig {
        _dir: dir,
        ws,
        root,
        win,
        ed,
        code,
    }
}

fn snip(id: &str) -> &'static Snippet {
    script_snippets().iter().find(|s| s.id == id).unwrap()
}

impl Rig {
    // Loads `text` as if read from the file (no history) and puts the caret at `at`.
    fn seed(&self, cx: &mut TestAppContext, text: &str, at: usize) {
        cx.update_window(self.win, |_, w, cx| {
            self.code.update(cx, |c, cx| {
                c.set_value(text.to_string(), w, cx);
                c.set_selected_range(at..at, cx);
            })
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn insert(&self, cx: &mut TestAppContext, s: &Snippet) -> bool {
        let ok = cx
            .update_window(self.win, |_, w, cx| {
                self.ed
                    .update(cx, |e, cx| e.insert_snippet(ScriptPhase::Tests, s, w, cx))
            })
            .unwrap();
        cx.run_until_parked();
        ok
    }

    fn text(&self, cx: &mut TestAppContext) -> String {
        self.code.read_with(cx, |c, _| c.value().to_string())
    }

    fn selected(&self, cx: &mut TestAppContext) -> String {
        self.code.read_with(cx, |c, _| {
            let r = c.selected_range();
            c.value()[r].to_string()
        })
    }

    fn editor_undo(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            w.click("script-tests-code", cx);
            w.press(&secondary("z"), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn editor_redo(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            w.click("script-tests-code", cx);
            w.press(&secondary("shift-z"), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}

// Inserts into `before` with the caret at `at`; returns the text after one insert and checks
// that a single editor undo restores `before` exactly and a single redo restores the insert.
fn check_one_step(cx: &mut TestAppContext, before: &str, at: usize, expect_prefix: &str) -> String {
    let r = rig(cx);
    let s = snip("status-200");
    r.seed(cx, before, at);
    assert!(r.insert(cx, s));
    let after = r.text(cx);
    assert!(after.starts_with(expect_prefix), "{after:?}");
    assert!(after.contains(s.code), "{after:?}");
    assert_eq!(r.selected(cx), s.select);
    assert!(view_of(cx, &r.ws, &r.root, "a.yml").dirty);
    assert_eq!(r.ed.read_with(cx, |e, _| e.section()), Section::Tests);

    r.editor_undo(cx);
    assert_eq!(r.text(cx), before, "one undo must restore the original");
    // A second undo finds nothing left from the insert (no separate selection step).
    r.editor_undo(cx);
    assert_eq!(r.text(cx), before);
    r.editor_redo(cx);
    assert_eq!(r.text(cx), after);
    // The redone text is the user's own, so a save still approves it.
    let typed = r.ed.read_with(cx, |e, cx| e.typed_scripts(cx));
    assert_eq!(typed, vec![after.clone()]);
    after
}

#[gpui_kit::test]
fn an_empty_editor_gets_the_snippet_selected_in_one_step(cx: &mut TestAppContext) {
    let after = check_one_step(cx, "", 0, "test(");
    assert!(!after.starts_with('\n'));
}

#[gpui_kit::test]
fn a_caret_after_text_starts_the_snippet_on_a_new_line(cx: &mut TestAppContext) {
    let before = "// first";
    let after = check_one_step(cx, before, before.len(), "// first\ntest(");
    assert_eq!(after.matches("test(").count(), 1);
}

#[gpui_kit::test]
fn a_caret_mid_line_splits_after_the_text_before_it(cx: &mut TestAppContext) {
    let after = check_one_step(cx, "ab cd", 2, "ab\ntest(");
    assert!(after.ends_with("\ncd"), "{after:?}");
}

#[gpui_kit::test]
fn a_trailing_newline_means_the_caret_line_is_empty(cx: &mut TestAppContext) {
    let before = "// first\n";
    let after = check_one_step(cx, before, before.len(), "// first\ntest(");
    assert!(!after.contains("\n\ntest("), "{after:?}");
}

#[gpui_kit::test]
fn a_selection_is_kept_and_the_snippet_goes_on_a_new_line_after_it(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.seed(cx, "xx old yy", 0);
    cx.update(|cx| {
        r.code.update(cx, |c, cx| c.set_selected_range(3..6, cx));
    });
    let s = snip("set-var");
    assert!(r.insert(cx, s));
    assert_eq!(r.text(cx), format!("xx old yy\n{}", s.code));
    assert_eq!(r.selected(cx), s.select);
}

// Seeds `text` with `sel` selected, inserts the status snippet and returns the text.
fn insert_over(cx: &mut TestAppContext, text: &str, sel: std::ops::Range<usize>) -> String {
    let r = rig(cx);
    r.seed(cx, text, 0);
    cx.update(|cx| {
        r.code.update(cx, |c, cx| c.set_selected_range(sel, cx));
    });
    assert!(r.insert(cx, snip("status-200")));
    r.text(cx)
}

#[gpui_kit::test]
fn a_selection_inserts_after_its_block_and_keeps_the_blank_separator(cx: &mut TestAppContext) {
    let code = snip("status-200").code;
    let got = insert_over(cx, "a\nb\n\nc\nd", 0..1);
    assert_eq!(got, format!("a\nb\n{code}\n\nc\nd"));
}

#[gpui_kit::test]
fn a_selection_ending_at_column_zero_belongs_to_the_block_of_its_last_character(
    cx: &mut TestAppContext,
) {
    let code = snip("status-200").code;
    let got = insert_over(cx, "a\n\nb", 0..2);
    assert_eq!(got, format!("a\n{code}\n\nb"));
    let got = insert_over(cx, "a\nb\n\nc", 0..2);
    assert_eq!(got, format!("a\nb\n{code}\n\nc"));
}

// Clicks `first` then `second` in the section's own editor; returns the text and checks that
// each click was one undo step and the second click's placeholder is selected.
fn two_clicks(cx: &mut TestAppContext, section: Section, ids: [&str; 2]) {
    let r = rig(cx);
    show_section(&r, cx, section);
    let (a, b) = (snip(ids[0]), snip(ids[1]));
    click_id(&r, cx, &format!("snippet-{}", a.id));
    click_id(&r, cx, &format!("snippet-{}", b.id));
    let editor = r.ed.read_with(cx, |e, cx| match section {
        Section::Tests => e
            .tests_script()
            .read(cx)
            .code(ScriptPhase::Tests)
            .unwrap()
            .clone(),
        _ => e
            .script()
            .read(cx)
            .code(ScriptPhase::PreRequest)
            .unwrap()
            .clone(),
    });
    let text = editor.read_with(cx, |c, _| c.value().to_string());
    assert_eq!(text, format!("{}\n{}", a.code, b.code));
    let sel = editor.read_with(cx, |c, _| c.value()[c.selected_range()].to_string());
    assert_eq!(sel, b.select);
    let id = match section {
        Section::Tests => "script-tests-code",
        _ => "script-pre-request-code",
    };
    for want in [a.code.to_string(), String::new()] {
        cx.update_window(r.win, |_, w, cx| {
            w.click(id, cx);
            w.press(&secondary("z"), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(editor.read_with(cx, |c, _| c.value().to_string()), want);
    }
}

#[gpui_kit::test]
fn two_tests_snippets_in_a_row_stay_intact(cx: &mut TestAppContext) {
    two_clicks(cx, Section::Tests, ["status-200", "time-under"]);
    two_clicks(cx, Section::Tests, ["body-contains", "json-field"]);
}

#[gpui_kit::test]
fn two_script_snippets_in_a_row_stay_intact(cx: &mut TestAppContext) {
    two_clicks(cx, Section::Script, ["set-header", "set-var"]);
}

#[gpui_kit::test]
fn a_snippet_click_focuses_the_editor_and_typing_replaces_the_placeholder(cx: &mut TestAppContext) {
    let r = rig(cx);
    show_section(&r, cx, Section::Tests);
    click_id(&r, cx, "snippet-status-200");
    let focused = cx
        .update_window(r.win, |_, w, cx| {
            r.code.read(cx).focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(focused, "the editor must hold focus after a snippet click");
    cx.update_window(r.win, |_, w, cx| w.input("4", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(r.text(cx), snip("status-200").code.replace("200", "4"));
    // One undo takes back the typing, a second the insert.
    for want in [snip("status-200").code, ""] {
        cx.update_window(r.win, |_, w, cx| w.press(&secondary("z"), cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(r.text(cx), want);
    }
}

#[gpui_kit::test]
fn the_request_level_undo_also_takes_the_insert_back_in_one_step(cx: &mut TestAppContext) {
    let r = rig(cx);
    assert!(r.insert(cx, snip("status-200")));
    assert!(!r.text(cx).is_empty());
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.undo_edit(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(r.text(cx), "");
    assert!(!view_of(cx, &r.ws, &r.root, "a.yml").dirty);
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.redo_edit(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(r.text(cx).contains(snip("status-200").code));
    let typed = r.ed.read_with(cx, |e, cx| e.typed_scripts(cx));
    assert_eq!(typed, vec![r.text(cx)]);
}

#[gpui_kit::test]
fn a_phase_the_section_lacks_inserts_nothing(cx: &mut TestAppContext) {
    let r = rig(cx);
    let ok = cx
        .update_window(r.win, |_, w, cx| {
            r.ed.update(cx, |e, cx| {
                e.tests_script().update(cx, |s, cx| {
                    s.insert_snippet(ScriptPhase::PreRequest, snip("set-var"), w, cx)
                })
            })
        })
        .unwrap();
    assert!(!ok);
    assert_eq!(r.text(cx), "");
}

#[gpui_kit::test]
fn the_script_section_inserts_into_the_phase_asked_for(cx: &mut TestAppContext) {
    let r = rig(cx);
    let ok = cx
        .update_window(r.win, |_, w, cx| {
            r.ed.update(cx, |e, cx| {
                e.insert_snippet(ScriptPhase::PostResponse, snip("set-var"), w, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(ok);
    assert_eq!(r.ed.read_with(cx, |e, _| e.section()), Section::Script);
    let view = view_of(cx, &r.ws, &r.root, "a.yml");
    assert!(
        view.scripts
            .get(ScriptPhase::PostResponse)
            .contains("bru.setVar")
    );
    assert_eq!(view.scripts.get(ScriptPhase::PreRequest), "");
    assert!(view.dirty);
}

// Shows `section` of the rig's editor and renders a frame so its buttons exist.
fn show_section(r: &Rig, cx: &mut TestAppContext, section: Section) {
    r.ed.update(cx, |e, cx| e.show(section, cx));
    frame(cx, r.win);
}

fn click_id(r: &Rig, cx: &mut TestAppContext, id: &str) {
    let id = id.to_string();
    cx.update_window(r.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn each_tests_snippet_has_a_button_that_inserts_its_code(cx: &mut TestAppContext) {
    for s in script_snippets()
        .iter()
        .filter(|s| s.phases.contains(&apinata_core::SnippetPhase::Tests))
    {
        let r = rig(cx);
        show_section(&r, cx, Section::Tests);
        click_id(&r, cx, &format!("snippet-{}", s.id));
        assert_eq!(r.text(cx), s.code, "snippet {}", s.id);
        assert_eq!(r.selected(cx), s.select, "snippet {}", s.id);
    }
}

#[gpui_kit::test]
fn the_script_section_offers_the_buttons_of_the_shown_phase(cx: &mut TestAppContext) {
    let r = rig(cx);
    show_section(&r, cx, Section::Script);
    let pre = |p| {
        script_snippets()
            .iter()
            .filter(|s| s.phases.contains(&p))
            .map(|s| s.id)
            .collect::<Vec<_>>()
    };
    let (pre_ids, post_ids) = (
        pre(apinata_core::SnippetPhase::PreRequest),
        pre(apinata_core::SnippetPhase::PostResponse),
    );
    // The table offers "Set a variable from the body" in post-response as well as in Tests.
    assert_eq!((pre_ids.len(), post_ids.len()), (2, 2));
    for id in script_snippets().iter().map(|s| s.id) {
        let want = pre_ids.contains(&id);
        assert_eq!(present(cx, r.win, format!("snippet-{id}")), want, "{id}");
    }
    r.ed.update(cx, |e, cx| {
        e.script()
            .update(cx, |s, cx| s.show(ScriptPhase::PostResponse, cx))
    });
    frame(cx, r.win);
    for id in script_snippets().iter().map(|s| s.id) {
        let want = post_ids.contains(&id);
        assert_eq!(present(cx, r.win, format!("snippet-{id}")), want, "{id}");
    }
}

#[gpui_kit::test]
fn a_focused_snippet_button_inserts_on_enter_and_space(cx: &mut TestAppContext) {
    for key in ["enter", "space"] {
        let r = rig(cx);
        show_section(&r, cx, Section::Tests);
        tab_to(cx, r.win, "snippet-status-200");
        cx.update_window(r.win, |_, w, cx| w.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(r.text(cx), snip("status-200").code, "{key}");
    }
}

#[test]
fn the_tooltip_is_the_code() {
    let s = snip("status-200");
    assert_eq!(apinata_app::script_section::snippet_tooltip(s), s.code);
}

// A long script (long lines when `wrap`) in `phase`, with a snippet inserted at its end in a
// window `height` high; returns the screen span of the placeholder's line and the editor's id.
fn placeholder_after_end_insert(
    cx: &mut TestAppContext,
    height: f32,
    phase: ScriptPhase,
    wrap: bool,
) -> (Rig, f32, f32, String) {
    let r = rig(cx);
    gpui_kit::VisualTestContext::from_window(r.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(height)));
    let (section, view) = match phase {
        ScriptPhase::Tests => (
            Section::Tests,
            r.ed.read_with(cx, |e, _| e.tests_script().clone()),
        ),
        _ => (
            Section::Script,
            r.ed.read_with(cx, |e, _| e.script().clone()),
        ),
    };
    let code = view.read_with(cx, |s, _| s.code(phase).unwrap().clone());
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.show(section, cx));
        view.update(cx, |s, cx| {
            s.show(phase, cx);
            s.set_wrap(phase, wrap, w, cx)
        });
        w.render_frame(cx)
    })
    .unwrap();
    let filler = if wrap {
        "word ".repeat(60)
    } else {
        String::new()
    };
    let long: String = (0..120)
        .map(|i| format!("// line {i} {filler}\n"))
        .collect();
    cx.update_window(r.win, |_, w, cx| {
        code.update(cx, |c, cx| {
            c.set_value(long.clone(), w, cx);
            c.set_selected_range(long.len()..long.len(), cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    let sp = match phase {
        ScriptPhase::Tests => apinata_core::SnippetPhase::Tests,
        _ => apinata_core::SnippetPhase::PostResponse,
    };
    let s = script_snippets()
        .iter()
        .find(|s| s.phases.contains(&sp))
        .unwrap();
    let ok = cx
        .update_window(r.win, |_, w, cx| {
            r.ed.update(cx, |e, cx| e.insert_snippet(phase, s, w, cx))
        })
        .unwrap();
    assert!(ok);
    cx.run_until_parked();
    let id = format!("script-{}-code", phase.as_str());
    let (top, bottom) = cx
        .update_window(r.win, |_, w, cx| {
            let ed = w.find(id.clone()).bounds();
            code.read_with(cx, |c, _| {
                let line = c.value()[..c.selected_range().start].matches('\n').count();
                let lh = f32::from(c.line_height().expect("laid out"));
                let top = f32::from(ed.top()) + f32::from(c.scroll_offset().y) + line as f32 * lh;
                (top, top + lh)
            })
        })
        .unwrap();
    (r, top, bottom, id)
}

fn assert_in_body(cx: &mut TestAppContext, r: &Rig, top: f32, bottom: f32) {
    let body = cx
        .update_window(r.win, |_, w, _| w.find("section-body").bounds())
        .unwrap();
    assert!(
        top >= f32::from(body.top()) && bottom <= f32::from(body.bottom()),
        "placeholder {top}..{bottom} outside the section body {body:?}"
    );
}

#[gpui_kit::test]
fn snippet_at_the_end_of_a_long_script_is_inside_the_editor(cx: &mut TestAppContext) {
    let (r, top, bottom, id) = placeholder_after_end_insert(cx, 900., ScriptPhase::Tests, false);
    let ed = cx
        .update_window(r.win, |_, w, _| w.find(id).bounds())
        .unwrap();
    assert!(
        top >= f32::from(ed.top()) && bottom <= f32::from(ed.bottom()),
        "placeholder {top}..{bottom} outside the editor {ed:?}"
    );
}

#[gpui_kit::test]
fn snippet_at_the_end_of_a_long_script_is_inside_the_section_body(cx: &mut TestAppContext) {
    let (r, top, bottom, _) = placeholder_after_end_insert(cx, 400., ScriptPhase::Tests, false);
    assert_in_body(cx, &r, top, bottom);
}

#[gpui_kit::test]
fn snippet_at_the_end_of_a_long_post_response_script_is_inside_the_section_body(
    cx: &mut TestAppContext,
) {
    let (r, top, bottom, _) =
        placeholder_after_end_insert(cx, 400., ScriptPhase::PostResponse, false);
    assert_in_body(cx, &r, top, bottom);
}

// Wrapped lines make rows differ from lines, so the check is only that the editor shows the
// caret's end of the document: the body must not hide the editor's last rows.
#[gpui_kit::test]
fn snippet_at_the_end_of_a_long_wrapped_script_leaves_the_editor_end_in_the_body(
    cx: &mut TestAppContext,
) {
    let (r, _, _, id) = placeholder_after_end_insert(cx, 400., ScriptPhase::Tests, true);
    let (body, ed) = cx
        .update_window(r.win, |_, w, _| {
            (w.find("section-body").bounds(), w.find(id).bounds())
        })
        .unwrap();
    assert!(
        f32::from(ed.bottom()) <= f32::from(body.bottom()) + 1.
            || f32::from(ed.top()) < f32::from(body.top()),
        "editor {ed:?} hangs below the section body {body:?}"
    );
}

// Armed by an insert but never consumed because the section changed first, the follow must not
// fire when the script section is shown again.
#[gpui_kit::test]
fn a_snippet_follow_does_not_outlive_leaving_the_section(cx: &mut TestAppContext) {
    let r = rig(cx);
    gpui_kit::VisualTestContext::from_window(r.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(400.)));
    show_section(&r, cx, Section::Tests);
    let long: String = (0..120).map(|i| format!("// line {i}\n")).collect();
    r.seed(cx, &long, long.len());
    let before = cx
        .update_window(r.win, |_, w, _| w.find("script-tests-code").bounds().top())
        .unwrap();
    // Insert and leave in one step, so no render consumes the follow in the Tests section.
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| {
            e.insert_snippet(ScriptPhase::Tests, snip("status-200"), w, cx);
            e.show(Section::Headers, cx);
        });
        w.render_frame(cx)
    })
    .unwrap();
    frame(cx, r.win);
    show_section(&r, cx, Section::Tests);
    frame(cx, r.win);
    let after = cx
        .update_window(r.win, |_, w, _| w.find("script-tests-code").bounds().top())
        .unwrap();
    assert_eq!(before, after, "the body jumped on coming back");
}
