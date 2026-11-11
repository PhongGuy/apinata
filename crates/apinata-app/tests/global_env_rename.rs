//! Renaming a global environment from the editor: a refused name keeps the source and the
//! field, and the error line belongs to the dialog that showed it.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;

use apinata_app::global_env_editor::{GlobalEnvEditor, open_global_env_editor};
use apinata_app::workspace::{GlobalOp, Workspace};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use support::*;

const NAME: &str = "global-env-name-input";
// A name the editor lets through and core refuses, so its outcome arrives late.
const CORE_REFUSES: &str = "a/b";

struct Blank;
impl gpui_kit::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        gpui_kit::div()
    }
}

struct Globals {
    dir: tempfile::TempDir,
    ws: Entity<Workspace>,
    win: AnyWindowHandle,
    editor: Entity<GlobalEnvEditor>,
    _keep: (tempfile::TempDir, tempfile::TempDir),
}

// A workspace over `names` (one global environment each, holding `host`) with the editor open on
// `selected`, and `active` the workspace's active global environment.
fn globals(cx: &mut TestAppContext, names: &[&str], active: &str, selected: &str) -> Globals {
    support::init(cx);
    let files: Vec<(String, String)> = names
        .iter()
        .map(|n| {
            (
                format!("{n}.yml"),
                format!("name: {n}\nvariables:\n  - name: host\n    value: a\n"),
            )
        })
        .collect();
    let files: Vec<(&str, &str)> = files.iter().map(|(f, t)| (&**f, &**t)).collect();
    let col = collection();
    let dir = write_tree(&files);
    let (state, ws, _root) = open_dir(cx, &col);
    let path = dir.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(path, cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.select_global_environment(Some(active.into()), cx)
    });
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let editor = open(cx, &ws, win);
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select(selected, w, cx))
    })
    .unwrap();
    frame(cx, win);
    Globals {
        dir,
        ws,
        win,
        editor,
        _keep: (col, state),
    }
}

fn open(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    win: AnyWindowHandle,
) -> Entity<GlobalEnvEditor> {
    let shared = ws.clone();
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(shared, w, cx))
        .unwrap();
    cx.run_until_parked();
    editor
}

// Every file in the globals directory with its bytes, sorted by name.
fn directory(g: &Globals) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(g.dir.path())
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            (name, std::fs::read(e.path()).unwrap())
        })
        .collect();
    files.sort();
    files
}

// Opens the name field from the Rename button, replaces its text with `to` and presses Enter.
fn rename_to(cx: &mut TestAppContext, g: &Globals, to: &str) {
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-rename", cx);
    })
    .unwrap();
    cx.simulate_keystrokes(g.win, &secondary("a"));
    cx.simulate_input(g.win, to);
    cx.simulate_keystrokes(g.win, "enter");
    cx.run_until_parked();
}

fn selected(cx: &mut TestAppContext, g: &Globals) -> Option<String> {
    g.editor
        .read_with(cx, |e, _| e.selected().map(str::to_owned))
}

// The name field as the next frame draws it: (value, focused), or None when it is gone.
fn name_field(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<(String, bool)> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(NAME).map(|e| {
            (
                e.value().unwrap_or_default().to_owned(),
                e.focused() == Some(true),
            )
        })
    })
    .unwrap()
}

// Renaming onto a taken name keeps the source selected and the field open, focused, with the
// typed text, marked invalid, and the reason under it; nothing on disk changes.
#[gpui_kit::test]
fn a_refused_rename_keeps_the_source_and_the_field(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    let before = directory(&g);
    rename_to(cx, &g, "Beta");
    assert_eq!(
        selected(cx, &g).as_deref(),
        Some("Alpha"),
        "the source stays selected"
    );
    assert_eq!(
        name_field(cx, g.win),
        Some(("Beta".to_owned(), true)),
        "the field stays open, focused, with the typed name"
    );
    assert!(present(cx, g.win, "global-env-name-danger-ring"));
    assert!(present(cx, g.win, "global-env-name-invalid-icon"));
    assert!(present(cx, g.win, "global-env-name-error"));
    assert!(
        !present(cx, g.win, "global-env-error"),
        "not on the top line"
    );
    assert_eq!(directory(&g), before, "a refused rename writes nothing");
}

// A rename that lands selects the new name and closes the field; the selection never passes
// through the active environment while the snapshot is published ahead of the outcome.
#[gpui_kit::test]
fn a_rename_selects_the_new_name_and_closes_the_field(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta", "Gamma"], "Gamma", "Beta");
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let _watch = cx.update(|cx| {
        cx.observe(&g.editor, move |e, cx| {
            log.borrow_mut()
                .push(e.read(cx).selected().map(str::to_owned));
        })
    });
    rename_to(cx, &g, "Delta");
    assert_eq!(selected(cx, &g).as_deref(), Some("Delta"));
    assert_eq!(name_field(cx, g.win), None, "the field closes");
    let seen = seen.borrow();
    assert!(
        seen.iter()
            .all(|s| matches!(s.as_deref(), Some("Beta") | Some("Delta"))),
        "only the source or the new name is ever selected: {seen:?}"
    );
    assert_eq!(
        g.ws.read_with(cx, |w, _| w.global_environments()),
        ["Alpha", "Delta", "Gamma"]
    );
}

// Editing the refused name takes the error away in the next frame, whether the editor refused
// it (a taken name, under the field) or core did (an invalid one, on the top line).
#[gpui_kit::test]
fn typing_in_the_name_field_clears_the_error(cx: &mut TestAppContext) {
    for (name, error) in [
        ("Beta", "global-env-name-error"),
        ("a/b", "global-env-error"),
    ] {
        let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
        rename_to(cx, &g, name);
        assert!(present(cx, g.win, error), "{name}");
        cx.simulate_input(g.win, "2");
        assert!(
            !present(cx, g.win, error),
            "{name}: typing clears the error"
        );
        assert!(!present(cx, g.win, "global-env-name-invalid-icon"));
        assert_eq!(name_field(cx, g.win).map(|f| f.0), Some(format!("{name}2")));
    }
}

// The error belongs to the dialog that showed it: closing clears it and a reopened editor
// starts clean.
#[gpui_kit::test]
fn closing_the_dialog_drops_the_error(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    g.ws.update(cx, |w, cx| {
        w.edit_globals(
            GlobalOp::Rename {
                from: "Alpha".into(),
                to: "Beta".into(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert!(present(cx, g.win, "global-env-error"));
    cx.simulate_keystrokes(g.win, "escape");
    cx.run_until_parked();
    assert!(!present(cx, g.win, "global-env-editor"), "Escape closes it");
    assert_eq!(
        g.ws.read_with(cx, |w, _| w.global_error().map(str::to_owned)),
        None
    );
    let _reopened = open(cx, &g.ws, g.win);
    assert!(present(cx, g.win, "global-env-editor"));
    assert!(
        !present(cx, g.win, "global-env-error"),
        "no stale error on reopen"
    );
}

// A stale error set while no editor was open does not greet the next one.
#[gpui_kit::test]
fn opening_the_dialog_starts_without_an_error(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    cx.simulate_keystrokes(g.win, "escape");
    cx.run_until_parked();
    g.ws.update(cx, |w, cx| {
        w.edit_globals(
            GlobalOp::Rename {
                from: "Alpha".into(),
                to: "Beta".into(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert!(g.ws.read_with(cx, |w, _| w.global_error().is_some()));
    let _reopened = open(cx, &g.ws, g.win);
    assert!(!present(cx, g.win, "global-env-error"));
}

// The painted-quad probe cannot see text colour, so the contract is pinned on the source: the
// error line is drawn in the theme's danger colour, as the cookie form's is.
#[test]
fn the_error_line_is_drawn_in_the_danger_colour() {
    let code = include_str!("../src/global_env_editor.rs");
    let squeezed: String = code.split_whitespace().collect();
    let at = squeezed
        .find(".id(\"global-env-error\")")
        .expect("the error line");
    let chain = &squeezed[at..];
    let chain = &chain[..chain.find(".child(").expect("its text")];
    assert!(
        chain.contains(".text_color(cx.theme().danger)"),
        "the error line must use the danger colour: {chain}"
    );
}

// Sets the workspace's error with a rename onto a taken name, as the dialog shows it.
fn refuse(cx: &mut TestAppContext, g: &Globals) {
    g.ws.update(cx, |w, cx| {
        w.edit_globals(
            GlobalOp::Rename {
                from: "Alpha".into(),
                to: "Beta".into(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert!(present(cx, g.win, "global-env-error"));
}

fn workspace_error(cx: &mut TestAppContext, g: &Globals) -> Option<String> {
    g.ws.read_with(cx, |w, _| w.global_error().map(str::to_owned))
}

// A second Enter while the rename is in flight sends nothing, so no refusal follows a rename
// that landed.
#[gpui_kit::test]
fn a_second_enter_during_a_rename_sends_nothing(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-rename", cx);
        w.press(&secondary("a"), cx);
        w.input("Delta", cx);
        w.press("enter", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &g).as_deref(), Some("Delta"));
    assert!(
        !present(cx, g.win, "global-env-error"),
        "no spurious refusal"
    );
    assert_eq!(
        g.ws.read_with(cx, |w, _| w.global_environments()),
        ["Beta", "Delta"]
    );
}

// Choosing another environment while a rename is in flight keeps that choice when the rename
// is published and when it lands.
#[gpui_kit::test]
fn a_selection_made_during_a_rename_is_kept(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta", "Gamma"], "Gamma", "Beta");
    let editor = g.editor.clone();
    cx.update_window(g.win, |_, w, cx| {
        editor.update(cx, |e, cx| {
            e.rename("Delta", w, cx);
            e.select("Alpha", w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &g).as_deref(), Some("Alpha"));
}

// Opening New while a rename is in flight keeps the New field when the rename lands.
#[gpui_kit::test]
fn a_new_name_field_opened_during_a_rename_stays_open(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    let editor = g.editor.clone();
    cx.update_window(g.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.rename("Delta", w, cx));
        w.render_frame(cx);
        w.click("global-env-create", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &g).as_deref(), Some("Delta"));
    assert_eq!(
        name_field(cx, g.win),
        Some((String::new(), true)),
        "the New field stays, focused and empty"
    );
}

// The x closes a clean editor at once and takes the error with it.
#[gpui_kit::test]
fn closing_by_the_x_drops_the_error(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    refuse(cx, &g);
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("close", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!present(cx, g.win, "global-env-editor"), "the x closes it");
    assert_eq!(workspace_error(cx, &g), None);
}

// Don't save closes an edited editor through the guard, which also takes the error.
#[gpui_kit::test]
fn closing_by_dont_save_drops_the_error(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    refuse(cx, &g);
    cx.update_window(g.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-discard");
    assert!(
        !present(cx, g.win, "global-env-editor"),
        "Don't save closes it"
    );
    assert_eq!(workspace_error(cx, &g), None);
}

// The danger ring of a refused name covers the input and no more.
#[gpui_kit::test]
fn the_danger_ring_hugs_the_name_input(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    rename_to(cx, &g, "Beta");
    let (ring, input) = cx
        .update_window(g.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("global-env-name-danger-ring").bounds(),
                w.find(NAME).bounds(),
            )
        })
        .unwrap();
    assert_eq!(ring, input, "the ring is the input's own frame");
}

// The first Escape closes the refused field and takes its error, the workspace's included,
// while the dialog stays.
#[gpui_kit::test]
fn escape_on_the_refused_field_clears_the_error(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    rename_to(cx, &g, CORE_REFUSES);
    assert!(present(cx, g.win, "global-env-error"));
    cx.simulate_keystrokes(g.win, "escape");
    cx.run_until_parked();
    assert!(present(cx, g.win, "global-env-editor"), "the dialog stays");
    assert_eq!(name_field(cx, g.win), None, "Escape closes the field");
    assert!(!present(cx, g.win, "global-env-error"), "and its error");
    assert_eq!(workspace_error(cx, &g), None);
}

// A save that fails after a refused rename is not the rename's error, so the still open name
// field is not marked by it.
#[gpui_kit::test]
fn a_failed_guard_save_does_not_mark_the_name_field(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    rename_to(cx, &g, CORE_REFUSES);
    assert!(present(cx, g.win, "global-env-name-invalid-icon"));
    replace_input(cx, g.win, None, ("env-value", 0usize), "b");
    std::fs::remove_file(g.dir.path().join("Alpha.yml")).unwrap();
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("close", cx)
    })
    .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-save");
    cx.run_until_parked();
    assert!(
        present(cx, g.win, "global-env-error"),
        "the save error shows"
    );
    assert!(name_field(cx, g.win).is_some(), "the field is still open");
    assert!(!present(cx, g.win, "global-env-name-invalid-icon"));
    assert!(!present(cx, g.win, "global-env-name-danger-ring"));
}

// Sends a rename of Alpha to `to` from the field, then, before its outcome, selects Beta and
// opens Rename on it. Each step is its own update, so the outcome waits for the next park.
fn rename_then_rename_beta(cx: &mut TestAppContext, g: &Globals, to: &str) {
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-rename", cx);
        w.press(&secondary("a"), cx);
        w.input(to, cx);
        w.press("enter", cx);
    })
    .unwrap();
    let editor = g.editor.clone();
    cx.update_window(g.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("Beta", w, cx))
    })
    .unwrap();
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-rename", cx);
    })
    .unwrap();
}

// A rename of Alpha that lands late leaves the Rename field opened on Beta since open.
#[gpui_kit::test]
fn a_late_landing_keeps_a_field_opened_on_another_environment(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta", "Gamma"], "Gamma", "Alpha");
    rename_then_rename_beta(cx, &g, "Delta");
    cx.run_until_parked();
    assert_eq!(selected(cx, &g).as_deref(), Some("Beta"));
    assert_eq!(
        name_field(cx, g.win).map(|f| f.0).as_deref(),
        Some("Beta"),
        "Beta's Rename field stays"
    );
    assert_eq!(
        g.ws.read_with(cx, |w, _| w.global_environments()),
        ["Beta", "Delta", "Gamma"]
    );
}

// A late refusal of Alpha's rename does not mark the Rename field opened on Beta since.
#[gpui_kit::test]
fn a_late_refusal_leaves_a_field_opened_on_another_environment(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta", "Gamma"], "Gamma", "Alpha");
    rename_then_rename_beta(cx, &g, CORE_REFUSES);
    cx.run_until_parked();
    assert_eq!(selected(cx, &g).as_deref(), Some("Beta"));
    assert_eq!(name_field(cx, g.win).map(|f| f.0).as_deref(), Some("Beta"));
    assert!(!present(cx, g.win, "global-env-name-invalid-icon"));
}

// Rename opened again on the new name after the snapshot moved the selection there, but before
// the outcome, is the user's own field: the landing leaves it open with what they typed.
#[gpui_kit::test]
fn a_rename_opened_on_the_new_name_before_the_outcome_stays_open(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Beta", "Alpha");
    let fired = Rc::new(std::cell::Cell::new(false));
    let (flag, editor, win) = (fired.clone(), g.editor.clone(), g.win);
    let _gap = cx.update(|cx| {
        cx.observe(&g.ws, move |_, cx| {
            if flag.get() || editor.read(cx).selected() != Some("Delta") {
                return;
            }
            flag.set(true);
            win.update(cx, |_, w, cx| {
                w.render_frame(cx);
                assert!(w.try_find(NAME).is_some(), "the outcome has not landed yet");
                w.click("global-env-rename", cx);
                w.press("end", cx);
                w.input("X", cx);
            })
            .unwrap();
        })
    });
    rename_to(cx, &g, "Delta");
    assert!(fired.get(), "Rename reopened on Delta before the outcome");
    assert_eq!(selected(cx, &g).as_deref(), Some("Delta"));
    assert_eq!(
        name_field(cx, g.win),
        Some(("DeltaX".to_owned(), true)),
        "the field opened on Delta stays, focused, with the typed text"
    );
}

// A refusal that lands after the user closed the Rename field and opened a new one on the same
// source, holding the same text, belongs to the old field: the new one is not marked.
#[gpui_kit::test]
fn a_late_refusal_leaves_a_reopened_field_with_the_same_text(cx: &mut TestAppContext) {
    let g = globals(cx, &["Alpha", "Beta"], "Alpha", "Alpha");
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-rename", cx);
        w.press(&secondary("a"), cx);
        w.input(CORE_REFUSES, cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.update_window(g.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    cx.update_window(g.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("global-env-rename", cx);
        w.press(&secondary("a"), cx);
        w.input(CORE_REFUSES, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &g).as_deref(), Some("Alpha"));
    assert_eq!(
        name_field(cx, g.win),
        Some((CORE_REFUSES.to_owned(), true)),
        "the reopened field stays with its text"
    );
    assert!(!present(cx, g.win, "global-env-name-invalid-icon"));
    assert!(!present(cx, g.win, "global-env-name-danger-ring"));
}
