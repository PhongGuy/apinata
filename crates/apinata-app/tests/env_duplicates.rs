//! Two secrets sharing one name in an environment: the later row is marked and a Save that
//! would write the collision is refused, in a collection environment and a global one.

use crate::support;

use apinata_app::command::{Command, Target};
use apinata_app::env_table::{DUPLICATE_SECRET_NAME, DUPLICATE_SECRET_NAMES, EnvRow};
use apinata_app::global_env_editor::open_global_env_editor;
use apinata_core::{Collection, MemoryStore, SecretStore};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, WindowOptions};
use std::sync::Arc;
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";

/// The rows sitting directly above a duplicate note, by name input text.
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

// Two new rows locked under one name would share one stored value, so the second is marked,
// Save writes neither the file nor the store, and renaming it away lets Save through.
#[gpui_kit::test]
fn a_second_secret_of_one_name_is_marked_and_save_refuses_it(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let store = Arc::new(MemoryStore::new());
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    let file = col.path().join("environments/dev.yml");
    type_at_end(cx, win, None, ("env-name", 1usize), "tok");
    cx.run_until_parked();
    lock(cx, win, 1);
    type_at_end(cx, win, None, ("env-value", 1usize), "first");
    cx.run_until_parked();
    assert!(marked(cx, win, 3).is_empty(), "one secret is no duplicate");
    type_at_end(cx, win, None, ("env-name", 2usize), "tok");
    cx.run_until_parked();
    lock(cx, win, 2);
    type_at_end(cx, win, None, ("env-value", 2usize), "second");
    cx.run_until_parked();
    assert_eq!(marked(cx, win, 3), [2], "only the later row is marked");
    let note = cx.update(|cx| ed.read(cx).table().read(cx).duplicate_note(2, cx));
    assert_eq!(note, Some(DUPLICATE_SECRET_NAME));

    save(cx, win);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        DEV,
        "nothing written"
    );
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "tok");
    assert_eq!(store.get(&key).unwrap(), None, "nothing stored");
    assert_eq!(
        ed.read_with(cx, |e, _| e.error()).as_deref(),
        Some(DUPLICATE_SECRET_NAMES)
    );
    assert!(present(cx, win, "env-error"), "Save says why");

    type_at_end(cx, win, None, ("env-name", 2usize), "2");
    cx.run_until_parked();
    assert!(marked(cx, win, 3).is_empty(), "the rename clears the mark");
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
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("first"));
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
    let dev = "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: tok\n  - secret: true\n    name: tok\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", dev)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = env_editor(cx, &ws, &root);
    assert_eq!(marked(cx, win, 3), [2]);
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    save(cx, win);
    assert_eq!(ed.read_with(cx, |e, _| e.error()), None);
    let saved = std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap();
    assert!(saved.contains("value: xy"), "{saved}");
}

// A Save from outside the editor (autosave, Save All) goes through the same refusal.
#[gpui_kit::test]
fn a_save_command_refuses_a_new_collision(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("environments/dev.yml", DEV)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = env_editor(cx, &ws, &root);
    for ix in [1usize, 2] {
        type_at_end(cx, win, None, ("env-name", ix), "tok");
        cx.run_until_parked();
        lock(cx, win, ix);
    }
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Environment("dev".into())), cx)
    });
    cx.run_until_parked();
    let file = col.path().join("environments/dev.yml");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), DEV);
    assert!(env_view(cx, &ws, &root, "dev").dirty, "the draft stays");
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
    let mut rows = editor.read_with(cx, |e, cx| e.table().read(cx).rows(cx));
    let secret = |value: &str| EnvRow {
        name: "tok".into(),
        value: value.into(),
        enabled: true,
        secret: true,
        unsaved: true,
        ..EnvRow::default()
    };
    rows.extend([secret(""), secret("")]);
    let table = editor.read_with(cx, |e, _| e.table().clone());
    cx.update_window(win, |_, w, cx| {
        table.update(cx, |t, cx| t.load(rows, w, cx))
    })
    .unwrap();
    assert_eq!(marked(cx, win, 3), [2]);
    let outcome = editor.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    assert_eq!(
        outcome.try_recv().ok(),
        Some(Err(DUPLICATE_SECRET_NAMES.to_owned()))
    );
    assert!(present(cx, win, "global-env-error"), "Save says why");
    assert_eq!(
        std::fs::read_to_string(globals.path().join("g.yml")).unwrap(),
        file
    );
}
