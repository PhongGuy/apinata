use crate::support;

use apinata_app::command::Command;
use apinata_app::env_editor::{
    ENV_MENU_DELETE, ENV_MENU_EXPORT, ENV_MENU_EXPORT_ALL, ENV_MENU_RENAME, ENV_SWATCH_NAMES,
    ENV_SWATCHES, MORE_ACTIONS, RENAME_LOCKED_REASON,
};
use apinata_app::env_table::EnvTableEvent;
use apinata_app::style::{ROW_HEIGHT, at_zoom};
use apinata_core::{Collection, MemoryStore, SecretStore, VarValue};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Focusable as _, TestAppContext, px};
use std::sync::Arc;
use support::*;

const DEV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", "name: prod\n"),
    ])
}

fn selected(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::env_editor::EnvEditor>,
) -> Option<String> {
    ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
}

#[gpui_kit::test]
fn a_variable_edit_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (tree(), tree());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(selected(cx, &ed).as_deref(), Some("dev"));
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    assert!(env_view(cx, &ws, &root, "dev").dirty);
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    let env = c.environment_mut("dev").unwrap();
    let mut vars = env.variables();
    vars[0].value = VarValue::Text("xy".into());
    env.set_variables(vars);
    env.save().unwrap();
    let read =
        |d: &tempfile::TempDir| std::fs::read(d.path().join("environments/dev.yml")).unwrap();
    assert_eq!(read(&col), read(&twin));
    assert!(!env_view(cx, &ws, &root, "dev").dirty);
}

#[gpui_kit::test]
fn a_secret_value_and_reveal_go_through_the_store(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.click(("env-value", 1usize), cx);
        w.input("s3cret", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "token");
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
    assert!(
        !env_view(cx, &ws, &root, "dev").dirty,
        "a secret value is not a draft"
    );
    store.set(&key, "rotated").unwrap();
    cx.update_window(win, |_, w, cx| w.click(("env-reveal", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    let table = ed.read_with(cx, |e, _| e.table().clone());
    let input = table
        .read_with(cx, |t, _| t.value_input(1).cloned())
        .unwrap();
    assert_eq!(input.read_with(cx, |i, _| i.value().to_string()), "rotated");
}

#[gpui_kit::test]
fn create_rename_and_delete(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    ed.update(cx, |e, cx| e.create("qa", cx));
    cx.run_until_parked();
    assert_eq!(selected(cx, &ed).as_deref(), Some("qa"));
    ed.update(cx, |e, cx| e.rename("staging", cx));
    cx.run_until_parked();
    assert_eq!(selected(cx, &ed).as_deref(), Some("staging"));
    let envs = col.path().join("environments");
    assert!(envs.join("staging.yml").exists() && !envs.join("qa.yml").exists());
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.delete(w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(!envs.join("staging.yml").exists());
    assert_eq!(
        selected(cx, &ed).as_deref(),
        Some("dev"),
        "the first one is selected after a delete"
    );
}

#[gpui_kit::test]
fn color_extends_and_a_refused_rename(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert!(!ed.update(cx, |e, cx| e.set_color("red", cx)));
    assert_eq!(
        env_view(cx, &ws, &root, "dev").color,
        None,
        "a refused colour is not drafted"
    );
    assert!(ed.update(cx, |e, cx| e.set_color("#ff0000", cx)));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.set_extends(Some("prod".into()), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let dev = env_view(cx, &ws, &root, "dev");
    assert_eq!(
        (dev.color.as_deref(), dev.extends.as_deref()),
        (Some("#ff0000"), Some("prod"))
    );
    assert!(!present(cx, win, "env-color"), "no free hex entry is drawn");
    ed.update(cx, |e, cx| e.rename("prod", cx));
    cx.run_until_parked();
    assert!(
        ed.read_with(cx, |e, _| e.error().is_some()),
        "an existing name is refused"
    );
    assert_eq!(
        selected(cx, &ed).as_deref(),
        Some("dev"),
        "a refused rename keeps the selection"
    );
    assert!(present(cx, win, "env-error"));
}

#[gpui_kit::test]
fn rename_and_the_secret_toggle_wait_for_a_clean_environment(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    env_menu(cx, win, ENV_MENU_RENAME);
    cx.update_window(win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-secret", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!present(cx, win, "env-name-input"));
    ed.update(cx, |e, cx| e.rename("qa", cx));
    cx.run_until_parked();
    let dev = env_view(cx, &ws, &root, "dev");
    assert!(
        dev.dirty && !dev.variables[0].secret,
        "nothing was saved behind the draft"
    );
    assert!(col.path().join("environments/dev.yml").exists());
}

#[gpui_kit::test]
fn switching_environments_stores_uncommitted_secret_typing_in_the_old_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        (
            "environments/prod.yml",
            "name: prod\nvariables:\n  - secret: true\n    name: token\n",
        ),
    ]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("prod", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    let c = Collection::open(col.path()).unwrap();
    assert_eq!(
        store.get(&c.secret_key("dev", "token")).unwrap().as_deref(),
        Some("typed")
    );
    assert_eq!(store.get(&c.secret_key("prod", "token")).unwrap(), None);
    let table = ed.read_with(cx, |e, _| e.table().clone());
    let input = table
        .read_with(cx, |t, _| t.value_input(0).cloned())
        .unwrap();
    assert_eq!(
        input.read_with(cx, |i, _| i.value().to_string()),
        "",
        "prod's row starts empty"
    );
}

#[gpui_kit::test]
fn a_rename_the_actor_refuses_shows_its_error_and_keeps_the_old_name(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("prod", w, cx))
    })
    .unwrap();
    ed.update(cx, |e, cx| e.rename("a/b", cx));
    // A redraw from a snapshot older than the rename keeps the source until the outcome.
    ws.update(cx, |_, cx| cx.notify());
    assert_eq!(
        selected(cx, &ed).as_deref(),
        Some("prod"),
        "the source stays selected at send"
    );
    cx.run_until_parked();
    let error = ed
        .read_with(cx, |e, _| e.error().map(|m| m.to_string()))
        .expect("an error");
    assert!(error.contains("\"a/b\"") && !error.contains('`'), "{error}");
    assert_eq!(selected(cx, &ed).as_deref(), Some("prod"));
    assert!(present(cx, win, "env-error"));
}

#[gpui_kit::test]
fn a_dirty_environment_locks_the_secret_toggle_and_the_editor_refuses_it(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    let table = ed.read_with(cx, |e, _| e.table().clone());
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _sub = cx.update(|cx| {
        let seen = seen.clone();
        cx.subscribe(&table, move |_, e: &EnvTableEvent, _| {
            seen.borrow_mut().push(e.clone())
        })
    });
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("env-secret", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let asked = seen
        .borrow()
        .iter()
        .any(|e| matches!(e, EnvTableEvent::SetSecret { .. }));
    assert!(!asked, "the table's toggle is locked");
    table.update(cx, |_, cx| {
        cx.emit(EnvTableEvent::SetSecret {
            name: "host".into(),
            secret: true,
        })
    });
    cx.run_until_parked();
    let dev = env_view(cx, &ws, &root, "dev");
    assert!(
        dev.dirty && !dev.variables[0].secret,
        "the editor refuses it too"
    );
}

#[gpui_kit::test]
fn saving_stores_uncommitted_secret_typing_first(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, _ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "token");
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn a_rename_waits_while_an_edit_is_in_flight(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    // The snapshot showing the draft has not arrived yet; the edit is only in flight.
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    assert!(!env_view(cx, &ws, &root, "dev").dirty);
    ed.update(cx, |e, cx| e.rename("qa", cx));
    cx.run_until_parked();
    assert!(
        env_view(cx, &ws, &root, "dev").dirty,
        "the draft was not saved by a rename"
    );
    assert!(col.path().join("environments/dev.yml").exists());
}

#[gpui_kit::test]
fn the_selected_environment_is_marked_in_the_list(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(selected(cx, &ed).as_deref(), Some("dev"));
    assert!(present(cx, win, ("env-item-mark", 0usize)));
    assert!(!present(cx, win, ("env-item-mark", 1usize)));
    let (mark, row) = cx
        .update_window(win, |_, w, _| {
            (
                w.find(("env-item-mark", 0usize)).bounds(),
                w.find(("env-item", 0usize)).bounds(),
            )
        })
        .unwrap();
    assert!(mark.size.width > px(0.), "the mark is drawn");
    assert_eq!(mark.size.height, row.size.height, "the mark spans the row");
    assert_eq!(row.size.height, at_zoom(ROW_HEIGHT, 1.0));
    cx.update_window(win, |_, w, cx| w.click(("env-item", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &ed).as_deref(), Some("prod"));
    assert!(!present(cx, win, ("env-item-mark", 0usize)));
    assert!(present(cx, win, ("env-item-mark", 1usize)));
}

#[gpui_kit::test]
fn the_selected_mark_is_the_accent_only_while_the_list_has_focus(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    let color = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| ed.read(cx).mark_color(w, cx))
            .unwrap()
    };
    let (primary, muted) = cx.update(|cx| (cx.theme().primary, cx.theme().muted_foreground));
    cx.update_window(win, |_, w, cx| w.blur(cx)).unwrap();
    assert_eq!(color(cx), muted);
    let handle = ed.read_with(cx, |e, cx| e.focus_handle(cx));
    cx.update_window(win, |_, w, cx| w.focus(&handle, cx))
        .unwrap();
    assert_eq!(color(cx), primary);
}

#[gpui_kit::test]
fn an_environment_row_is_selectable_from_the_keyboard(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    // Clicking the selected row only focuses it; Tab then reaches the next row.
    cx.update_window(win, |_, w, cx| w.click(("env-item", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &ed).as_deref(), Some("dev"));
    for k in ["tab", "enter"] {
        cx.update_window(win, |_, w, cx| w.press(k, cx)).unwrap();
        cx.run_until_parked();
    }
    assert_eq!(selected(cx, &ed).as_deref(), Some("prod"));
}

#[gpui_kit::test]
fn new_and_rename_label_their_name_input(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert!(!present(cx, win, "env-name-label"));
    assert_eq!(ed.read_with(cx, |e, _| e.naming_label()), None);
    let input = ed.read_with(cx, |e, _| e.name_input().clone());
    assert_eq!(
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string()),
        "Environment name"
    );
    cx.update_window(win, |_, w, cx| w.click("env-create", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, "env-name-label"));
    assert_eq!(
        ed.read_with(cx, |e, _| e.naming_label()).as_deref(),
        Some("New environment name")
    );
    assert_eq!(input.read_with(cx, |i, _| i.value().to_string()), "");
    env_menu(cx, win, ENV_MENU_RENAME);
    assert_eq!(
        ed.read_with(cx, |e, _| e.naming_label()).as_deref(),
        Some("Rename \"dev\"")
    );
    assert_eq!(input.read_with(cx, |i, _| i.value().to_string()), "dev");
    env_menu(cx, win, ENV_MENU_RENAME);
    assert!(
        !present(cx, win, "env-name-label"),
        "a second click closes it"
    );
}

// New's field is a new entry at the end of the list, so it cannot read as a rename of the
// open environment, whose heading stays as it was.
#[gpui_kit::test]
fn the_new_environment_field_is_a_new_row_in_the_list(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| w.click("env-create", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let (list, field, label, last, new, title) = (
            w.find("env-list").bounds(),
            w.find("env-name-input").bounds(),
            w.find("env-name-label").bounds(),
            w.find(("env-item", 1usize)).bounds(),
            w.find("env-create").bounds(),
            w.find("env-title").bounds(),
        );
        for b in [field, label] {
            assert!(
                b.left() >= list.left() && b.right() <= list.right() + px(1.),
                "{b:?} sits inside the list {list:?}"
            );
        }
        assert!(label.top() >= last.bottom(), "after the last environment");
        assert!(field.bottom() <= new.top(), "before New");
        assert!(
            field.left() < title.left(),
            "not in the open environment's pane"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_editor_opens_on_the_active_environment(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("prod".into()), cx)
    });
    cx.run_until_parked();
    let (_win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(selected(cx, &ed).as_deref(), Some("prod"));
}

#[gpui_kit::test]
fn deleting_the_selected_environment_falls_back_to_the_active_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", "name: prod\n"),
        ("environments/qa.yml", "name: qa\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    ws.update(cx, |w, cx| {
        w.select_environment(&root, Some("prod".into()), cx)
    });
    cx.run_until_parked();
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.select("qa", w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &ed).as_deref(), Some("qa"));
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.delete(w, cx)))
        .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(!col.path().join("environments/qa.yml").exists());
    assert_eq!(selected(cx, &ed).as_deref(), Some("prod"));
}

#[gpui_kit::test]
fn escape_closes_the_name_prompt_and_keeps_focus_in_the_editor(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    for opener in ["env-create", "rename"] {
        if opener == "env-create" {
            cx.update_window(win, |_, w, cx| w.click(opener, cx))
                .unwrap();
            cx.run_until_parked();
        } else {
            env_menu(cx, win, ENV_MENU_RENAME);
        }
        assert!(present(cx, win, "env-name-label"), "{opener} opens it");
        cx.update_window(win, |_, w, cx| {
            w.input("typed", cx);
            w.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(ed.read_with(cx, |e, _| e.naming_label()), None, "{opener}");
        assert!(!present(cx, win, "env-name-label"), "{opener}");
        let focused = cx
            .update_window(win, |_, w, cx| {
                ed.read(cx).focus_handle(cx).contains_focused(w, cx)
            })
            .unwrap();
        assert!(focused, "{opener}: focus returns to the editor");
    }
    assert_eq!(selected(cx, &ed).as_deref(), Some("dev"));
    assert_eq!(
        ws.read_with(cx, |w, cx| w.environments(&root, cx)),
        ["dev", "prod"],
        "nothing was created or renamed"
    );
}

// Deletes `env` on disk and tells the collection, as the watcher would.
fn delete_outside(
    cx: &mut TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    col: &tempfile::TempDir,
    env: &str,
) {
    let path = col.path().join(format!("environments/{env}.yml"));
    std::fs::remove_file(&path).unwrap();
    ws.update(cx, |w, cx| w.send(root, Command::Reload(vec![path]), cx));
}

#[gpui_kit::test]
fn rename_closes_when_the_last_environment_is_deleted_outside(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    env_menu(cx, win, ENV_MENU_RENAME);
    assert_eq!(
        ed.read_with(cx, |e, _| e.naming_label()).as_deref(),
        Some("Rename \"dev\"")
    );
    delete_outside(cx, &ws, &root, &col, "dev");
    wait_for(cx, "dev gone", |cx| selected(cx, &ed).is_none());
    assert_eq!(ed.read_with(cx, |e, _| e.naming_label()), None);
    assert!(!present(cx, win, "env-name-label"));
    assert!(present(cx, win, "env-empty"));
}

#[gpui_kit::test]
fn rename_closes_rather_than_retargets_when_its_environment_goes(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    env_menu(cx, win, ENV_MENU_RENAME);
    delete_outside(cx, &ws, &root, &col, "dev");
    wait_for(cx, "prod selected", |cx| {
        selected(cx, &ed).as_deref() == Some("prod")
    });
    assert_eq!(ed.read_with(cx, |e, _| e.naming_label()), None);
}

#[gpui_kit::test]
fn a_create_prompt_survives_the_last_environment_going(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| w.click("env-create", cx))
        .unwrap();
    cx.run_until_parked();
    delete_outside(cx, &ws, &root, &col, "dev");
    wait_for(cx, "dev gone", |cx| selected(cx, &ed).is_none());
    assert_eq!(
        ed.read_with(cx, |e, _| e.naming_label()).as_deref(),
        Some("New environment name")
    );
    assert!(present(cx, win, "env-name-label"));
}

// A row typed into a dirty environment has no saved source, so Secret can be ticked on it:
// the value masks, the draft marks it secret, and Save sends the typed value to the store
// while the environment file only ever names the variable.
#[gpui_kit::test]
fn a_new_row_can_be_made_secret_before_the_first_save(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    type_at_end(cx, win, None, ("env-name", 2usize), "api_key");
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 2usize), "hunter2");
    cx.run_until_parked();
    assert!(env_view(cx, &ws, &root, "dev").dirty);
    let ticked = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.click(("env-secret", 2usize), cx);
            w.render_frame(cx);
            w.find(("env-secret", 2usize)).checked()
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ticked,
        Some(true),
        "a row with no saved source can be made secret"
    );
    let table = ed.read_with(cx, |e, _| e.table().clone());
    let input = table
        .read_with(cx, |t, _| t.value_input(2).cloned())
        .expect("the new row");
    assert!(input.read_with(cx, |i, _| i.presentation().is_masked()));
    let draft = env_view(cx, &ws, &root, "dev");
    let row = draft
        .variables
        .iter()
        .find(|v| v.name == "api_key")
        .expect("the draft holds the new row");
    assert!(row.secret, "the draft carries secret: true");
    assert!(!std::fs::read_to_string(&file).unwrap().contains("hunter2"));

    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(!saved.contains("hunter2"), "{saved}");
    assert!(saved.contains("name: api_key"), "{saved}");
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "api_key");
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("hunter2"));
    let dev = env_view(cx, &ws, &root, "dev");
    assert!(!dev.dirty);
    assert!(
        dev.variables
            .iter()
            .any(|v| v.name == "api_key" && v.secret)
    );
}

// Ticking Secret on a new row and unticking it again leaves an ordinary variable whose value
// reaches the file, not the store.
#[gpui_kit::test]
fn unticking_a_new_rows_secret_keeps_it_plain(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, _ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    type_at_end(cx, win, None, ("env-name", 2usize), "region");
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 2usize), "eu");
    cx.run_until_parked();
    for want in [true, false] {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.click(("env-secret", 2usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let checked = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.find(("env-secret", 2usize)).checked()
            })
            .unwrap();
        assert_eq!(
            checked,
            Some(want),
            "the new row's Secret follows each click"
        );
    }
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("value: eu"), "{saved}");
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "region");
    assert_eq!(store.get(&key).unwrap(), None);
    let dev = env_view(cx, &ws, &root, "dev");
    assert!(
        dev.variables
            .iter()
            .any(|v| v.name == "region" && !v.secret)
    );
}

// The lock is a Tab stop like every clickable control: Enter makes the row secret, Space plain.
#[gpui_kit::test]
fn the_secrecy_lock_is_reached_and_toggled_from_the_keyboard(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-name", 2usize), "region");
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 2usize), "eu");
    cx.run_until_parked();
    let focused = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("env-secret", 2usize)).focused() == Some(true)
        })
        .unwrap()
    };
    let secret = |cx: &mut TestAppContext| {
        env_view(cx, &ws, &root, "dev")
            .variables
            .iter()
            .find(|v| v.name == "region")
            .expect("the draft holds the new row")
            .secret
    };
    // Tab on from the value just typed until the row's lock holds focus; the bound only stops a loop.
    let mut stops = 0;
    while !focused(cx) {
        assert!(stops < 10, "Tab never reaches the row's lock");
        cx.update_window(win, |_, w, cx| w.press("tab", cx))
            .unwrap();
        cx.run_until_parked();
        stops += 1;
    }
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(secret(cx), "Enter makes the row secret");
    assert!(focused(cx), "a keyboard toggle keeps focus on the lock");
    cx.update_window(win, |_, w, cx| w.press("space", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!secret(cx), "Space makes it plain again");
}

// Renaming a saved secret makes its row unsaved under the new name, yet it stays secret: Save
// writes the new name with no value. The name cell of a secret row is locked, so the rename
// arrives as a table edit, as any draft change does.
#[gpui_kit::test]
fn renaming_a_saved_secret_row_keeps_it_secret_on_save(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    let table = ed.read_with(cx, |e, _| e.table().clone());
    table.update(cx, |t, cx| {
        let mut rows = t.rows(cx);
        rows[1].name = "token2".into();
        cx.emit(EnvTableEvent::Changed(rows));
    });
    cx.run_until_parked();
    let draft = env_view(cx, &ws, &root, "dev");
    assert!(draft.dirty && draft.variables[1].name == "token2");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("env-value", 0usize), cx);
        w.press(&key("s"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(
        saved.ends_with("  - secret: true\n    name: token2\n"),
        "{saved}"
    );
    let dev = env_view(cx, &ws, &root, "dev");
    assert!(!dev.dirty);
    assert!(dev.variables.iter().any(|v| v.name == "token2" && v.secret));
}

// Ticks Secret on new row `ix` of the dev editor, then types `value` into its masked input
// and blurs it by clicking the first row's value. Blur is seen by comparing drawn frames of
// an active window, so each focus change gets a frame.
fn secret_typed_then_blurred(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, value: &str) {
    cx.update_window(win, |_, w, cx| {
        w.activate_window();
        w.render_frame(cx);
        w.click(("env-secret", 2usize), cx);
    })
    .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, ("env-value", 2usize), value);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-value", 0usize), cx))
        .unwrap();
    frame(cx, win);
}

// An unsaved secret row's value waits for Save: blurring it stores nothing, so unticking
// before Save leaves no orphan in the store and the value lands in the file as plain text.
#[gpui_kit::test]
fn an_unsaved_secret_row_stores_nothing_until_save(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, _ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    let key_of = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "region");
    type_at_end(cx, win, None, ("env-name", 2usize), "region");
    cx.run_until_parked();
    secret_typed_then_blurred(cx, win, "eu");
    assert_eq!(store.get(&key_of).unwrap(), None, "blur stores nothing");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("env-secret", 2usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(store.get(&key_of).unwrap(), None);
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("name: region\n    value: eu"), "{saved}");
}

// The same row left secret reaches the store on Save and never the file.
#[gpui_kit::test]
fn an_unsaved_secret_row_blurred_then_saved_goes_to_the_store(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, _ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    let key_of = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "api_key");
    type_at_end(cx, win, None, ("env-name", 2usize), "api_key");
    cx.run_until_parked();
    secret_typed_then_blurred(cx, win, "hunter2");
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(store.get(&key_of).unwrap().as_deref(), Some("hunter2"));
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("name: api_key"), "{saved}");
    assert!(!saved.contains("hunter2"), "{saved}");
}

// Reveal and Hide on an unsaved secret row only unmask and mask its typing: nothing is in the
// store to show, and the typing must survive to Save.
#[gpui_kit::test]
fn reveal_and_hide_on_an_unsaved_secret_row_keep_its_typing(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    let key_of = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "api_key");
    type_at_end(cx, win, None, ("env-name", 2usize), "api_key");
    cx.run_until_parked();
    secret_typed_then_blurred(cx, win, "hunter2");
    let table = ed.read_with(cx, |e, _| e.table().clone());
    let input = table
        .read_with(cx, |t, _| t.value_input(2).cloned())
        .expect("the new row");
    for (button, masked) in [("env-reveal", false), ("env-hide", true)] {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.click((button, 2usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        input.read_with(cx, |i, _| {
            assert_eq!(i.presentation().is_masked(), masked, "after {button}");
            assert_eq!(i.value(), "hunter2", "after {button}");
        });
        assert_eq!(store.get(&key_of).unwrap(), None, "after {button}");
    }
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(store.get(&key_of).unwrap().as_deref(), Some("hunter2"));
}

// Switching environment neither saves nor discards the edit, so an unsaved secret row's value
// has no file to belong to yet and is dropped rather than stored.
#[gpui_kit::test]
fn switching_environment_drops_an_unsaved_secret_rows_value(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    let key_of = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "api_key");
    type_at_end(cx, win, None, ("env-name", 2usize), "api_key");
    cx.run_until_parked();
    secret_typed_then_blurred(cx, win, "hunter2");
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("prod", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(store.get(&key_of).unwrap(), None);
}

// Leaving a dirty environment and coming back must not promote its unsaved secret row to
// saved: the row stays unsaved, so typing into it again is still dropped on the next switch.
#[gpui_kit::test]
fn an_unsaved_secret_row_stays_unsaved_across_a_switch_and_back(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    let key_of = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "api_key");
    type_at_end(cx, win, None, ("env-name", 2usize), "api_key");
    cx.run_until_parked();
    secret_typed_then_blurred(cx, win, "hunter2");
    let select = |cx: &mut TestAppContext, name: &'static str| {
        cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.select(name, w, cx)))
            .unwrap();
        cx.run_until_parked();
        frame(cx, win);
    };
    select(cx, "prod");
    select(cx, "dev");
    let table = ed.read_with(cx, |e, _| e.table().clone());
    let row = table
        .read_with(cx, |t, cx| t.rows(cx))
        .into_iter()
        .find(|r| r.name == "api_key")
        .expect("the draft keeps the row");
    assert!(row.unsaved, "{row:?}");
    let enabled = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.click(("env-secret", 2usize), cx);
            w.render_frame(cx);
            w.find(("env-secret", 2usize)).checked()
        })
        .unwrap();
    assert_eq!(enabled, Some(false), "the toggle is not locked");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("env-secret", 2usize), cx);
    })
    .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, ("env-value", 2usize), "again");
    frame(cx, win);
    select(cx, "prod");
    assert_eq!(store.get(&key_of).unwrap(), None);
}

#[gpui_kit::test]
fn the_editor_is_a_list_a_divider_and_the_pane(cx: &mut TestAppContext) {
    use apinata_app::style::ENV_EDITOR_LIST_WIDTH;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let list = w.find("env-list").bounds();
        let divider = w.find("env-divider").bounds();
        let pane = w.find("env-pane").bounds();
        assert_eq!(list.size.width, at_zoom(ENV_EDITOR_LIST_WIDTH, 1.0));
        assert!(
            list.right() <= divider.left() && divider.right() <= pane.left(),
            "{list:?} {divider:?} {pane:?}"
        );
        assert!(divider.size.width <= px(1.), "{divider:?}");
        assert!(divider.size.height >= list.size.height, "{divider:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_mark_turns_muted_while_focus_is_in_the_inputs(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    let color = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| ed.read(cx).mark_color(w, cx))
            .unwrap()
    };
    let (primary, muted) = cx.update(|cx| (cx.theme().primary, cx.theme().muted_foreground));
    cx.update_window(win, |_, w, cx| w.click(("env-item", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(color(cx), primary, "the list holds focus");
    cx.update_window(win, |_, w, cx| w.click(("env-value", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(color(cx), muted, "focus is in a value");
    cx.update_window(win, |_, w, cx| w.click(("env-item", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(color(cx), primary);
}

#[gpui_kit::test]
fn the_empty_state_offers_a_new_environment_and_selects_it(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert!(present(cx, win, "env-empty"));
    cx.update_window(win, |_, w, cx| w.click("env-empty-create", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.naming_label()).as_deref(),
        Some("New environment name")
    );
    cx.update_window(win, |_, w, cx| {
        w.input("qa", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "qa selected", |cx| {
        selected(cx, &ed).as_deref() == Some("qa")
    });
    assert!(!present(cx, win, "env-empty"));
}

#[gpui_kit::test]
fn the_empty_action_sits_below_its_label_inside_a_narrow_pane(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    gpui_kit::VisualTestContext::from_window(win, cx)
        .simulate_resize(gpui_kit::size(px(420.), px(400.)));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let pane = w.find("env-empty-pane").bounds();
        let label = w.find("env-empty-label").bounds();
        let button = w.find("env-empty-create").bounds();
        assert!(button.top() >= label.bottom(), "{button:?} below {label:?}");
        assert!(
            (button.center().x - label.center().x).abs() < px(1.),
            "centred under it"
        );
        assert!(
            button.left() >= pane.left() && button.right() <= pane.right(),
            "{button:?} inside {pane:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_header_is_the_name_then_the_menu_and_save(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    let primary: gpui_kit::Hsla = cx.update(|cx| cx.theme().tokens.button_primary.into());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let title = w.find("env-title");
        assert_eq!(title.label(), Some("dev"));
        let more = w.find("env-more");
        assert_eq!(more.label(), Some(MORE_ACTIONS));
        let (t, m, save) = (title.bounds(), more.bounds(), w.find("env-save").bounds());
        assert!(
            t.right() <= m.left() && m.right() <= save.left(),
            "{t:?} {m:?} {save:?}"
        );
        for gone in ["env-rename", "env-export", "env-delete"] {
            assert!(w.try_find(gone).is_none(), "{gone} moved into the menu");
        }
    })
    .unwrap();
    assert!(
        !save_filled(cx, win, primary),
        "nothing to save: Save is plain"
    );
    ed.update(cx, |e, cx| e.set_color("#ff0000", cx));
    frame(cx, win);
    assert!(
        save_filled(cx, win, primary),
        "an edit to save: Save is primary"
    );
}

// Whether a quad of `primary` is painted under Save's centre in the next frame.
fn save_filled(cx: &mut TestAppContext, win: AnyWindowHandle, primary: gpui_kit::Hsla) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let save = w.find("env-save").bounds();
        let scale = w.scale_factor();
        let (cx_, cy) = (f32::from(save.center().x), f32::from(save.center().y));
        w.painted_quads().into_iter().any(|q| {
            let (x, y) = (q.bounds.origin.x.0 / scale, q.bounds.origin.y.0 / scale);
            let r = x + q.bounds.size.width.0 / scale;
            let b = y + q.bounds.size.height.0 / scale;
            q.background.as_solid() == Some(primary) && x <= cx_ && r >= cx_ && y <= cy && b >= cy
        })
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_menu_holds_rename_export_and_a_separated_delete(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| w.click("env-more", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let menu = w.within("popup-menu");
        assert_eq!(menu.find(ENV_MENU_RENAME).label(), Some("Rename"));
        assert_eq!(
            menu.find(ENV_MENU_EXPORT).label(),
            Some("Export to Postman…")
        );
        assert_eq!(
            menu.find(ENV_MENU_EXPORT_ALL).label(),
            Some("Export all to Postman…")
        );
        assert_eq!(menu.find("env-menu-delete").label(), Some("Delete"));
    })
    .unwrap();
    cx.update_window(win, |_, w, cx| {
        w.within("popup-menu").click(ENV_MENU_DELETE, cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-delete");
    assert!(!col.path().join("environments/dev.yml").exists());
    assert_eq!(selected(cx, &ed).as_deref(), Some("prod"));
}

#[gpui_kit::test]
fn a_dirty_environment_offers_rename_disabled_with_its_reason(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    assert_eq!(RENAME_LOCKED_REASON, "Save or discard changes first");
    env_menu(cx, win, ENV_MENU_RENAME);
    let reason = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("env-rename-locked").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(
        reason.as_deref(),
        Some(RENAME_LOCKED_REASON),
        "the entry names why"
    );
    // Disabled: the click ran nothing and left the menu open on the same entry.
    assert_eq!(ed.read_with(cx, |e, _| e.naming_label()), None);
    assert!(present(cx, win, "env-rename-locked"), "the menu stays open");
    assert!(!present(cx, win, "env-name-input"));
}

const COLORED: &str = "name: dev\ncolor: \"#123456\"\nvariables:\n  - name: host\n    value: x\n";

fn ringed(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.within(id).try_find("env-swatch-ring").is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_swatches_are_none_then_the_six_presets(cx: &mut TestAppContext) {
    assert_eq!(
        ENV_SWATCHES,
        [
            "#CE4F3B", "#2E8A54", "#346AB2", "#C77A0F", "#B83D7F", "#8D44B2"
        ]
    );
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("env-swatch-none").label(), Some("No color"));
        for (ix, name) in ENV_SWATCH_NAMES.iter().enumerate() {
            assert_eq!(w.find(("env-swatch", ix)).label(), Some(*name));
        }
        assert!(w.try_find(("env-swatch", 6usize)).is_none());
        assert!(w.try_find("env-swatch-custom").is_none());
        assert!(w.try_find("env-color").is_none());
    })
    .unwrap();
    assert!(ringed(cx, win, "env-swatch-none"), "dev has no colour");
}

#[test]
fn the_presets_are_named_in_plain_words() {
    assert_eq!(
        ENV_SWATCH_NAMES,
        ["Red", "Green", "Blue", "Orange", "Pink", "Purple"]
    );
}

#[gpui_kit::test]
fn the_list_shows_each_colour_and_keeps_the_names_in_line(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", COLORED),
        ("environments/prod.yml", "name: prod\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let mut dotted = |ix: usize| {
            w.within(("env-item", ix))
                .try_find("env-item-dot")
                .is_some()
        };
        assert!(dotted(0), "dev is coloured");
        assert!(!dotted(1), "prod has no colour");
        let left = |ix: usize| w.find(("env-item-name", ix)).bounds().left();
        assert_eq!(left(0), left(1));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_colour_off_the_presets_shows_as_its_own_swatch_and_round_trips(cx: &mut TestAppContext) {
    let make = || {
        write_tree(&[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", COLORED),
        ])
    };
    let (col, twin) = (make(), make());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    assert!(ringed(cx, win, "env-swatch-custom"));
    assert!(!ringed(cx, win, "env-swatch-none"));
    let label = cx
        .update_window(win, |_, w, _| {
            w.find("env-swatch-custom").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("#123456"));
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    let mut c = Collection::open(twin.path()).unwrap();
    let env = c.environment_mut("dev").unwrap();
    let mut vars = env.variables();
    vars[0].value = VarValue::Text("xy".into());
    env.set_variables(vars);
    env.save().unwrap();
    let read =
        |d: &tempfile::TempDir| std::fs::read(d.path().join("environments/dev.yml")).unwrap();
    assert_eq!(read(&col), read(&twin));
    assert!(String::from_utf8(read(&col)).unwrap().contains("#123456"));
}

#[gpui_kit::test]
fn picking_a_swatch_sets_the_colour_and_none_removes_the_key(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", COLORED),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| w.click(("env-swatch", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        env_view(cx, &ws, &root, "dev").color.as_deref(),
        Some("#CE4F3B")
    );
    assert!(ringed(cx, win, ("env-swatch", 0usize)));
    assert!(
        !present(cx, win, "env-swatch-custom"),
        "the colour is a preset now"
    );
    cx.update_window(win, |_, w, cx| w.click("env-swatch-none", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(env_view(cx, &ws, &root, "dev").color, None);
    assert!(ringed(cx, win, "env-swatch-none"));
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    let saved = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    assert!(!saved.contains("color"), "{saved}");
}

#[gpui_kit::test]
fn the_swatches_are_reached_and_picked_from_the_keyboard(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    let focused = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("env-swatch", ix)).focused() == Some(true)
        })
        .unwrap()
    };
    // Tab from the editor root until a preset swatch holds focus; the bound only stops a loop.
    let handle = ed.read_with(cx, |e, cx| e.focus_handle(cx));
    cx.update_window(win, |_, w, cx| w.focus(&handle, cx))
        .unwrap();
    let mut stops = 0;
    while !focused(cx, 2) {
        assert!(stops < 30, "Tab never reaches a preset swatch");
        cx.update_window(win, |_, w, cx| w.press("tab", cx))
            .unwrap();
        cx.run_until_parked();
        stops += 1;
    }
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    let color = |cx: &mut TestAppContext| env_view(cx, &ws, &root, "dev").color.clone();
    assert_eq!(color(cx).as_deref(), Some(ENV_SWATCHES[2]));
    cx.update_window(win, |_, w, cx| w.press("tab", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(focused(cx, 3), "Tab moves on to the next preset");
    cx.update_window(win, |_, w, cx| w.press("space", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(color(cx).as_deref(), Some(ENV_SWATCHES[3]));
    assert!(focused(cx, 3), "a keyboard pick keeps focus on the swatch");
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    let saved = Collection::open(col.path()).unwrap();
    let saved = saved.environment("dev").unwrap().color();
    assert_eq!(
        saved.as_deref(),
        Some(ENV_SWATCHES[3]),
        "Ctrl+S from a swatch saves"
    );
}

#[gpui_kit::test]
fn a_lowercase_preset_selects_its_swatch_without_a_rewrite(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\ncolor: \"#ce4f3b\"\n"),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    assert!(ringed(cx, win, ("env-swatch", 0usize)));
    assert!(!present(cx, win, "env-swatch-custom"));
    let dev = env_view(cx, &ws, &root, "dev");
    assert_eq!(dev.color.as_deref(), Some("#ce4f3b"));
    assert!(!dev.dirty);
}

#[gpui_kit::test]
fn a_malformed_file_colour_shows_as_a_custom_swatch_without_crashing(cx: &mut TestAppContext) {
    for bad in ["#1é234", "#ce4f3", "#ééé"] {
        let col = write_tree(&[
            ("opencollection.yml", ROOT),
            (
                "environments/dev.yml",
                &format!("name: dev\ncolor: \"{bad}\"\n"),
            ),
        ]);
        let (_state, ws, root) = open_dir(cx, &col);
        let (win, _ed) = env_editor(cx, &ws, &root);
        assert!(present(cx, win, "env-swatch-custom"), "{bad:?}");
        assert!(ringed(cx, win, "env-swatch-custom"), "{bad:?}");
    }
}

#[gpui_kit::test]
fn colour_and_extends_share_the_form_grid(cx: &mut TestAppContext) {
    use apinata_app::style::FORM_LABEL_WIDTH;
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let color = w.within("env-color-row").find("form-control").bounds();
        let extends = w.within("env-extends-row").find("form-control").bounds();
        assert_eq!(color.left(), extends.left());
        let label = w.within("env-color-row").find("form-label").bounds();
        assert_eq!(label.size.width, at_zoom(FORM_LABEL_WIDTH, 1.0));
        let select = w.find("env-extends").bounds();
        assert!(
            select.left() >= extends.left() - px(0.5),
            "{select:?} {extends:?}"
        );
    })
    .unwrap();
}

// WCAG relative luminance of `c` over an opaque backdrop.
fn luminance(c: gpui_kit::Hsla) -> f32 {
    let rgb = c.to_rgb();
    let lin = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb.r) + 0.7152 * lin(rgb.g) + 0.0722 * lin(rgb.b)
}

// The unselected None swatch's outline reads against the pane at the 3:1 that WCAG asks of
// a control's boundary, in both themes.
#[gpui_kit::test]
fn the_none_swatch_outline_is_visible_in_both_themes(cx: &mut TestAppContext) {
    use gpui_kit::component::{ActiveTheme as _, ThemeMode};
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", COLORED),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    assert!(!ringed(cx, win, "env-swatch-none"));
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        cx.update(|cx| apinata_app::theme::set_mode(mode, None, cx));
        let (outline, backdrop) = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                let dot = w.within("env-swatch-none").find("env-swatch-dot").bounds();
                let scale = w.scale_factor();
                let quad = w
                    .painted_quads()
                    .into_iter()
                    .find(|q| {
                        (q.bounds.origin.x.0 / scale - f32::from(dot.left())).abs() < 1.
                            && (q.bounds.origin.y.0 / scale - f32::from(dot.top())).abs() < 1.
                            && q.border_widths.left.0 > 0.
                    })
                    .expect("the dot's outline is painted");
                (quad.border_color, cx.theme().background)
            })
            .unwrap();
        let (a, b) = (luminance(outline), luminance(backdrop));
        let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(
            ratio >= 3.0,
            "{mode:?}: {ratio:.2} for {outline:?} on {backdrop:?}"
        );
    }
}

#[gpui_kit::test]
fn a_long_unbroken_error_wraps_and_leaves_the_pane_width_alone(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("prod", w, cx))
    })
    .unwrap();
    let widths = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("env-pane").bounds().size.width,
                w.find("env-list").bounds().size.width,
            )
        })
        .unwrap()
    };
    let before = widths(cx);
    // A refused rename echoes the name, so a long name with no spaces makes a long error.
    ed.update(cx, |e, cx| e.rename(&format!("a/{}", "b".repeat(600)), cx));
    cx.run_until_parked();
    assert!(ed.read_with(cx, |e, _| e.error()).is_some(), "an error");
    assert_eq!(widths(cx), before, "the error wraps inside the pane");
}

#[gpui_kit::test]
fn the_duplicate_button_copies_the_selected_environment_and_selects_the_copy(
    cx: &mut TestAppContext,
) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        // A button of the header, not an entry of the "…" menu.
        assert_eq!(w.find("env-duplicate").label(), Some("Duplicate"));
        w.click("env-duplicate", cx)
    })
    .unwrap();
    wait_for(cx, "copy selected", |cx| {
        selected(cx, &ed).as_deref() == Some("dev copy")
    });
    let copy = col.path().join("environments/dev copy.yml");
    assert_eq!(
        std::fs::read_to_string(copy).expect("copy"),
        DEV.replacen("name: dev\n", "name: dev copy\n", 1)
    );
}

// How the person throws the draft away: the banner's Reload, or a Revert sent from elsewhere
// (the tab's Discard changes sends the same command).
#[derive(Clone, Copy)]
enum Revert {
    Banner,
    Sent,
}

// Types into a secret row (sent to the store with Enter when `send_secret`), changes the file
// outside, reverts, then presses Save. Returns the store value and the secret input's text.
fn revert_after_outside_change(
    cx: &mut TestAppContext,
    send_secret: bool,
    how: Revert,
) -> (Option<String>, String) {
    let col = tree();
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    // A plain edit makes the environment a draft, so an outside change raises the banner.
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 1usize), "typed-secret");
    if send_secret {
        cx.update_window(win, |_, w, cx| w.press("enter", cx))
            .unwrap();
    }
    cx.run_until_parked();
    let path = col.path().join("environments/dev.yml");
    std::fs::write(&path, format!("{DEV}  - name: extra\n    value: z\n")).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![path.clone()]), cx)
    });
    cx.run_until_parked();
    assert!(present(cx, win, "banner-changed"));
    match how {
        Revert::Banner => cx
            .update_window(win, |_, w, cx| w.click("banner-reload", cx))
            .unwrap(),
        Revert::Sent => {
            let target = apinata_app::command::Target::Environment("dev".into());
            ws.update(cx, |w, cx| w.send(&root, Command::Revert(target), cx));
        }
    }
    cx.run_until_parked();
    let shown = ed
        .read_with(cx, |e, _| e.table().clone())
        .read_with(cx, |t, _| t.value_input(1).cloned())
        .map(|i| i.read_with(cx, |i, _| i.value().to_string()))
        .unwrap_or_default();
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "token");
    (store.get(&key).unwrap(), shown)
}

#[gpui_kit::test]
fn reload_drops_secret_typing_not_yet_sent(cx: &mut TestAppContext) {
    let (stored, shown) = revert_after_outside_change(cx, false, Revert::Banner);
    assert_eq!(shown, "", "the secret input is cleared by the reload");
    assert_eq!(stored, None, "a later Save stores nothing");
}

#[gpui_kit::test]
fn a_revert_sent_from_outside_the_banner_drops_unsent_secret_typing(cx: &mut TestAppContext) {
    let (stored, shown) = revert_after_outside_change(cx, false, Revert::Sent);
    assert_eq!(shown, "", "the secret input is cleared by the revert");
    assert_eq!(stored, None, "a later Save stores nothing");
}

// A secret already sent is stored at once ("a secret value is not a draft"). Reload clears the
// input but does NOT restore the store's earlier value: the sent value stays stored.
#[gpui_kit::test]
fn reload_clears_the_input_of_a_secret_already_sent_and_keeps_the_store(cx: &mut TestAppContext) {
    let (stored, shown) = revert_after_outside_change(cx, true, Revert::Banner);
    assert_eq!(shown, "", "the secret input is cleared by the reload");
    assert_eq!(stored.as_deref(), Some("typed-secret"), "not restored");
}

// Types into dev's secret row, reverts prod, then edits a plain row so the editor refreshes.
fn type_secret_and_republish(
    cx: &mut TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    win: AnyWindowHandle,
    ed: &gpui_kit::Entity<apinata_app::env_editor::EnvEditor>,
) -> String {
    type_at_end(cx, win, None, ("env-value", 1usize), "fresh-typing");
    let target = apinata_app::command::Target::Environment("prod".into());
    ws.update(cx, |w, cx| w.send(root, Command::Revert(target), cx));
    // A plain edit publishes a snapshot, which makes the editor refresh.
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    ed.read_with(cx, |e, _| e.table().clone())
        .read_with(cx, |t, _| t.value_input(1).cloned())
        .map(|i| i.read_with(cx, |i, _| i.value().to_string()))
        .unwrap_or_default()
}

#[gpui_kit::test]
fn a_revert_of_another_environment_keeps_the_typing_here(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(selected(cx, &ed).as_deref(), Some("dev"));
    let shown = type_secret_and_republish(cx, &ws, &root, win, &ed);
    assert_eq!(shown, "fresh-typing", "prod's revert is not dev's");
}

// Typing entered after a Revert of the shown environment was handled belongs to no revert:
// the next refresh keeps it.
#[gpui_kit::test]
fn typing_entered_after_an_earlier_revert_is_kept(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    let target = apinata_app::command::Target::Environment("dev".into());
    ws.update(cx, |w, cx| w.send(&root, Command::Revert(target), cx));
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 1usize), "fresh-typing");
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    assert_eq!(
        secret_text(cx, &ed, 1),
        "fresh-typing",
        "an earlier revert wipes nothing"
    );
}

// The text in secret row `ix`'s input.
fn secret_text(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::env_editor::EnvEditor>,
    ix: usize,
) -> String {
    ed.read_with(cx, |e, _| e.table().clone())
        .read_with(cx, |t, _| t.value_input(ix).cloned())
        .map(|i| i.read_with(cx, |i, _| i.value().to_string()))
        .unwrap_or_default()
}

// An open dev editor whose secret `token` holds unsent typing when dev.yml changes outside.
// With `draft`, a plain edit first makes dev a draft, so the change raises the banner.
struct Typed {
    col: tempfile::TempDir,
    _state: tempfile::TempDir,
    store: Arc<MemoryStore>,
    ws: gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: gpui_kit::Entity<apinata_app::env_editor::EnvEditor>,
    dev: String,
}

impl Typed {
    fn new(cx: &mut TestAppContext, dev: &str, draft: bool) -> Typed {
        let col = write_tree(&[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", dev),
            ("environments/prod.yml", "name: prod\n"),
        ]);
        let store = Arc::new(MemoryStore::new());
        let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
        let (win, ed) = env_editor(cx, &ws, &root);
        cx.update_window(win, |_, w, _| w.activate_window())
            .unwrap();
        frame(cx, win);
        if draft {
            type_at_end(cx, win, None, ("env-value", 0usize), "y");
            cx.run_until_parked();
        }
        Typed {
            col,
            _state,
            store,
            ws,
            root,
            win,
            ed,
            dev: dev.to_string(),
        }
    }

    fn type_secret(&self, cx: &mut TestAppContext) {
        type_at_end(cx, self.win, None, ("env-value", 1usize), "typed-secret");
        frame(cx, self.win);
    }

    fn path(&self) -> std::path::PathBuf {
        self.col.path().join("environments/dev.yml")
    }

    // Rewrites dev.yml with an extra variable and makes the actor read it.
    fn change_outside(&self, cx: &mut TestAppContext) {
        let path = self.path();
        let text = format!("{}  - name: extra\n    value: z\n", self.dev);
        std::fs::write(&path, text).unwrap();
        self.reload(cx);
    }

    fn reload(&self, cx: &mut TestAppContext) {
        let path = self.path();
        self.ws.update(cx, |w, cx| {
            w.send(&self.root, Command::Reload(vec![path]), cx)
        });
        cx.run_until_parked();
        frame(cx, self.win);
    }

    fn stored(&self) -> Option<String> {
        let key = Collection::open(self.col.path())
            .unwrap()
            .secret_key("dev", "token");
        self.store.get(&key).unwrap()
    }

    fn text(&self, cx: &mut TestAppContext) -> String {
        secret_text(cx, &self.ed, 1)
    }

    fn save(&self, cx: &mut TestAppContext) {
        cx.update_window(self.win, |_, w, cx| w.press(&key("s"), cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn label(&self, cx: &mut TestAppContext, id: &'static str) -> Option<String> {
        cx.update_window(self.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find(id).and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap()
    }
}

// (a) The banner's Reload clears the typing at once, before the actor answers.
#[gpui_kit::test]
fn banner_reload_clears_unsent_typing_at_once(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    t.change_outside(cx);
    cx.update_window(t.win, |_, w, cx| w.click("banner-reload", cx))
        .unwrap();
    assert_eq!(t.text(cx), "", "cleared before the reload is acknowledged");
    cx.run_until_parked();
    t.save(cx);
    assert_eq!(t.stored(), None, "a later Save stores nothing");
}

// (b) Opening Compare takes focus from the secret input; that blur must not store the typing
// the outside change is about to replace, and the dialog's Reload then clears it.
#[gpui_kit::test]
fn compare_then_reload_never_stores_unsent_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    t.change_outside(cx);
    cx.update_window(t.win, |_, w, cx| w.click("banner-compare", cx))
        .unwrap();
    frame(cx, t.win);
    // The real dialog takes focus; move it off the input in case the test window kept it.
    let ed = t.ed.clone();
    cx.update_window(t.win, |_, w, cx| {
        let focus = ed.read(cx).focus_handle(cx);
        w.focus(&focus, cx);
    })
    .unwrap();
    frame(cx, t.win);
    assert_eq!(t.stored(), None, "the blur stored nothing");
    let dialog = cx
        .windows()
        .into_iter()
        .find(|&win| {
            cx.update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.try_find("compare-reload").is_some()
            })
            .unwrap_or(false)
        })
        .expect("the compare dialog is open");
    cx.update_window(dialog, |_, w, cx| w.click("compare-reload", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(t.text(cx), "", "the dialog's Reload clears the input");
    t.save(cx);
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// (c) A Revert sent from elsewhere clears the typing before any later input, so selecting
// the same environment before the actor answers stores nothing.
#[gpui_kit::test]
fn a_click_before_the_revert_is_acknowledged_stores_nothing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    t.change_outside(cx);
    let target = apinata_app::command::Target::Environment("dev".into());
    t.ws.update(cx, |w, cx| w.send(&t.root, Command::Revert(target), cx));
    cx.update_window(t.win, |_, w, cx| w.click(("env-item", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    t.save(cx);
    assert_eq!(t.text(cx), "", "the revert cleared the input");
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// (d) Typing alone makes no draft, so an outside change reloads silently; the typing it
// would have kept is cleared and the editor says so.
#[gpui_kit::test]
fn a_silent_reload_clears_unsent_typing_and_says_so(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    assert_eq!(t.label(cx, "env-notice"), None);
    t.type_secret(cx);
    t.change_outside(cx);
    assert_eq!(t.text(cx), "", "the reload cleared the typing");
    assert_eq!(
        t.label(cx, "env-notice").as_deref(),
        Some("This file changed on disk. Unsent secret values were cleared.")
    );
    t.save(cx);
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// The editor's own plain edit changes the rows too; that echo is not an outside change.
#[gpui_kit::test]
fn the_editors_own_edit_keeps_unsent_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    t.type_secret(cx);
    type_at_end(cx, t.win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    assert_eq!(t.text(cx), "typed-secret");
    assert_eq!(t.label(cx, "env-notice"), None);
}

// A rename the editor sent comes back as the same rows under a new name; typing stays.
#[gpui_kit::test]
fn the_editors_own_rename_keeps_unsent_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    t.type_secret(cx);
    t.ed.update(cx, |e, cx| e.rename("staging", cx));
    cx.run_until_parked();
    frame(cx, t.win);
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("staging"));
    assert_eq!(
        t.label(cx, "env-notice"),
        None,
        "no outside change was seen"
    );
}

// Discard on a deleted environment's banner is a Revert too; it drops the typing.
#[gpui_kit::test]
fn discard_of_a_removed_environment_stores_no_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    std::fs::remove_file(t.path()).unwrap();
    t.reload(cx);
    assert!(present(cx, t.win, "banner-deleted"));
    cx.update_window(t.win, |_, w, cx| w.click("banner-discard", cx))
        .unwrap();
    assert_eq!(t.text(cx), "", "cleared before the discard is acknowledged");
    cx.run_until_parked();
    t.save(cx);
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// The selected environment deleted outside takes its typing with it; nothing is stored under
// its name or the next one's.
#[gpui_kit::test]
fn an_environment_deleted_outside_stores_no_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    t.type_secret(cx);
    std::fs::remove_file(t.path()).unwrap();
    t.reload(cx);
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("prod"));
    t.save(cx);
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// While the file is changed on disk a blur holds the typing: leaving the window blurs the
// input, and that must not store it.
#[gpui_kit::test]
fn leaving_the_window_while_held_stores_nothing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    t.change_outside(cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(t.win, cx);
    vcx.deactivate_window();
    frame(cx, t.win);
    assert_eq!(t.stored(), None, "the blur stored nothing");
    assert_eq!(t.text(cx), "typed-secret", "the typing is still shown");
}

// Clicking the environment already shown is no switch, so it stores nothing a Reload drops.
#[gpui_kit::test]
fn reselecting_the_shown_environment_under_a_banner_stores_nothing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    t.change_outside(cx);
    cx.update_window(t.win, |_, w, cx| w.click(("env-item", 0usize), cx))
        .unwrap();
    frame(cx, t.win);
    assert_eq!(t.text(cx), "typed-secret", "the reselect kept the typing");
    cx.update_window(t.win, |_, w, cx| w.click("banner-reload", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(t.text(cx), "", "the reload cleared the input");
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// Focus leaving the secret input under a deleted or broken banner holds the typing, so the
// banner's Discard leaves no orphan in the store.
fn blur_then_discard(cx: &mut TestAppContext, gone: bool) -> Option<String> {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    if gone {
        std::fs::remove_file(t.path()).unwrap();
    } else {
        std::fs::write(t.path(), "name: [dev\nvariables: {\n").unwrap();
    }
    t.reload(cx);
    let banner = if gone {
        "banner-deleted"
    } else {
        "banner-broken"
    };
    assert!(present(cx, t.win, banner));
    let ed = t.ed.clone();
    cx.update_window(t.win, |_, w, cx| {
        let focus = ed.read(cx).focus_handle(cx);
        w.focus(&focus, cx);
    })
    .unwrap();
    frame(cx, t.win);
    cx.update_window(t.win, |_, w, cx| w.click("banner-discard", cx))
        .unwrap();
    cx.run_until_parked();
    t.stored()
}

#[gpui_kit::test]
fn a_blur_under_the_deleted_banner_then_discard_stores_nothing(cx: &mut TestAppContext) {
    assert_eq!(blur_then_discard(cx, true), None);
}

#[gpui_kit::test]
fn a_blur_under_the_broken_banner_then_discard_stores_nothing(cx: &mut TestAppContext) {
    assert_eq!(blur_then_discard(cx, false), None);
}

// Overwrite writes the draft as Save does, so it stores the typing as Save does.
#[gpui_kit::test]
fn banner_overwrite_stores_the_typed_secret(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    t.change_outside(cx);
    cx.update_window(t.win, |_, w, cx| w.click("banner-overwrite", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(t.stored().as_deref(), Some("typed-secret"));
}

// A Reload restores no secret already sent, and the banner says so when there are secrets.
#[gpui_kit::test]
fn the_banner_says_sent_secrets_are_kept(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.change_outside(cx);
    assert!(present(cx, t.win, "banner-changed"));
    assert_eq!(
        t.label(cx, "env-secrets-kept").as_deref(),
        Some("Secret values already sent are kept.")
    );
}

#[gpui_kit::test]
fn the_banner_omits_the_secrets_line_without_secrets(cx: &mut TestAppContext) {
    let t = Typed::new(
        cx,
        "name: dev\nvariables:\n  - name: host\n    value: x\n",
        true,
    );
    t.change_outside(cx);
    assert!(present(cx, t.win, "banner-changed"));
    assert_eq!(t.label(cx, "env-secrets-kept"), None);
}

// Save under the deleted banner writes the draft back, so it stores the typing as Save does.
#[gpui_kit::test]
fn save_under_the_deleted_banner_stores_the_typed_secret(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    t.type_secret(cx);
    std::fs::remove_file(t.path()).unwrap();
    t.reload(cx);
    assert!(present(cx, t.win, "banner-deleted"));
    t.save(cx);
    assert_eq!(t.stored().as_deref(), Some("typed-secret"));
}

// Two secrets: `token` at row 1 and `key` at row 2.
const DEV2: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n  - secret: true\n    name: key\n";

fn stored_as(t: &Typed, name: &str) -> Option<String> {
    let key = Collection::open(t.col.path())
        .unwrap()
        .secret_key("dev", name);
    t.store.get(&key).unwrap()
}

fn notice(t: &Typed, cx: &mut TestAppContext) -> Option<String> {
    t.label(cx, "env-notice")
}

// An outside change that lands while the editor's own secret send is still unacknowledged
// is still an outside change: the typing in the other secret is cleared.
#[gpui_kit::test]
fn an_outside_change_during_an_unacked_secret_send_clears_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV2, false);
    type_at_end(cx, t.win, None, ("env-value", 1usize), "sent-secret");
    cx.update_window(t.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    type_at_end(cx, t.win, None, ("env-value", 2usize), "typed-key");
    let text = format!("{DEV2}  - name: extra\n    value: z\n");
    std::fs::write(t.path(), text).unwrap();
    t.reload(cx);
    assert_eq!(
        secret_text(cx, &t.ed, 2),
        "",
        "the reload cleared the typing"
    );
    assert_eq!(
        notice(&t, cx).as_deref(),
        Some("This file changed on disk. Unsent secret values were cleared.")
    );
    t.save(cx);
    assert_eq!(stored_as(&t, "key"), None, "nothing reached the store");
}

// A file removed and written again before one coalesced reload has new bytes, so core reports
// it as Modified and the stamp rises.
#[gpui_kit::test]
fn a_remove_and_recreate_in_one_reload_clears_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    t.type_secret(cx);
    std::fs::remove_file(t.path()).unwrap();
    std::fs::write(t.path(), format!("{DEV}  - name: extra\n    value: z\n")).unwrap();
    t.reload(cx);
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("dev"));
    assert_eq!(t.text(cx), "", "the reload cleared the typing");
    assert_eq!(
        notice(&t, cx).as_deref(),
        Some("This file changed on disk. Unsent secret values were cleared.")
    );
    t.save(cx);
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// Removal in its own reload moves the editor off dev, so the recreated dev shows no typing.
#[gpui_kit::test]
fn a_remove_then_recreate_in_two_reloads_leaves_no_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    t.type_secret(cx);
    std::fs::remove_file(t.path()).unwrap();
    t.reload(cx);
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("prod"));
    std::fs::write(t.path(), format!("{DEV}  - name: extra\n    value: z\n")).unwrap();
    t.reload(cx);
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("prod"));
    cx.update_window(t.win, |_, w, cx| w.click(("env-item", 0usize), cx))
        .unwrap();
    frame(cx, t.win);
    assert_eq!(selected(cx, &t.ed).as_deref(), Some("dev"));
    assert_eq!(t.text(cx), "");
    t.save(cx);
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// The editor's own Save and its watcher echo are not an outside change.
#[gpui_kit::test]
fn the_editors_own_save_keeps_unsent_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    type_at_end(cx, t.win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    t.save(cx);
    t.type_secret(cx);
    t.reload(cx);
    assert_eq!(
        notice(&t, cx),
        None,
        "no command since the echo cleared a notice"
    );
    assert_eq!(t.text(cx), "typed-secret");
}

// A secret send never touches the file, so a reload after it changes nothing and raises no
// false positive: the typing in another secret stays.
#[gpui_kit::test]
fn a_no_op_reload_after_an_own_secret_send_keeps_unsent_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV2, false);
    type_at_end(cx, t.win, None, ("env-value", 1usize), "sent-secret");
    cx.update_window(t.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    type_at_end(cx, t.win, None, ("env-value", 2usize), "typed-key");
    t.reload(cx);
    assert_eq!(secret_text(cx, &t.ed, 2), "typed-key");
    assert_eq!(notice(&t, cx), None);
    assert_eq!(stored_as(&t, "token").as_deref(), Some("sent-secret"));
}

// An outside change and a plain edit sent before it was seen fold into one snapshot; the actor
// holds the edit as stale, and the outside change still clears the typing.
#[gpui_kit::test]
fn an_outside_change_batched_with_an_edit_clears_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    t.type_secret(cx);
    std::fs::write(t.path(), format!("{DEV}  - name: extra\n    value: z\n")).unwrap();
    let path = t.path();
    t.ws.update(cx, |w, cx| {
        w.send(&t.root, Command::Reload(vec![path]), cx);
        let edit = apinata_app::command::Command::Edit {
            target: apinata_app::command::Target::Environment("dev".into()),
            edit: apinata_app::command::Edit::Variables(vec![
                apinata_core::EnvVariable::new("host", VarValue::Text("xy".into())),
                apinata_core::EnvVariable::secret("token"),
                apinata_core::EnvVariable::new("extra", VarValue::Text("z".into())),
            ]),
        };
        w.send(&t.root, edit, cx)
    });
    cx.run_until_parked();
    frame(cx, t.win);
    assert_eq!(t.text(cx), "", "the outside change cleared the typing");
    assert_eq!(
        notice(&t, cx).as_deref(),
        Some("This file changed on disk. Unsent secret values were cleared.")
    );
}

// Seeds the store with `token`'s value and reveals row 1, so the row shows it unmasked.
fn reveal_stored(t: &Typed, cx: &mut TestAppContext) {
    let key = Collection::open(t.col.path())
        .unwrap()
        .secret_key("dev", "token");
    t.store.set(&key, "stored-token").unwrap();
    cx.update_window(t.win, |_, w, cx| w.click(("env-reveal", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(t.text(cx), "stored-token");
    assert!(present(cx, t.win, ("env-hide", 1usize)));
}

// Row `ix` is in the state Hide leaves: masked, dotted placeholder, eye offering Show.
fn assert_masked(t: &Typed, cx: &mut TestAppContext, ix: usize) {
    let table = t.ed.read_with(cx, |e, _| e.table().clone());
    assert!(!table.read_with(cx, |t, _| t.revealed(ix)), "not revealed");
    let input = table
        .read_with(cx, |t, _| t.value_input(ix).cloned())
        .unwrap();
    assert!(input.read_with(cx, |i, _| i.presentation().is_masked()));
    assert_eq!(
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string()),
        apinata_app::env_table::SECRET_HIDDEN,
        "a stored secret never reads Not set"
    );
    assert!(present(cx, t.win, ("env-reveal", ix)));
    assert!(!present(cx, t.win, ("env-hide", ix)));
}

#[gpui_kit::test]
fn an_outside_change_with_typing_elsewhere_masks_a_revealed_row(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV2, false);
    reveal_stored(&t, cx);
    type_at_end(cx, t.win, None, ("env-value", 2usize), "typed-key");
    frame(cx, t.win);
    t.change_outside(cx);
    assert_masked(&t, cx, 1);
    assert_eq!(
        notice(&t, cx).as_deref(),
        Some("This file changed on disk. Unsent secret values were cleared.")
    );
}

#[gpui_kit::test]
fn an_outside_change_without_typing_masks_a_revealed_row_silently(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    reveal_stored(&t, cx);
    t.change_outside(cx);
    assert_masked(&t, cx, 1);
    assert_eq!(notice(&t, cx), None, "no typing was dropped");
}

#[gpui_kit::test]
fn a_banner_revert_masks_a_revealed_row(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, true);
    reveal_stored(&t, cx);
    t.change_outside(cx);
    cx.update_window(t.win, |_, w, cx| w.click("banner-reload", cx))
        .unwrap();
    cx.run_until_parked();
    assert_masked(&t, cx, 1);
}

#[gpui_kit::test]
fn a_reveal_after_an_outside_change_shows_the_stored_value(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    reveal_stored(&t, cx);
    t.change_outside(cx);
    cx.update_window(t.win, |_, w, cx| w.click(("env-reveal", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(t.text(cx), "stored-token");
    assert!(present(cx, t.win, ("env-hide", 1usize)));
}

// A plain edit's echo carries no raised stamp, so it is not an outside change.
#[gpui_kit::test]
fn the_editors_own_edit_keeps_a_reveal(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    reveal_stored(&t, cx);
    type_at_end(cx, t.win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    let table = t.ed.read_with(cx, |e, _| e.table().clone());
    assert!(table.read_with(cx, |t, _| t.revealed(1)));
    assert_eq!(t.text(cx), "stored-token");
}

// The reset empties the focused input, so focus leaves it: a key and a blur after it must not
// replace the stored secret.
#[gpui_kit::test]
fn an_outside_change_moves_focus_off_a_revealed_value(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    reveal_stored(&t, cx);
    cx.update_window(t.win, |_, w, cx| w.click(("env-value", 1usize), cx))
        .unwrap();
    frame(cx, t.win);
    t.change_outside(cx);
    let table = t.ed.read_with(cx, |e, _| e.table().clone());
    let input = table
        .read_with(cx, |t, _| t.value_input(1).cloned())
        .unwrap();
    let focused = cx
        .update_window(t.win, |_, w, cx| {
            input.read(cx).focus_handle(cx).is_focused(w)
        })
        .unwrap();
    assert!(!focused, "the emptied input lost focus");
    cx.update_window(t.win, |_, w, cx| w.input("z", cx))
        .unwrap();
    cx.update_window(t.win, |_, w, cx| w.click(("env-value", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(t.stored().as_deref(), Some("stored-token"));
}

// Writes `rel` in the collection behind the app's back and makes the actor read it.
fn write_outside(t: &Typed, cx: &mut TestAppContext, rel: &str, text: &str) {
    let path = t.col.path().join(rel);
    std::fs::write(&path, text).unwrap();
    t.ws.update(cx, |w, cx| w.send(&t.root, Command::Reload(vec![path]), cx));
    cx.run_until_parked();
    frame(cx, t.win);
}

type Landed = fn(&apinata_app::snapshot::CollectionSnapshot) -> bool;

// Typing in dev's secret survives a reload of another file, and the blur still stores it.
fn kept_through_a_reload_of(cx: &mut TestAppContext, rel: &str, text: &str, landed: Landed) {
    let t = Typed::new(cx, DEV, false);
    t.type_secret(cx);
    write_outside(&t, cx, rel, text);
    assert!(landed(&ws_snap(cx, &t.ws, &t.root)), "the reload landed");
    assert_eq!(t.text(cx), "typed-secret", "the reload replaced the typing");
    assert_eq!(notice(&t, cx), None);
    cx.update_window(t.win, |_, w, cx| w.click(("env-value", 0usize), cx))
        .unwrap();
    frame(cx, t.win);
    assert_eq!(t.stored().as_deref(), Some("typed-secret"));
}

#[gpui_kit::test]
fn unsent_secret_typing_survives_a_reload_of_another_environment(cx: &mut TestAppContext) {
    let prod = "name: prod\nvariables:\n  - name: host\n    value: p\n";
    kept_through_a_reload_of(cx, "environments/prod.yml", prod, |s| {
        (s.content.environments.iter()).any(|e| e.name == "prod" && !e.variables.is_empty())
    });
}

#[gpui_kit::test]
fn unsent_secret_typing_survives_a_reload_of_a_request(cx: &mut TestAppContext) {
    kept_through_a_reload_of(cx, "a.yml", &request("a", "http://x/2"), |s| {
        s.content.requests.contains_key(&ip("a.yml"))
    });
}

// The outside change made `token` plain, so the row shows the file's value, not the typing.
#[gpui_kit::test]
fn an_outside_change_making_the_secret_plain_replaces_the_typing(cx: &mut TestAppContext) {
    let t = Typed::new(cx, DEV, false);
    t.type_secret(cx);
    let plain = DEV.replace(
        "  - secret: true\n    name: token\n",
        "  - name: token\n    value: disk\n",
    );
    write_outside(&t, cx, "environments/dev.yml", &plain);
    assert_eq!(t.text(cx), "disk");
    assert_eq!(t.stored(), None, "nothing reached the store");
}

// The value input of row `ix` as the open editor shows it.
fn shown_value(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::env_editor::EnvEditor>,
    ix: usize,
) -> String {
    let table = ed.read_with(cx, |e, _| e.table().clone());
    table
        .read_with(cx, |t, _| t.value_input(ix).cloned())
        .map(|i| i.read_with(cx, |i, _| i.value().to_string()))
        .unwrap_or_default()
}

const OUTSIDE: &str = "name: dev\nvariables:\n  - name: host\n    value: outside\n";

#[gpui_kit::test]
fn an_outside_edit_shows_in_the_open_editor_without_a_click(cx: &mut TestAppContext) {
    let col = tree();
    let w = watched(cx, &col);
    let (win, ed) = env_editor(cx, &w.ws, &w.root);
    assert_eq!(shown_value(cx, &ed, 0), "x");
    std::fs::write(w.root.join("environments/dev.yml"), OUTSIDE).unwrap();
    wait_for(cx, "the outside value in the editor", |cx| {
        frame(cx, win);
        shown_value(cx, &ed, 0) == "outside"
    });
}

// Typing makes a draft, so the same edit raises the disk banner and leaves the typing alone.
#[gpui_kit::test]
fn an_outside_edit_under_a_draft_raises_the_banner_and_keeps_the_typing(cx: &mut TestAppContext) {
    let col = tree();
    let w = watched(cx, &col);
    let (win, ed) = env_editor(cx, &w.ws, &w.root);
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    std::fs::write(w.root.join("environments/dev.yml"), OUTSIDE).unwrap();
    wait_for(cx, "the changed-on-disk banner", |cx| {
        present(cx, win, "banner-changed")
    });
    assert_eq!(shown_value(cx, &ed, 0), "xy");
}

// Another program's rewrite of a collection environment reaches the open editor promptly.
#[gpui_kit::test]
fn an_outside_edit_reaches_the_open_environment_editor_promptly(cx: &mut TestAppContext) {
    let col = tree();
    let w = watched(cx, &col);
    let (_win, ed) = env_editor(cx, &w.ws, &w.root);
    let table = ed.read_with(cx, |e, _| e.table().clone());
    settle(cx);
    let written = std::time::Instant::now();
    let outside = DEV.replace("value: x", "value: outside");
    std::fs::write(col.path().join("environments/dev.yml"), outside).unwrap();
    wait_for(cx, "the outside value in the environment editor", |cx| {
        table.read_with(cx, |t, cx| t.rows(cx)[0].value == "outside")
    });
    assert!(written.elapsed() < std::time::Duration::from_secs(3));
}

fn discard_disabled(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find("env-discard").disabled() == Some(true)
    })
    .unwrap()
}

#[gpui_kit::test]
fn discard_changes_by_save_restores_the_saved_environment(cx: &mut TestAppContext) {
    let col = tree();
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    let label = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("env-discard").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Discard changes"));
    assert!(discard_disabled(cx, win), "nothing to discard");
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    assert!(env_view(cx, &ws, &root, "dev").dirty);
    assert!(!discard_disabled(cx, win));
    cx.update_window(win, |_, w, cx| w.click("env-discard", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx), "discarding asks first");
    answer_confirm(cx, "confirm-discard");
    let view = env_view(cx, &ws, &root, "dev");
    assert!(!view.dirty);
    assert!(matches!(&view.variables[0].value, VarValue::Text(v) if v == "x"));
    assert!(discard_disabled(cx, win));
    assert_eq!(
        std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap(),
        DEV
    );
}
