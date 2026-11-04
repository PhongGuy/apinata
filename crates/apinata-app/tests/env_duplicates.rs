//! Two secrets sharing one name in an environment: both rows are marked and every write that
//! would store the collision is refused, in a collection environment and a global one.

use crate::support;

use apinata_app::command::{Command, Target};
use apinata_app::env_table::{DUPLICATE_SECRET_NAME, DUPLICATE_SECRET_NAMES};
use apinata_app::global_env_editor::open_global_env_editor;
use apinata_core::{Collection, MemoryStore, SecretStore};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, WindowOptions};
use std::sync::Arc;
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";

/// The rows among the first `rows` that carry a duplicate note.
fn marked(cx: &mut TestAppContext, win: AnyWindowHandle, rows: usize) -> Vec<usize> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        (0..rows)
            .filter(|ix| w.try_find(("env-name-duplicate", *ix)).is_some())
            .collect()
    })
    .unwrap()
}

fn lock(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("env-secret", ix), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn save(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
}

const TWINS: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: tok\n  - secret: true\n    name: tok\n";

/// Rows 1 and 2 typed as new secrets named `tok` holding `first` and `second`.
fn type_twins(cx: &mut TestAppContext, win: AnyWindowHandle) {
    for (ix, value) in [(1usize, "first"), (2, "second")] {
        type_at_end(cx, win, None, ("env-name", ix), "tok");
        cx.run_until_parked();
        lock(cx, win, ix);
        type_at_end(cx, win, None, ("env-value", ix), value);
        cx.run_until_parked();
    }
}

fn tok_key(col: &tempfile::TempDir) -> apinata_core::SecretKey {
    Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "tok")
}

fn refused(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::env_editor::EnvEditor>,
) -> bool {
    ed.read_with(cx, |e, _| e.error()).as_deref() == Some(DUPLICATE_SECRET_NAMES)
}

// Two new rows locked under one name would share one stored value, so both are marked, Save
// writes neither the file nor the store, and renaming one away lets Save through.
#[gpui_kit::test]
fn two_secrets_of_one_name_are_marked_and_save_refuses_them(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    type_at_end(cx, win, None, ("env-name", 1usize), "tok");
    cx.run_until_parked();
    lock(cx, win, 1);
    assert!(marked(cx, win, 3).is_empty(), "one secret is no duplicate");
    type_at_end(cx, win, None, ("env-value", 1usize), "first");
    type_at_end(cx, win, None, ("env-name", 2usize), "tok");
    cx.run_until_parked();
    lock(cx, win, 2);
    type_at_end(cx, win, None, ("env-value", 2usize), "second");
    cx.run_until_parked();
    assert_eq!(marked(cx, win, 3), [1, 2], "both twins are marked");
    let note = cx.update(|cx| ed.read(cx).table().read(cx).duplicate_note(1, cx));
    assert_eq!(note, Some(DUPLICATE_SECRET_NAME));

    save(cx, win);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        DEV,
        "nothing written"
    );
    assert_eq!(store.get(&tok_key(&col)).unwrap(), None, "nothing stored");
    assert!(refused(cx, &ed));
    assert!(present(cx, win, "env-error"), "Save says why");

    type_at_end(cx, win, None, ("env-name", 2usize), "2");
    cx.run_until_parked();
    assert!(marked(cx, win, 3).is_empty(), "the rename clears the marks");
    save(cx, win);
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(
        saved.contains("name: tok\n") && saved.contains("name: tok2"),
        "{saved}"
    );
    assert_eq!(ed.read_with(cx, |e, _| e.error()), None);
    let tok2 = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "tok2");
    assert_eq!(store.get(&tok_key(&col)).unwrap().as_deref(), Some("first"));
    assert_eq!(store.get(&tok2).unwrap().as_deref(), Some("second"));
}

// A plain variable may share a secret's name, as the format allows; the secret wins when a
// request resolves, so neither row is marked and Save writes both.
#[gpui_kit::test]
fn a_plain_row_named_like_a_secret_is_not_marked(cx: &mut TestAppContext) {
    let dev = "name: dev\nvariables:\n  - secret: true\n    name: tok\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", dev)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-name", 1usize), "tok");
    cx.run_until_parked();
    assert!(marked(cx, win, 2).is_empty());
    save(cx, win);
    assert_eq!(ed.read_with(cx, |e, _| e.error()), None);
    let saved = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    assert_eq!(saved.matches("name: tok").count(), 2, "{saved}");
}

// A collision already on disk is marked, but an edit elsewhere still saves: refusing it would
// leave the environment unsavable.
#[gpui_kit::test]
fn a_collision_already_saved_is_marked_but_other_edits_save(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", TWINS),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(marked(cx, win, 3), [1, 2]);
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    save(cx, win);
    assert_eq!(ed.read_with(cx, |e, _| e.error()), None);
    let saved = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    assert!(saved.contains("value: xy"), "{saved}");
}

// With a collision on disk, a save that changes one of the twins is refused: unticking it
// changes the file's secrets, and typing its value would store over the shared one.
#[gpui_kit::test]
fn changing_a_saved_twin_is_refused(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", TWINS),
    ]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("env-enabled", 2usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    save(cx, win);
    assert!(refused(cx, &ed), "unticking a twin is refused");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), TWINS);

    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("env-enabled", 2usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 2usize), "typed");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        store.get(&tok_key(&col)).unwrap(),
        None,
        "Enter stores no twin"
    );
    save(cx, win);
    assert!(refused(cx, &ed), "a twin's typed value is refused");
    assert_eq!(store.get(&tok_key(&col)).unwrap(), None, "nothing stored");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), TWINS);
}

// A Save or Overwrite from outside the editor (autosave, Save All) meets the same refusal.
#[gpui_kit::test]
fn save_and_overwrite_commands_refuse_a_new_collision(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    type_twins(cx, win);
    let file = col.path().join("environments/dev.yml");
    let target = || Target::Environment("dev".into());
    for command in [Command::Save(target()), Command::Overwrite(target())] {
        ws.update(cx, |w, cx| w.send(&root, command, cx));
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), DEV);
        assert!(env_view(cx, &ws, &root, "dev").dirty, "the draft stays");
    }
}

// The changed-on-disk banner's Overwrite writes the draft as Save does, so it is refused
// before any typing reaches the store.
#[gpui_kit::test]
fn banner_overwrite_refuses_a_collision_and_stores_nothing(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    type_twins(cx, win);
    let file = col.path().join("environments/dev.yml");
    let outside = format!("{DEV}  - name: extra\n    value: z\n");
    std::fs::write(&file, &outside).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![file.clone()]), cx)
    });
    cx.run_until_parked();
    assert!(present(cx, win, "banner-changed"));
    cx.update_window(win, |_, w, cx| w.click("banner-overwrite", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(refused(cx, &ed));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), outside);
    assert_eq!(store.get(&tok_key(&col)).unwrap(), None, "nothing stored");
}

// Save under the deleted banner writes the draft back; with a collision it writes nothing.
#[gpui_kit::test]
fn save_under_the_deleted_banner_refuses_a_collision(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    type_twins(cx, win);
    let file = col.path().join("environments/dev.yml");
    let key = tok_key(&col);
    std::fs::remove_file(&file).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![file.clone()]), cx)
    });
    cx.run_until_parked();
    assert!(present(cx, win, "banner-deleted"));
    cx.update_window(win, |_, w, cx| w.click("banner-save", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(refused(cx, &ed));
    assert!(!file.exists(), "nothing written");
    assert_eq!(store.get(&key).unwrap(), None, "nothing stored");
}

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

// The global editor shares the table, so its rows are marked and its Save refuses alike.
#[gpui_kit::test]
fn a_global_environment_refuses_two_secrets_of_one_name(cx: &mut TestAppContext) {
    support::init(cx);
    let file = "name: g\nvariables:\n  - name: host\n    value: a\n";
    let col = collection();
    let globals = write_tree(&[("g.yml", file)]);
    let (_state, ws, _root) = open_dir(cx, &col);
    let dir = globals.path().to_path_buf();
    ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let shared = ws.clone();
    let editor = cx
        .update_window(win, |_, w, cx| open_global_env_editor(shared, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.select("g", w, cx))
    })
    .unwrap();
    type_twins(cx, win);
    assert_eq!(marked(cx, win, 3), [1, 2]);
    cx.update_window(win, |_, w, cx| w.click("global-env-save", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, "global-env-error"), "Save says why");
    assert_eq!(
        std::fs::read_to_string(globals.path().join("g.yml")).unwrap(),
        file
    );
    let (store, globals) =
        ws.read_with(cx, |w, _| (w.store().clone(), w.globals().expect("loaded")));
    assert_eq!(store.get(&globals.secret_key("g", "tok")).unwrap(), None);
}

const PROD: &str = "name: prod\n";

fn typed_at(
    cx: &mut TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::env_editor::EnvEditor>,
    ix: usize,
) -> Option<String> {
    let table = ed.read_with(cx, |e, _| e.table().clone());
    let input = table.read_with(cx, |t, _| t.value_input(ix).cloned())?;
    Some(input.read_with(cx, |i, _| i.value().to_string()))
}

// Typing in a twin already on disk can never be stored, so a switch asks before dropping it:
// Cancel keeps the selection and the typing, Discard switches.
#[gpui_kit::test]
fn a_switch_asks_before_dropping_a_twins_typing(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", TWINS),
        ("environments/prod.yml", PROD),
    ]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    assert!(
        ed.read_with(cx, |e, cx| e.holds_typing(cx)),
        "it counts as unsaved"
    );
    let choose_prod = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| w.click(("env-item", 1usize), cx))
            .unwrap();
        cx.run_until_parked();
    };
    choose_prod(cx);
    assert!(confirm_open(cx), "the switch asks");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("dev")
    );
    assert_eq!(
        typed_at(cx, &ed, 1).as_deref(),
        Some("typed"),
        "Cancel keeps it"
    );
    choose_prod(cx);
    answer_confirm(cx, "confirm-discard");
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("prod")
    );
    assert_eq!(store.get(&tok_key(&col)).unwrap(), None, "nothing stored");
}

// Closing the tab asks the same: Cancel keeps the tab and the typing, Discard closes it.
#[gpui_kit::test]
fn a_tab_close_asks_before_dropping_a_twins_typing(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", TWINS),
    ]);
    let store = Arc::new(MemoryStore::new());
    let s = started_with_store(cx, &col, store.clone());
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    type_at_end(cx, s.win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    let close = |cx: &mut TestAppContext| {
        cx.dispatch_action(s.win, apinata_app::tab_area::CloseTab);
        cx.run_until_parked();
    };
    close(cx);
    assert!(confirm_open(cx), "the close asks");
    answer_confirm(cx, "confirm-cancel");
    assert!(present(cx, s.win, "env-editor"), "the tab stays");
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    let id =
        s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().clone());
    assert!(
        cx.update(|cx| area.read(cx).holds_typing(&id, cx)),
        "Cancel keeps it"
    );
    close(cx);
    answer_confirm(cx, "confirm-discard");
    assert!(!present(cx, s.win, "env-editor"), "the tab went");
    assert_eq!(store.get(&tok_key(&col)).unwrap(), None, "nothing stored");
}

// The lock takes a row's typing with it, but never a twin's, which would land on the shared value.
#[gpui_kit::test]
fn the_lock_never_stores_a_twins_typing(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", TWINS),
    ]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, _ed) = env_editor(cx, &ws, &root);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    lock(cx, win, 1);
    let saved = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    assert!(!saved.contains("typed"), "the typing never moved: {saved}");
    assert_eq!(store.get(&tok_key(&col)).unwrap(), None, "nothing stored");
}
