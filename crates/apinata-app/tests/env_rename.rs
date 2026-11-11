//! Renaming a request environment from its editor: a refused name keeps the source and the
//! field, marked invalid, and the error goes once the name is edited or the field closes.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;

use apinata_app::env_editor::{ENV_MENU_RENAME, EnvEditor};
use apinata_app::tabs::TabKind;
use apinata_app::workspace::Workspace;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

const NAME: &str = "env-name-input";

struct Envs {
    col: tempfile::TempDir,
    ws: Entity<Workspace>,
    win: AnyWindowHandle,
    ed: Entity<EnvEditor>,
    root: std::path::PathBuf,
    _state: tempfile::TempDir,
}

fn files() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
        ("environments/prod.yml", "name: prod\n"),
        ("environments/qa.yml", "name: qa\n"),
    ])
}

// An editor over dev, prod and qa with qa active and dev selected, so a fallback would show qa.
fn envs(cx: &mut TestAppContext) -> Envs {
    let col = files();
    let (state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("qa".into()), cx)
    });
    cx.run_until_parked();
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("dev", w, cx))
    })
    .unwrap();
    frame(cx, win);
    Envs {
        col,
        ws,
        win,
        ed,
        root,
        _state: state,
    }
}

// Every file in the environments directory with its bytes, sorted by name.
fn directory(col: &tempfile::TempDir) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(col.path().join("environments"))
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

// Opens the name field from the "…" menu, replaces its text with `to` and presses Enter.
fn rename_to(cx: &mut TestAppContext, win: AnyWindowHandle, to: &str) {
    env_menu(cx, win, ENV_MENU_RENAME);
    cx.simulate_keystrokes(win, &secondary("a"));
    cx.simulate_input(win, to);
    cx.simulate_keystrokes(win, "enter");
    cx.run_until_parked();
}

fn selected(cx: &mut TestAppContext, ed: &Entity<EnvEditor>) -> Option<String> {
    ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
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

// The refused field as one frame draws it: the typed name, focused, ringed and flagged, with
// the error just under the field.
fn assert_refused(cx: &mut TestAppContext, win: AnyWindowHandle, typed: &str) {
    assert_eq!(
        name_field(cx, win),
        Some((typed.to_owned(), true)),
        "the field stays open, focused, with the typed name"
    );
    assert!(present(cx, win, "env-name-danger-ring"), "the danger ring");
    assert!(
        present(cx, win, "env-name-invalid-icon"),
        "the invalid icon"
    );
    assert!(
        present(cx, win, "env-name-error"),
        "the error under the field"
    );
}

// What each change of the editor showed: the selection and whether the name field was open.
type Seen = Rc<RefCell<Vec<(Option<String>, bool)>>>;

// Records (selected, field open) at every change of the editor.
fn watch(cx: &mut TestAppContext, ed: &Entity<EnvEditor>) -> (Seen, gpui_kit::Subscription) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let sub = cx.update(|cx| {
        cx.observe(ed, move |e, cx| {
            let e = e.read(cx);
            log.borrow_mut()
                .push((e.selected().map(str::to_owned), e.naming_label().is_some()));
        })
    });
    (seen, sub)
}

// A name another environment holds is refused before anything is sent: dev stays selected and
// the field stays as typed, marked invalid; nothing on disk changes.
#[gpui_kit::test]
fn a_taken_name_keeps_the_source_and_the_field(cx: &mut TestAppContext) {
    let t = envs(cx);
    let before = directory(&t.col);
    rename_to(cx, t.win, "prod");
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("dev"));
    assert_refused(cx, t.win, "prod");
    assert_eq!(directory(&t.col), before, "a refused rename writes nothing");
}

// A name core refuses keeps the source and the field too, and no frame between Enter and the
// refusal shows another environment or a closed field.
#[gpui_kit::test]
fn a_core_refusal_keeps_the_source_and_the_field(cx: &mut TestAppContext) {
    let t = envs(cx);
    let before = directory(&t.col);
    env_menu(cx, t.win, ENV_MENU_RENAME);
    let (seen, _sub) = watch(cx, &t.ed);
    cx.simulate_keystrokes(t.win, &secondary("a"));
    cx.simulate_input(t.win, "a/b");
    cx.simulate_keystrokes(t.win, "enter");
    cx.run_until_parked();
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("dev"));
    assert_refused(cx, t.win, "a/b");
    let seen = seen.borrow();
    assert!(
        seen.iter()
            .all(|(s, open)| s.as_deref() == Some("dev") && *open),
        "only dev, with the field open, is ever shown: {seen:?}"
    );
    assert_eq!(directory(&t.col), before);
}

// Rename puts its field in the selected row's place in the list, the other rows untouched.
#[gpui_kit::test]
fn the_rename_field_takes_the_selected_rows_place(cx: &mut TestAppContext) {
    let t = envs(cx);
    env_menu(cx, t.win, ENV_MENU_RENAME);
    frame(cx, t.win);
    assert!(
        !present(cx, t.win, ("env-item", 0usize)),
        "dev's row gives way"
    );
    cx.update_window(t.win, |_, w, cx| {
        w.render_frame(cx);
        let (list, field, next) = (
            w.find("env-list").bounds(),
            w.find(NAME).bounds(),
            w.find(("env-item", 1usize)).bounds(),
        );
        assert!(
            field.left() >= list.left() && field.right() <= list.right() + gpui_kit::px(1.),
            "{field:?} sits inside the list {list:?}"
        );
        assert!(field.bottom() <= next.top(), "in dev's place, above prod");
    })
    .unwrap();
}

// A taken name's reason floats just under the field as a sentence, not at the pane's bottom.
#[gpui_kit::test]
fn a_taken_names_reason_sits_under_the_field(cx: &mut TestAppContext) {
    let t = envs(cx);
    rename_to(cx, t.win, "prod");
    assert!(!present(cx, t.win, "env-error"), "no line under the pane");
    cx.update_window(t.win, |_, w, cx| {
        w.render_frame(cx);
        let (field, error) = (w.find(NAME).bounds(), w.find("env-name-error"));
        assert_eq!(
            error.label(),
            Some("An environment named \"prod\" already exists")
        );
        let error = error.bounds();
        assert!(
            error.top() >= field.bottom() - gpui_kit::px(1.)
                && error.top() <= field.bottom() + gpui_kit::px(8.),
            "{error:?} sits just under {field:?}"
        );
    })
    .unwrap();
}

// Editing the refused name answers the error: the line, ring and icon go in the next frame.
#[gpui_kit::test]
fn typing_in_the_refused_field_clears_the_error(cx: &mut TestAppContext) {
    let t = envs(cx);
    rename_to(cx, t.win, "prod");
    assert_refused(cx, t.win, "prod");
    cx.simulate_input(t.win, "2");
    assert!(
        !present(cx, t.win, "env-name-error"),
        "typing clears the error"
    );
    assert!(!present(cx, t.win, "env-name-danger-ring"));
    assert!(!present(cx, t.win, "env-name-invalid-icon"));
    assert_eq!(name_field(cx, t.win).map(|f| f.0).as_deref(), Some("prod2"));
}

// Escape closes the refused field and takes its error with it.
#[gpui_kit::test]
fn escape_on_the_refused_field_clears_the_error(cx: &mut TestAppContext) {
    let t = envs(cx);
    rename_to(cx, t.win, "a/b");
    assert_refused(cx, t.win, "a/b");
    cx.simulate_keystrokes(t.win, "escape");
    cx.run_until_parked();
    assert_eq!(name_field(cx, t.win), None, "Escape closes the field");
    assert!(!present(cx, t.win, "env-name-error"), "and drops its error");
    assert!(!present(cx, t.win, "env-name-invalid-icon"));
}

// The error belongs to the tab that showed it: a reopened environments tab starts clean.
#[gpui_kit::test]
fn a_reopened_tab_shows_no_error(cx: &mut TestAppContext) {
    let col = files();
    let s = started(cx, &col);
    let open = |cx: &mut TestAppContext| {
        let tab =
            s.ws.update(cx, |w, cx| {
                w.open_settings(&s.root, TabKind::Environments, cx)
            })
            .unwrap();
        frame(cx, s.win);
        tab
    };
    let tab = open(cx);
    rename_to(cx, s.win, "prod");
    assert!(present(cx, s.win, "env-name-invalid-icon"), "refused");
    assert!(present(cx, s.win, "env-name-error"));
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![tab], w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "env-editor"), "the tab is closed");
    open(cx);
    assert!(present(cx, s.win, "env-editor"));
    assert!(
        !present(cx, s.win, "env-name-error"),
        "no stale error on reopen"
    );
    assert!(!present(cx, s.win, "env-error"));
}

// A rename that lands selects the new name and closes the field; until then dev stays
// selected with the field open, and no frame ever shows the active qa.
#[gpui_kit::test]
fn a_rename_selects_the_new_name_once_it_lands(cx: &mut TestAppContext) {
    let t = envs(cx);
    env_menu(cx, t.win, ENV_MENU_RENAME);
    let (seen, _sub) = watch(cx, &t.ed);
    cx.update_window(t.win, |_, w, cx| {
        w.press(&secondary("a"), cx);
        w.input("stage", cx);
        w.press("enter", cx);
    })
    .unwrap();
    assert_eq!(
        selected(cx, &t.ed).as_deref(),
        Some("dev"),
        "the selection waits for the outcome"
    );
    assert_eq!(
        name_field(cx, t.win),
        Some(("stage".to_owned(), true)),
        "the field waits too"
    );
    cx.run_until_parked();
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("stage"));
    assert_eq!(name_field(cx, t.win), None, "the field closes");
    assert!(!present(cx, t.win, "env-error"));
    let seen = seen.borrow();
    assert!(
        seen.iter()
            .all(|(s, _)| matches!(s.as_deref(), Some("dev") | Some("stage"))),
        "only the source or the new name is ever selected: {seen:?}"
    );
    let mut names = t.ws.read_with(cx, |w, cx| w.environments(&t.root, cx));
    names.sort();
    assert_eq!(names, ["prod", "qa", "stage"]);
}

// Choosing another environment before a refusal arrives keeps that choice, and no field is
// marked invalid.
#[gpui_kit::test]
fn a_selection_made_before_a_refusal_is_kept(cx: &mut TestAppContext) {
    let t = envs(cx);
    let ed = t.ed.clone();
    cx.update_window(t.win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.rename("a/b", cx);
            e.select("prod", w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("prod"));
    assert!(!present(cx, t.win, "env-name-invalid-icon"));
    assert!(!present(cx, t.win, "env-name-danger-ring"));
}

// A second Enter while the rename is in flight sends nothing, so no refusal follows a rename
// that landed.
#[gpui_kit::test]
fn a_second_enter_during_a_rename_sends_nothing(cx: &mut TestAppContext) {
    let t = envs(cx);
    env_menu(cx, t.win, ENV_MENU_RENAME);
    cx.update_window(t.win, |_, w, cx| {
        w.press(&secondary("a"), cx);
        w.input("stage", cx);
        w.press("enter", cx);
    })
    .unwrap();
    assert!(name_field(cx, t.win).is_some(), "the field is still open");
    cx.update_window(t.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("stage"));
    assert!(!present(cx, t.win, "env-error"), "no spurious refusal");
}

// The painted-quad probe cannot see text colour, so the contract is pinned on the source: the
// error line is drawn in the theme's danger colour, as the cookie form's is.
#[test]
fn the_error_line_is_drawn_in_the_danger_colour() {
    let code = include_str!("../src/env_editor.rs");
    let squeezed: String = code.split_whitespace().collect();
    let at = squeezed.find(".id(\"env-error\")").expect("the error line");
    let chain = &squeezed[at..];
    let chain = &chain[..chain.find(".child(").expect("its text")];
    assert!(
        chain.contains(".text_color(cx.theme().danger)"),
        "the error line must use the danger colour: {chain}"
    );
}

// The danger ring of a refused name covers the input and no more.
#[gpui_kit::test]
fn the_danger_ring_hugs_the_name_input(cx: &mut TestAppContext) {
    let t = envs(cx);
    rename_to(cx, t.win, "prod");
    let (ring, input) = cx
        .update_window(t.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("env-name-danger-ring").bounds(),
                w.find(NAME).bounds(),
            )
        })
        .unwrap();
    assert_eq!(ring, input, "the ring is the input's own frame");
}

// A later command (here Save, which leaves the field open) is not the rename's refusal: it
// clears the error line, and the field is no longer marked invalid.
#[gpui_kit::test]
fn a_later_save_unmarks_the_refused_field(cx: &mut TestAppContext) {
    let t = envs(cx);
    rename_to(cx, t.win, "prod");
    assert!(present(cx, t.win, "env-name-invalid-icon"));
    t.ed.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert!(name_field(cx, t.win).is_some(), "the field is still open");
    assert!(!present(cx, t.win, "env-error"));
    assert!(!present(cx, t.win, "env-name-error"));
    assert!(!present(cx, t.win, "env-name-invalid-icon"));
}

// Sends a rename of dev to `to` from the field, then, before its outcome, selects prod and
// opens Rename on it. Each step is its own update, so nothing runs the actor in between.
fn rename_then_rename_prod(cx: &mut TestAppContext, t: &Envs, to: &str) {
    env_menu(cx, t.win, ENV_MENU_RENAME);
    cx.update_window(t.win, |_, w, cx| {
        w.press(&secondary("a"), cx);
        w.input(to, cx);
        w.press("enter", cx);
    })
    .unwrap();
    let ed = t.ed.clone();
    cx.update_window(t.win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("prod", w, cx))
    })
    .unwrap();
    cx.update_window(t.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("env-more", cx);
    })
    .unwrap();
    cx.update_window(t.win, |_, w, cx| {
        w.render_frame(cx);
        w.within("popup-menu").click(ENV_MENU_RENAME, cx);
    })
    .unwrap();
}

// A late refusal of dev's rename does not mark the Rename field opened on prod since.
#[gpui_kit::test]
fn a_late_refusal_leaves_a_field_opened_on_another_environment(cx: &mut TestAppContext) {
    let t = envs(cx);
    rename_then_rename_prod(cx, &t, "a/b");
    cx.run_until_parked();
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("prod"));
    assert_eq!(
        t.ed.read_with(cx, |e, _| e.naming_label()).as_deref(),
        Some("Rename \"prod\" to")
    );
    assert!(!present(cx, t.win, "env-name-invalid-icon"));
    assert!(!present(cx, t.win, "env-name-danger-ring"));
}

// A rename of dev that lands late leaves the Rename field opened on prod since open.
#[gpui_kit::test]
fn a_late_landing_keeps_a_field_opened_on_another_environment(cx: &mut TestAppContext) {
    let t = envs(cx);
    rename_then_rename_prod(cx, &t, "stage");
    cx.run_until_parked();
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("prod"));
    assert_eq!(
        name_field(cx, t.win).map(|f| f.0).as_deref(),
        Some("prod"),
        "prod's Rename field stays"
    );
    let mut names = t.ws.read_with(cx, |w, cx| w.environments(&t.root, cx));
    names.sort();
    assert_eq!(names, ["prod", "qa", "stage"]);
}

// Runs `act` once, from a workspace observer, on the first snapshot that acknowledges a newer
// command than now: after that snapshot's redraw and before its notices are delivered.
fn in_the_gap(
    cx: &mut TestAppContext,
    t: &Envs,
    act: impl Fn(&mut EnvEditor, &mut gpui_kit::Context<EnvEditor>) + 'static,
) -> (Rc<std::cell::Cell<bool>>, gpui_kit::Subscription) {
    let root = t.root.clone();
    let acked = move |ws: &Entity<Workspace>, cx: &gpui_kit::App| {
        ws.read(cx).snapshot(&root, cx).map_or(0, |s| s.acked)
    };
    let before = t.ws.read_with(cx, |_, cx| acked(&t.ws, cx));
    let fired = Rc::new(std::cell::Cell::new(false));
    let (flag, ed) = (fired.clone(), t.ed.clone());
    let sub = cx.update(|cx| {
        cx.observe(&t.ws, move |ws, cx| {
            if !flag.get() && acked(&ws, cx) > before {
                flag.set(true);
                ed.update(cx, |e, cx| act(e, cx));
            }
        })
    });
    (fired, sub)
}

// A Save sent after the refusal's snapshot redrew but before its notices arrived does not
// swallow the refusal: the next frame shows the error line and the refused field.
#[gpui_kit::test]
fn a_command_sent_before_a_refusal_is_delivered_keeps_it(cx: &mut TestAppContext) {
    let t = envs(cx);
    env_menu(cx, t.win, ENV_MENU_RENAME);
    let (fired, _gap) = in_the_gap(cx, &t, |e, cx| e.save(cx));
    cx.simulate_keystrokes(t.win, &secondary("a"));
    cx.simulate_input(t.win, "a/b");
    cx.simulate_keystrokes(t.win, "enter");
    cx.run_until_parked();
    assert!(fired.get(), "the Save went out in the gap");
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("dev"));
    assert_refused(cx, t.win, "a/b");
}

// A created environment is still selected when another command goes out between its
// snapshot's redraw and the delivery of its creation notice.
#[gpui_kit::test]
fn a_command_sent_before_a_creation_is_delivered_keeps_it(cx: &mut TestAppContext) {
    let t = envs(cx);
    let (fired, _gap) = in_the_gap(cx, &t, |e, cx| e.save(cx));
    t.ed.update(cx, |e, cx| e.create("stage", cx));
    cx.run_until_parked();
    assert!(fired.get(), "the Save went out in the gap");
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("stage"));
}

// Enter commits the typed name, so typing on in the field before the rename lands starts a new
// run: one undo takes back only that typing. The kit's single-line Enter ends the run itself.
#[gpui_kit::test]
fn one_undo_after_a_rename_commit_keeps_the_committed_name(cx: &mut TestAppContext) {
    let t = envs(cx);
    env_menu(cx, t.win, ENV_MENU_RENAME);
    cx.update_window(t.win, |_, w, cx| {
        w.press(&secondary("a"), cx);
        for key in ["s", "t", "a", "g", "e", "enter", "x", "y"] {
            w.press(key, cx);
        }
    })
    .unwrap();
    assert_eq!(name_field(cx, t.win), Some(("stagexy".to_owned(), true)));
    cx.update_window(t.win, |_, w, cx| w.press(&secondary("z"), cx))
        .unwrap();
    assert_eq!(name_field(cx, t.win), Some(("stage".to_owned(), true)));
}
