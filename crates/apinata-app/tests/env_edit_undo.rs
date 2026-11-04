//! Undo and redo of an environment's variable, color and parent edits.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::env_editor::EnvEditor;
use apinata_app::env_table::EnvTableEvent;
use apinata_core::{EnvVariable, VarValue};
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<EnvEditor>,
}

fn rig(cx: &mut TestAppContext) -> Rig {
    rig_with(cx, DEV)
}

fn rig_with(cx: &mut TestAppContext, dev: &str) -> Rig {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    }
}

impl Rig {
    fn undo(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed.update(cx, |e, cx| e.undo_edit(w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn redo(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| {
            self.ed.update(cx, |e, cx| e.redo_edit(w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn value(&self, cx: &mut TestAppContext, env: &str) -> String {
        match &env_view(cx, &self.ws, &self.root, env).variables[0].value {
            VarValue::Text(t) => t.clone(),
            _ => String::new(),
        }
    }
}

#[gpui_kit::test]
fn a_variable_value_change_is_one_step_and_clean_at_the_saved_state(cx: &mut TestAppContext) {
    let r = rig(cx);
    type_at_end(cx, r.win, None, ("env-value", 0usize), "y");
    type_at_end(cx, r.win, None, ("env-value", 0usize), "z");
    cx.run_until_parked();
    assert_eq!(r.value(cx, "dev"), "xyz");
    r.undo(cx);
    assert_eq!(r.value(cx, "dev"), "x");
    assert!(!env_view(cx, &r.ws, &r.root, "dev").dirty);
    r.redo(cx);
    assert_eq!(r.value(cx, "dev"), "xyz");
    assert!(env_view(cx, &r.ws, &r.root, "dev").dirty);
}

#[gpui_kit::test]
fn switching_environment_clears_the_history(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.ed.update(cx, |e, cx| e.set_color("#ff0000", cx));
    cx.run_until_parked();
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.select("prod", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    r.ed.update(cx, |e, cx| e.set_color("#00ff00", cx));
    cx.run_until_parked();
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.select("dev", w, cx));
        r.ed.update(cx, |e, cx| e.select("prod", w, cx));
    })
    .unwrap();
    cx.run_until_parked();
    r.undo(cx);
    assert_eq!(
        env_view(cx, &r.ws, &r.root, "prod").color.as_deref(),
        Some("#00ff00")
    );
    assert_eq!(
        env_view(cx, &r.ws, &r.root, "dev").color.as_deref(),
        Some("#ff0000")
    );
}

#[gpui_kit::test]
fn an_outside_change_retires_only_its_own_field(cx: &mut TestAppContext) {
    let r = rig(cx);
    type_at_end(cx, r.win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    r.ed.update(cx, |e, cx| e.set_color("#ff0000", cx));
    cx.run_until_parked();
    let outside = vec![EnvVariable::new("host", VarValue::Text("out".into()))];
    let command = Command::Edit {
        target: Target::Environment("dev".into()),
        edit: Edit::Variables(outside),
    };
    r.ws.update(cx, |w, cx| w.send(&r.root, command, cx));
    cx.run_until_parked();
    // The color step is still valid; the variables step went stale.
    r.undo(cx);
    assert_eq!(env_view(cx, &r.ws, &r.root, "dev").color, None);
    r.undo(cx);
    assert_eq!(r.value(cx, "dev"), "out");
    r.redo(cx);
    assert_eq!(
        env_view(cx, &r.ws, &r.root, "dev").color.as_deref(),
        Some("#ff0000")
    );
}

impl Rig {
    // A saved color step, so the environment is clean (renames and duplicates need that) and
    // the history still holds an undoable edit.
    fn saved_color(&self, cx: &mut TestAppContext, color: &str) {
        self.ed.update(cx, |e, cx| e.set_color(color, cx));
        cx.run_until_parked();
        self.ed.update(cx, |e, cx| e.save(cx));
        cx.run_until_parked();
        assert!(!env_view(cx, &self.ws, &self.root, "dev").dirty);
    }

    fn selected(&self, cx: &mut TestAppContext) -> String {
        self.ed
            .read_with(cx, |e, _| e.selected().map(str::to_string))
            .unwrap_or_default()
    }
}

#[gpui_kit::test]
fn renaming_the_environment_clears_the_history(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.saved_color(cx, "#ff0000");
    r.ed.update(cx, |e, cx| e.rename("qa", cx));
    cx.run_until_parked();
    assert_eq!(r.selected(cx), "qa");
    r.undo(cx);
    assert_eq!(
        env_view(cx, &r.ws, &r.root, "qa").color.as_deref(),
        Some("#ff0000")
    );
}

#[gpui_kit::test]
fn deleting_the_environment_clears_the_history(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.saved_color(cx, "#ff0000");
    cx.update_window(r.win, |_, w, cx| r.ed.update(cx, |e, cx| e.delete(w, cx)))
        .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert_eq!(r.selected(cx), "prod");
    r.undo(cx);
    assert_eq!(env_view(cx, &r.ws, &r.root, "prod").color, None);
}

#[gpui_kit::test]
fn an_outside_delete_of_the_selected_environment_clears_the_history(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.ed.update(cx, |e, cx| e.set_color("#ff0000", cx));
    cx.run_until_parked();
    let delete = Command::Env(apinata_app::command::EnvOp::Delete { name: "dev".into() });
    r.ws.update(cx, |w, cx| w.send(&r.root, delete, cx));
    cx.run_until_parked();
    assert_eq!(r.selected(cx), "prod");
    r.undo(cx);
    assert_eq!(env_view(cx, &r.ws, &r.root, "prod").color, None);
}

#[gpui_kit::test]
fn duplicating_selects_the_copy_with_an_empty_history(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.saved_color(cx, "#ff0000");
    r.ed.update(cx, |e, cx| e.duplicate(cx));
    cx.run_until_parked();
    let copy = r.selected(cx);
    assert_ne!(copy, "dev");
    r.undo(cx);
    assert_eq!(
        env_view(cx, &r.ws, &r.root, &copy).color.as_deref(),
        Some("#ff0000")
    );
    assert_eq!(
        env_view(cx, &r.ws, &r.root, "dev").color.as_deref(),
        Some("#ff0000")
    );
}

fn table_event(r: &Rig, cx: &mut TestAppContext, event: EnvTableEvent) {
    let table = r.ed.read_with(cx, |e, _| e.table().clone());
    table.update(cx, |_, cx| cx.emit(event));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_secret_flag_change_retires_the_variables_steps(cx: &mut TestAppContext) {
    // Pins behavior only: the step's own value check also rejects the stale redo, so removing
    // the retirement does not turn this red.
    let r = rig_with(cx, "name: dev\nvariables:\n  - name: host\n    value: ''\n");
    type_at_end(cx, r.win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    r.undo(cx);
    assert!(!env_view(cx, &r.ws, &r.root, "dev").dirty);
    table_event(
        &r,
        cx,
        EnvTableEvent::SetSecret {
            name: "host".into(),
            secret: true,
        },
    );
    assert!(env_view(cx, &r.ws, &r.root, "dev").variables[0].secret);
    r.redo(cx);
    let dev = env_view(cx, &r.ws, &r.root, "dev");
    assert!(dev.variables[0].secret, "redo must not undo the flag");
    assert!(!dev.dirty, "and sends nothing");
}

#[gpui_kit::test]
fn storing_a_secret_value_leaves_the_history_alone(cx: &mut TestAppContext) {
    let r = rig(cx);
    table_event(
        &r,
        cx,
        EnvTableEvent::SetSecret {
            name: "host".into(),
            secret: true,
        },
    );
    r.ed.update(cx, |e, cx| e.set_color("#ff0000", cx));
    cx.run_until_parked();
    table_event(
        &r,
        cx,
        EnvTableEvent::SecretValue {
            name: "host".into(),
            value: "s3".into(),
        },
    );
    r.undo(cx);
    assert_eq!(env_view(cx, &r.ws, &r.root, "dev").color, None);
    r.redo(cx);
    assert_eq!(
        env_view(cx, &r.ws, &r.root, "dev").color.as_deref(),
        Some("#ff0000")
    );
}

#[gpui_kit::test]
fn a_color_edit_is_one_step_and_clean_at_the_saved_state(cx: &mut TestAppContext) {
    let r = rig(cx);
    r.ed.update(cx, |e, cx| e.set_color("#ff0000", cx));
    cx.run_until_parked();
    assert!(env_view(cx, &r.ws, &r.root, "dev").dirty);
    r.undo(cx);
    let dev = env_view(cx, &r.ws, &r.root, "dev");
    assert!(dev.color.is_none() && !dev.dirty);
    r.redo(cx);
    let dev = env_view(cx, &r.ws, &r.root, "dev");
    assert!(dev.color.as_deref() == Some("#ff0000") && dev.dirty);
}

#[gpui_kit::test]
fn a_parent_edit_is_one_step_and_clean_at_the_saved_state(cx: &mut TestAppContext) {
    let r = rig(cx);
    cx.update_window(r.win, |_, w, cx| {
        r.ed.update(cx, |e, cx| e.set_extends(Some("prod".into()), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(env_view(cx, &r.ws, &r.root, "dev").dirty);
    r.undo(cx);
    let dev = env_view(cx, &r.ws, &r.root, "dev");
    assert!(dev.extends.is_none() && !dev.dirty);
    r.redo(cx);
    let dev = env_view(cx, &r.ws, &r.root, "dev");
    assert!(dev.extends.as_deref() == Some("prod") && dev.dirty);
}

fn shown(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> bool {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn deleting_an_extended_environment_offers_only_cancel(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/qa.yml", "name: qa\nextends: dev\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.select("dev", w, cx);
            e.delete(w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert!(shown(cx, win, "confirm-cancel"));
    assert!(!shown(cx, win, "confirm-delete"));
    assert!(shown(cx, win, "confirm-detail"));
    let title = cx
        .update_window(win, |_, w, _| {
            use gpui_kit::test::TestWindowExt as _;
            w.find("confirm-dialog").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(
        title.as_deref(),
        Some("Environment \"dev\" is extended by \"qa\"")
    );
    answer_confirm(cx, "confirm-cancel");
    assert!(col.path().join("environments/dev.yml").is_file());
}

// A variable typed into the empty row, undone and redone by keys alone: the table keeps focus
// once the undo drops the row it was in, so the redo key still reaches the editor.
#[gpui_kit::test]
fn a_new_variable_row_is_redone_by_keys_after_its_undo(cx: &mut TestAppContext) {
    let r = rig(cx);
    let names = |cx: &mut TestAppContext| -> Vec<String> {
        (env_view(cx, &r.ws, &r.root, "dev").variables.iter())
            .map(|v| v.name.clone())
            .collect()
    };
    type_at_end(cx, r.win, None, ("env-name", 1usize), "port");
    cx.run_until_parked();
    assert_eq!(names(cx), ["host", "port"]);
    cx.simulate_keystrokes(r.win, &secondary("z"));
    cx.run_until_parked();
    assert_eq!(names(cx), ["host"]);
    assert!(
        holds_focus(cx, r.win, "env-table"),
        "the table holds focus after the undo"
    );
    cx.simulate_keystrokes(r.win, &secondary("shift-z"));
    cx.run_until_parked();
    assert_eq!(
        names(cx),
        ["host", "port"],
        "the redo key brings the row back"
    );
}

const ABC: &str = "name: dev\nvariables:\n  - name: a\n    value: \"1\"\n  - name: b\n    value: \"2\"\n  - name: c\n    value: \"3\"\n";

const ABCD: &str = "name: dev\nvariables:\n  - name: a\n    value: \"1\"\n  - name: b\n    value: \"2\"\n  - name: c\n    value: \"3\"\n  - name: d\n    value: \"4\"\n";

/// The variables of `dev`, with b (the second) removed and the removal undone, so a redo drops
/// it again.
fn with_a_redoable_delete_of_b(cx: &mut TestAppContext, dev: &str) -> Rig {
    let r = rig_with(cx, dev);
    let names = |cx: &mut TestAppContext| -> Vec<String> {
        (env_view(cx, &r.ws, &r.root, "dev").variables.iter())
            .map(|v| v.name.clone())
            .collect()
    };
    frame(cx, r.win);
    cx.update_window(r.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click(("env-delete", 1usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let all = names(cx);
    r.undo(cx);
    assert_eq!(names(cx).len(), all.len() + 1, "the undo brings b back");
    r
}

// A redone delete drops the row whose secrecy lock holds focus: focus goes to the name of the
// row now in that slot, as a delete sends it.
#[gpui_kit::test]
fn a_redone_variable_delete_from_its_lock_focuses_the_next_rows_name(cx: &mut TestAppContext) {
    let r = with_a_redoable_delete_of_b(cx, ABC);
    tab_within_to(cx, r.win, "env-table", ("env-secret", 1usize));
    r.redo(cx);
    assert!(holds_focus(cx, r.win, ("env-name", 1usize)));
}

// An outside change drops another row while a later row's lock holds focus: that row
// survives one slot up, and its lock keeps focus there.
#[gpui_kit::test]
fn an_outside_reload_keeps_focus_on_a_surviving_rows_lock(cx: &mut TestAppContext) {
    let r = rig_with(cx, ABC);
    frame(cx, r.win);
    cx.update_window(r.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click(("env-name", 0usize), cx)
    })
    .unwrap();
    tab_within_to(cx, r.win, "env-table", ("env-secret", 2usize));
    let dev = r.root.join("environments/dev.yml");
    let ac =
        "name: dev\nvariables:\n  - name: a\n    value: \"1\"\n  - name: c\n    value: \"3\"\n";
    std::fs::write(&dev, ac).unwrap();
    let root = r.root.clone();
    r.ws.update(cx, |w, cx| w.send(&root, Command::Reload(vec![dev]), cx));
    cx.run_until_parked();
    assert!(holds_focus(cx, r.win, ("env-secret", 1usize)));
}

// A redone delete leaves the environment unsaved, which locks every lock: the surviving row
// whose lock held focus (c, now second) keeps it on its name instead.
#[gpui_kit::test]
fn a_redone_variable_delete_moves_a_surviving_rows_lock_focus_to_its_name(cx: &mut TestAppContext) {
    let r = with_a_redoable_delete_of_b(cx, ABCD);
    tab_within_to(cx, r.win, "env-table", ("env-secret", 2usize));
    r.redo(cx);
    assert!(holds_focus(cx, r.win, ("env-name", 1usize)));
}

// An environment's variable step from before a disk reload must not let a later undo write
// the pre-reload variables back over the outside change.
#[gpui_kit::test]
fn undo_after_a_disk_reload_keeps_the_outside_change(cx: &mut TestAppContext) {
    support::init(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", ABC),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let w = watched(cx, &col);
    let (win, ed) = env_editor(cx, &w.ws, &w.root);
    let r = Rig {
        _col: col,
        _state: w.state,
        ws: w.ws,
        root: w.root,
        win,
        ed,
    };
    let pairs = |r: &Rig, cx: &mut TestAppContext| -> Vec<(String, String)> {
        (env_view(cx, &r.ws, &r.root, "dev").variables.iter())
            .map(|v| match &v.value {
                VarValue::Text(t) => (v.name.clone(), t.clone()),
                _ => (v.name.clone(), String::new()),
            })
            .collect()
    };
    let delete = |r: &Rig, cx: &mut TestAppContext, ix: usize| {
        use gpui_kit::test::TestWindowExt as _;
        cx.update_window(r.win, |_, w, cx| w.click(("env-delete", ix), cx))
            .unwrap();
        frame(cx, r.win);
    };
    let save = |r: &Rig, cx: &mut TestAppContext| {
        r.ed.update(cx, |e, cx| e.save(cx));
        cx.run_until_parked();
        assert!(!env_view(cx, &r.ws, &r.root, "dev").dirty, "the save lands");
    };
    frame(cx, r.win);
    delete(&r, cx, 1);
    save(&r, cx);
    settle(cx);
    let outside =
        "name: dev\nvariables:\n  - name: a\n    value: \"1\"\n  - name: c\n    value: \"3x\"\n";
    std::fs::write(r.root.join("environments/dev.yml"), outside).unwrap();
    wait_for(cx, "the reload to show the outside change", |cx| {
        pairs(&r, cx).iter().any(|(_, v)| v == "3x")
    });
    frame(cx, r.win);
    delete(&r, cx, 0);
    press_key(cx, r.win, &secondary("z"));
    let want = |list: &[(&str, &str)]| -> Vec<(String, String)> {
        list.iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    };
    assert_eq!(
        pairs(&r, cx),
        want(&[("a", "1"), ("c", "3x")]),
        "the undo brings a back and keeps c's outside value"
    );
    save(&r, cx);
    let disk = std::fs::read_to_string(r.root.join("environments/dev.yml")).unwrap();
    assert!(disk.contains("3x") && disk.contains("name: a"), "{disk}");
    assert!(!disk.contains("name: b"), "{disk}");
}

fn var_names(cx: &mut TestAppContext, r: &Rig) -> Vec<String> {
    (env_view(cx, &r.ws, &r.root, "dev").variables.iter())
        .map(|v| v.name.clone())
        .collect()
}

/// When `twice`, notifies the workspace once more in the same update as the first snapshot
/// whose variables are `want`, as the app's later notices do before its next frame; tests
/// draw after every update, the app only once per frame.
fn second_load_when(
    cx: &mut TestAppContext,
    r: &Rig,
    want: &'static [&'static str],
    twice: bool,
) -> Option<gpui_kit::Subscription> {
    let root = r.root.clone();
    let armed = std::cell::Cell::new(true);
    twice.then(|| {
        cx.update(|cx| {
            cx.observe(&r.ws, move |ws, cx| {
                let names: Option<Vec<String>> = ws.read(cx).snapshot(&root, cx).and_then(|s| {
                    let dev = s.content.environments.iter().find(|e| e.name == "dev")?;
                    Some(dev.variables.iter().map(|v| v.name.clone()).collect())
                });
                if names.is_some_and(|n| n == want) && armed.replace(false) {
                    ws.update(cx, |_, cx| cx.notify());
                }
            })
        })
    })
}

// Tab from a row's lock reaches its Remove, then the next row's name.
#[gpui_kit::test]
fn tab_reaches_a_variables_remove_after_its_lock(cx: &mut TestAppContext) {
    let r = rig_with(cx, ABC);
    frame(cx, r.win);
    cx.update_window(r.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click(("env-name", 0usize), cx)
    })
    .unwrap();
    tab_within_to(cx, r.win, "env-table", ("env-secret", 0usize));
    press_key(cx, r.win, "tab");
    assert!(holds_focus(cx, r.win, ("env-delete", 0usize)));
    press_key(cx, r.win, "tab");
    assert!(holds_focus(cx, r.win, ("env-name", 1usize)));
}

// A delete of a undone while c's Remove holds focus: c moves one slot down and its Remove
// keeps focus there.
fn undone_delete_above_keeps_remove(cx: &mut TestAppContext, twice: bool) {
    let r = rig_with(cx, ABC);
    frame(cx, r.win);
    cx.update_window(r.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click(("env-delete", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(var_names(cx, &r), ["b", "c"]);
    tab_within_to(cx, r.win, "env-table", ("env-delete", 1usize));
    let _second = second_load_when(cx, &r, &["a", "b", "c"], twice);
    r.undo(cx);
    assert_eq!(var_names(cx, &r), ["a", "b", "c"]);
    assert!(holds_focus(cx, r.win, ("env-delete", 2usize)));
}

#[gpui_kit::test]
fn an_undone_delete_above_keeps_focus_on_the_shifted_variables_remove(cx: &mut TestAppContext) {
    undone_delete_above_keeps_remove(cx, false);
}

#[gpui_kit::test]
fn an_undone_delete_above_through_two_loads_keeps_focus_on_the_shifted_remove(
    cx: &mut TestAppContext,
) {
    undone_delete_above_keeps_remove(cx, true);
}

// A move of a below b undone while a's Remove holds focus: a goes back first and its Remove
// keeps focus there.
fn undone_move_keeps_remove(cx: &mut TestAppContext, twice: bool) {
    let r = rig_with(cx, ABC);
    frame(cx, r.win);
    cx.update_window(r.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click(("env-name", 0usize), cx)
    })
    .unwrap();
    cx.simulate_keystrokes(r.win, "alt-down");
    cx.run_until_parked();
    assert_eq!(var_names(cx, &r), ["b", "a", "c"]);
    tab_within_to(cx, r.win, "env-table", ("env-delete", 1usize));
    let _second = second_load_when(cx, &r, &["a", "b", "c"], twice);
    r.undo(cx);
    assert_eq!(var_names(cx, &r), ["a", "b", "c"]);
    assert!(holds_focus(cx, r.win, ("env-delete", 0usize)));
}

#[gpui_kit::test]
fn an_undone_move_keeps_focus_on_the_moved_variables_remove(cx: &mut TestAppContext) {
    undone_move_keeps_remove(cx, false);
}

#[gpui_kit::test]
fn an_undone_move_through_two_loads_keeps_focus_on_the_moved_variables_remove(
    cx: &mut TestAppContext,
) {
    undone_move_keeps_remove(cx, true);
}

// A redone delete drops the row whose Remove holds focus: focus goes to the name of the row
// now in that slot.
fn redone_delete_from_remove(cx: &mut TestAppContext, twice: bool) {
    let r = with_a_redoable_delete_of_b(cx, ABC);
    tab_within_to(cx, r.win, "env-table", ("env-delete", 1usize));
    let _second = second_load_when(cx, &r, &["a", "c"], twice);
    r.redo(cx);
    assert_eq!(var_names(cx, &r), ["a", "c"]);
    assert!(holds_focus(cx, r.win, ("env-name", 1usize)));
}

#[gpui_kit::test]
fn a_redone_variable_delete_from_its_remove_focuses_the_next_rows_name(cx: &mut TestAppContext) {
    redone_delete_from_remove(cx, false);
}

#[gpui_kit::test]
fn a_redone_variable_delete_through_two_loads_from_its_remove_focuses_the_next_name(
    cx: &mut TestAppContext,
) {
    redone_delete_from_remove(cx, true);
}

/// Runs at each workspace notify whose snapshot shows c's outside value, given how many such
/// notifies came before it; one registered before the editor runs ahead of its refresh.
type Hook = std::rc::Rc<std::cell::RefCell<Box<dyn FnMut(&mut gpui_kit::App, usize)>>>;

fn hook(f: impl FnMut(&mut gpui_kit::App, usize) + 'static) -> Hook {
    std::rc::Rc::new(std::cell::RefCell::new(Box::new(f)))
}

fn var_text(v: &EnvVariable) -> &str {
    match &v.value {
        VarValue::Text(t) => t,
        _ => "",
    }
}

fn dev_vars(
    cx: &gpui_kit::App,
    ws: &Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
) -> Vec<EnvVariable> {
    let snap = ws.read(cx).snapshot(root, cx).unwrap();
    let dev = snap
        .content
        .environments
        .iter()
        .find(|e| e.name == "dev")
        .unwrap();
    dev.variables.clone()
}

fn on_outside_c(
    cx: &mut TestAppContext,
    ws: &Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    hook: Hook,
) -> gpui_kit::Subscription {
    let root = root.to_path_buf();
    let count = std::cell::Cell::new(0);
    cx.update(|cx| {
        cx.observe(ws, move |ws, cx| {
            if dev_vars(cx, &ws, &root).iter().any(|v| var_text(v) == "3x") {
                count.set(count.get() + 1);
                (hook.borrow_mut())(cx, count.get());
            }
        })
    })
}

type Slot = std::rc::Rc<
    std::cell::RefCell<
        Option<(
            Entity<EnvEditor>,
            Entity<apinata_app::workspace::Workspace>,
            std::path::PathBuf,
        )>,
    >,
>;

/// The ABC environment shown, `before` observing the workspace ahead of the editor and
/// `after` behind it; `slot` is filled with the editor, workspace and root.
fn race_rig(
    cx: &mut TestAppContext,
    slot: &Slot,
    before: Hook,
    after: Hook,
) -> (Rig, [gpui_kit::Subscription; 2]) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", ABC)]);
    let (state, ws, root) = open_dir(cx, &col);
    let first = on_outside_c(cx, &ws, &root, before);
    let (win, ed) = env_editor(cx, &ws, &root);
    let second = on_outside_c(cx, &ws, &root, after);
    *slot.borrow_mut() = Some((ed.clone(), ws.clone(), root.clone()));
    let r = Rig {
        _col: col,
        _state: state,
        ws,
        root,
        win,
        ed,
    };
    (r, [first, second])
}

/// Turns a off in the table's rows as they stand and emits them, as a click would.
fn table_turns_a_off(cx: &mut gpui_kit::App, ed: &Entity<EnvEditor>) {
    let table = ed.read(cx).table().clone();
    let mut rows = table.read(cx).rows(cx);
    rows[0].enabled = false;
    table.update(cx, |_, cx| cx.emit(EnvTableEvent::Changed(rows)));
}

/// Turns b off in the snapshot's own variables, an edit built from the reloaded content.
fn view_turns_b_off(
    cx: &mut gpui_kit::App,
    ws: &Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
) {
    let mut vars = dev_vars(cx, ws, root);
    vars[1].enabled = false;
    let command = Command::Edit {
        target: Target::Environment("dev".into()),
        edit: Edit::Variables(vars),
    };
    ws.update(cx, |w, cx| w.send(root, command, cx));
}

impl Rig {
    /// Writes c's outside value to disk and reloads the file, as the watcher does.
    fn reload_outside_c(&self, cx: &mut TestAppContext) {
        let dev = self.root.join("environments/dev.yml");
        std::fs::write(&dev, ABC.replace("value: \"3\"", "value: \"3x\"")).unwrap();
        let root = self.root.clone();
        self.ws
            .update(cx, |w, cx| w.send(&root, Command::Reload(vec![dev]), cx));
        cx.run_until_parked();
        frame(cx, self.win);
        cx.run_until_parked();
    }

    /// After everything settled: c keeps its outside value, the user's edit of a either landed
    /// on top of it or is held back as stale, and a save leaves the outside value on disk.
    fn assert_outside_c_kept(&self, cx: &mut TestAppContext) {
        let dev = env_view(cx, &self.ws, &self.root, "dev");
        assert_eq!(
            var_text(&dev.variables[2]),
            "3x",
            "the outside change survives"
        );
        let held = ws_snap(cx, &self.ws, &self.root)
            .stale
            .contains_key(&Target::Environment("dev".into()));
        assert!(
            held || !dev.variables[0].enabled,
            "the edit of a landed or is held"
        );
        if dev.dirty {
            self.ed.update(cx, |e, cx| e.save(cx));
            cx.run_until_parked();
        }
        let disk = std::fs::read_to_string(self.root.join("environments/dev.yml")).unwrap();
        assert!(disk.contains("3x"), "{disk}");
    }
}

fn same_update_race(cx: &mut TestAppContext, loads: usize) {
    let slot = Slot::default();
    let (s1, s2) = (slot.clone(), slot.clone());
    let before = hook(move |cx, n| {
        if n == 1 {
            let (ed, _, _) = s1.borrow().clone().unwrap();
            table_turns_a_off(cx, &ed);
        }
    });
    let after = hook(move |cx, n| {
        if n < loads {
            let (_, ws, _) = s2.borrow().clone().unwrap();
            ws.update(cx, |_, cx| cx.notify());
        }
    });
    let (r, _subs) = race_rig(cx, &slot, before, after);
    r.reload_outside_c(cx);
    r.assert_outside_c_kept(cx);
}

fn busy_field_race(cx: &mut TestAppContext, loads: usize) {
    let slot = Slot::default();
    let (s1, s2) = (slot.clone(), slot.clone());
    let before = hook(move |cx, n| {
        if n == 1 {
            let (_, ws, root) = s1.borrow().clone().unwrap();
            view_turns_b_off(cx, &ws, &root);
        }
    });
    let after = hook(move |cx, n| {
        let (ed, ws, _) = s2.borrow().clone().unwrap();
        if n < loads {
            ws.update(cx, |_, cx| cx.notify());
        } else if n == loads {
            table_turns_a_off(cx, &ed);
        }
    });
    let (r, _subs) = race_rig(cx, &slot, before, after);
    r.reload_outside_c(cx);
    r.assert_outside_c_kept(cx);
}

// Window one: the table sends an edit in the update the reload lands in, before the editor's
// refresh has loaded the reloaded rows.
#[gpui_kit::test]
fn a_table_edit_in_the_update_a_reload_lands_in_keeps_the_outside_change(cx: &mut TestAppContext) {
    same_update_race(cx, 1);
}

#[gpui_kit::test]
fn a_table_edit_in_the_update_a_reload_lands_in_keeps_it_across_two_loads(cx: &mut TestAppContext) {
    same_update_race(cx, 2);
}

// Window two: the reload lands while Variables has an edit in flight, so the refresh leaves
// the table on its pre-reload rows; the table's next edit is built from them.
#[gpui_kit::test]
fn a_table_edit_while_its_field_is_busy_through_a_reload_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    busy_field_race(cx, 1);
}

#[gpui_kit::test]
fn a_table_edit_while_its_field_is_busy_through_two_loads_keeps_the_outside_change(
    cx: &mut TestAppContext,
) {
    busy_field_race(cx, 2);
}
