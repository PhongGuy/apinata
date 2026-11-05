//! The orphan section of the environment editor.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::env_editor::EnvEditor;
use apinata_app::env_table::EnvRow;
use apinata_app::global_env_editor::{GlobalEnvEditor, open_global_env_editor};
use apinata_app::orphan_section::{Orphans, visible};
use apinata_core::{
    Collection, EnvVariable, GlobalEnvironments, MemoryStore, SecretError, SecretKey, SecretStore,
    VarValue,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowOptions};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use support::*;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n";
const NOT_ORPHANED: &str = "not orphaned";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", "name: prod\n"),
    ])
}

/// A memory store that counts `list` calls, or fails them.
#[derive(Default)]
pub(crate) struct Listing {
    inner: MemoryStore,
    pub(crate) lists: AtomicUsize,
    fail: bool,
    pub(crate) fail_get: AtomicBool,
    pub(crate) gets: AtomicUsize,
}

impl SecretStore for Listing {
    fn get(&self, k: &SecretKey) -> Result<Option<String>, SecretError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        if self.fail_get.load(Ordering::SeqCst) {
            return Err(SecretError::Unavailable {
                message: "locked".into(),
            });
        }
        self.inner.get(k)
    }
    fn set(&self, k: &SecretKey, v: &str) -> Result<(), SecretError> {
        self.inner.set(k, v)
    }
    fn delete(&self, k: &SecretKey) -> Result<(), SecretError> {
        self.inner.delete(k)
    }
    fn list(&self, p: &Path) -> Result<Vec<SecretKey>, SecretError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(SecretError::Unavailable {
                message: "locked".into(),
            });
        }
        self.inner.list(p)
    }
    fn list_roots(&self) -> Result<Vec<std::path::PathBuf>, SecretError> {
        self.inner.list_roots()
    }
}

pub(crate) fn seeded(col: &tempfile::TempDir) -> (Arc<Listing>, SecretKey) {
    let store = Arc::new(Listing::default());
    let key = Collection::open(col.path())
        .unwrap()
        .secret_key("dev", "old");
    store.set(&key, "s3cret").unwrap();
    (store, key)
}

fn state(cx: &mut TestAppContext, ed: &Entity<EnvEditor>) -> Orphans {
    ed.read_with(cx, |e, _| e.orphans().clone())
}

fn names(list: &[&str]) -> Orphans {
    Orphans::Names(list.iter().map(|n| n.to_string()).collect())
}

#[gpui_kit::test]
fn an_orphan_shows_by_name_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let col = tree();
    let (store, key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert_eq!(state(cx, &ed), names(&["old"]));
    assert!(present(cx, win, "env-orphans"));
    assert!(present(cx, win, ("env-orphan-remove", 0usize)));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.ask_remove_orphan("old", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
    assert_eq!(state(cx, &ed), names(&["old"]));
}

#[gpui_kit::test]
fn confirming_removes_it_and_the_row_goes(cx: &mut TestAppContext) {
    let col = tree();
    let (store, key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.ask_remove_orphan("old", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert_eq!(store.get(&key).unwrap(), None);
    assert_eq!(state(cx, &ed), names(&[]));
    assert!(
        !present(cx, win, "env-orphans"),
        "nothing to say, no section"
    );
}

#[gpui_kit::test]
fn a_failed_list_is_a_line_and_never_an_empty_answer(cx: &mut TestAppContext) {
    let col = tree();
    let store = Arc::new(Listing {
        fail: true,
        ..Default::default()
    });
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert!(matches!(state(cx, &ed), Orphans::Failed(_)));
    assert!(present(cx, win, "env-orphans-failed"));
}

#[gpui_kit::test]
fn only_a_persons_action_lists(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    let after_open = store.lists.load(Ordering::SeqCst);
    assert_eq!(after_open, 1, "opening lists once");

    // An outside edit and the reload it brings never list.
    std::fs::write(col.path().join("environments/dev.yml"), "name: dev\n").unwrap();
    ws.update(cx, |w, cx| w.send(&root, Command::ReloadAll, cx));
    cx.run_until_parked();
    assert_eq!(store.lists.load(Ordering::SeqCst), after_open);

    // Nor does the editor moving its own selection when the selected file vanishes.
    std::fs::remove_file(col.path().join("environments/dev.yml")).unwrap();
    ws.update(cx, |w, cx| w.send(&root, Command::ReloadAll, cx));
    cx.run_until_parked();
    assert_eq!(selected_name(cx, &ed).as_deref(), Some("prod"));
    assert_eq!(store.lists.load(Ordering::SeqCst), after_open);

    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.choose("prod", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(store.lists.load(Ordering::SeqCst), after_open + 1);
}

fn selected_name(cx: &mut TestAppContext, ed: &Entity<EnvEditor>) -> Option<String> {
    ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
}

#[gpui_kit::test]
fn saving_lists_again(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    let before = store.lists.load(Ordering::SeqCst);
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(store.lists.load(Ordering::SeqCst), before + 1);
}

#[test]
fn an_unsaved_secret_hides_its_name() {
    let mut secret = EnvVariable::secret("a");
    secret.enabled = false;
    let plain = EnvVariable::new("b", VarValue::Text("x".into()));
    let got = visible(
        &["a".to_owned(), "b".to_owned(), "c".to_owned()],
        &[secret, plain],
    );
    assert_eq!(got, ["b", "c"]);
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

pub(crate) type GlobalHarness = (
    tempfile::TempDir,
    tempfile::TempDir,
    tempfile::TempDir,
    gpui_kit::AnyWindowHandle,
    Entity<GlobalEnvEditor>,
    SecretKey,
);

fn global_editor(cx: &mut TestAppContext, store: Arc<Listing>) -> GlobalHarness {
    global_editor_of(
        cx,
        store,
        "name: g\nvariables:\n  - name: host\n    value: a\n",
    )
}

/// The global editor on `g`, whose file is `g_yml`, with a stored value for `old`.
pub(crate) fn global_editor_of(
    cx: &mut TestAppContext,
    store: Arc<Listing>,
    g_yml: &str,
) -> GlobalHarness {
    let (col, globals, state, win, editor, key, _ws) = global_editor_with_ws(cx, store, g_yml);
    (col, globals, state, win, editor, key)
}

/// [`global_editor_of`], with the workspace too.
pub(crate) fn global_editor_with_ws(
    cx: &mut TestAppContext,
    store: Arc<Listing>,
    g_yml: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    tempfile::TempDir,
    gpui_kit::AnyWindowHandle,
    Entity<GlobalEnvEditor>,
    SecretKey,
    Entity<apinata_app::workspace::Workspace>,
) {
    support::init(cx);
    let col = collection();
    let globals = write_tree(&[("g.yml", g_yml), ("h.yml", "name: h\n")]);
    let key = GlobalEnvironments::open(globals.path())
        .unwrap()
        .secret_key("g", "old");
    store.set(&key, "s3cret").unwrap();
    let (state, ws, _root) = open_dir_with(cx, &col, store);
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
        editor.update(cx, |e, cx| e.choose("g", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    (col, globals, state, win, editor, key, ws)
}

#[gpui_kit::test]
fn a_global_orphan_shows_and_is_removed_after_confirming(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, _g, _s, win, ed, key) = global_editor(cx, store.clone());
    assert_eq!(
        ed.read_with(cx, |e, _| e.orphans().clone()),
        names(&["old"])
    );
    assert!(present(cx, win, ("env-orphan-remove", 0usize)));
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.ask_remove_orphan("old", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.ask_remove_orphan("old", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert_eq!(store.get(&key).unwrap(), None);
    assert_eq!(ed.read_with(cx, |e, _| e.orphans().clone()), names(&[]));
}

#[gpui_kit::test]
fn a_global_list_failure_is_a_line(cx: &mut TestAppContext) {
    let store = Arc::new(Listing {
        fail: true,
        ..Default::default()
    });
    let (_c, _g, _s, win, ed, _key) = global_editor(cx, store);
    assert!(matches!(
        ed.read_with(cx, |e, _| e.orphans().clone()),
        Orphans::Failed(_)
    ));
    assert!(present(cx, win, "env-orphans-failed"));
}

/// Marks `name` secret in an unsaved draft of `dev`.
fn draft_secret(
    cx: &mut TestAppContext,
    ws: &Entity<apinata_app::workspace::Workspace>,
    root: &Path,
    name: &str,
) {
    let host = EnvVariable::new("host", VarValue::Text("x".into()));
    let command = Command::Edit {
        target: Target::Environment("dev".into()),
        edit: Edit::Variables(vec![host, EnvVariable::secret(name)]),
    };
    ws.update(cx, |w, cx| w.send(root, command, cx));
    cx.run_until_parked();
}

fn ask_remove(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, ed: &Entity<EnvEditor>) {
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.ask_remove_orphan("old", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_draft_secret_hides_its_row_until_it_is_gone(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert!(present(cx, win, "env-orphans"));
    draft_secret(cx, &ws, &root, "old");
    assert_eq!(state(cx, &ed), names(&["old"]));
    assert!(!present(cx, win, "env-orphans"));
}

#[gpui_kit::test]
fn a_draft_declaring_the_name_during_the_prompt_keeps_the_value(cx: &mut TestAppContext) {
    let col = tree();
    let (store, key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    ask_remove(cx, win, &ed);
    draft_secret(cx, &ws, &root, "old");
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
}

#[gpui_kit::test]
fn moving_the_selection_during_the_prompt_keeps_the_value(cx: &mut TestAppContext) {
    let col = tree();
    let (store, key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    ask_remove(cx, win, &ed);
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("prod", w, cx))
    })
    .unwrap();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
}

#[gpui_kit::test]
fn a_late_list_answer_for_another_environment_is_dropped(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    // The list leaves for `dev` and the selection moves before the actor can answer.
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.list_orphans(cx);
            e.select("prod", w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(state(cx, &ed), Orphans::Unknown);
}

#[gpui_kit::test]
fn a_rename_clears_the_rows_of_the_old_name(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (_win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert_eq!(state(cx, &ed), names(&["old"]));
    ed.update(cx, |e, cx| e.rename("staging", cx));
    cx.run_until_parked();
    assert_eq!(selected_name(cx, &ed).as_deref(), Some("staging"));
    assert_eq!(state(cx, &ed), Orphans::Unknown);
}

#[gpui_kit::test]
fn a_list_with_nowhere_to_go_is_a_failure(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (_win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    ws.update(cx, |w, cx| w.close(&root, cx));
    cx.run_until_parked();
    ed.update(cx, |e, cx| e.list_orphans(cx));
    cx.run_until_parked();
    assert!(matches!(state(cx, &ed), Orphans::Failed(_)));
}

fn global_state(cx: &mut TestAppContext, ed: &Entity<GlobalEnvEditor>) -> Orphans {
    ed.read_with(cx, |e, _| e.orphans().clone())
}

/// Adds an unsaved row of `g` that marks `name` secret.
fn global_draft_secret(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    ed: &Entity<GlobalEnvEditor>,
    name: &str,
) {
    let row = EnvRow {
        name: name.to_owned(),
        value: String::new(),
        enabled: true,
        secret: true,
        source: None,
        unsaved: true,
    };
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.table().clone().update(cx, |t, cx| {
                let mut rows = t.rows(cx);
                rows.push(row);
                t.load(rows, w, cx);
            })
        })
    })
    .unwrap();
    cx.run_until_parked();
}

fn global_ask_remove(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    ed: &Entity<GlobalEnvEditor>,
) {
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.ask_remove_orphan("old", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_global_draft_secret_hides_its_row(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, _g, _s, win, ed, _key) = global_editor(cx, store);
    assert!(present(cx, win, "env-orphans"));
    global_draft_secret(cx, win, &ed, "old");
    assert_eq!(global_state(cx, &ed), names(&["old"]));
    assert!(!present(cx, win, "env-orphans"));
}

#[gpui_kit::test]
fn a_global_draft_declaring_the_name_during_the_prompt_keeps_the_value(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, _g, _s, win, ed, key) = global_editor(cx, store.clone());
    global_ask_remove(cx, win, &ed);
    global_draft_secret(cx, win, &ed, "old");
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
}

#[gpui_kit::test]
fn moving_the_global_selection_during_the_prompt_keeps_the_value(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, _g, _s, win, ed, key) = global_editor(cx, store.clone());
    global_ask_remove(cx, win, &ed);
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.select("h", w, cx)))
        .unwrap();
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
}

#[gpui_kit::test]
fn a_late_global_list_answer_for_another_environment_is_dropped(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, _g, _s, win, ed, _key) = global_editor(cx, store);
    // The list leaves for `g` and the selection moves before its answer can land.
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| {
            e.list_orphans(cx);
            e.select("h", w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(global_state(cx, &ed), Orphans::Unknown);
}

#[gpui_kit::test]
fn a_global_rename_clears_the_rows_of_the_old_name(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, _g, _s, win, ed, _key) = global_editor(cx, store);
    assert_eq!(global_state(cx, &ed), names(&["old"]));
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.rename("k", w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        ed.read_with(cx, |e, _| e.selected().map(str::to_owned))
            .as_deref(),
        Some("k")
    );
    assert_eq!(global_state(cx, &ed), Orphans::Unknown);
}

const PLAIN_OLD: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: old\n    value: ''\n";

/// `dev` declares `old` plain and empty while the store still holds a value for it.
fn plain_tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", PLAIN_OLD),
        ("environments/prod.yml", "name: prod\n"),
    ])
}

pub(crate) fn click_lock(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, ix: usize) {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-secret", ix), cx))
        .unwrap();
    cx.run_until_parked();
}

fn dev_file(col: &tempfile::TempDir) -> String {
    std::fs::read_to_string(col.path().join("environments/dev.yml")).unwrap()
}

#[gpui_kit::test]
fn the_lock_on_then_off_moves_a_stored_value_into_the_file(cx: &mut TestAppContext) {
    let col = plain_tree();
    let (store, key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    assert_eq!(state(cx, &ed), names(&["old"]));
    click_lock(cx, win, 1);
    let held_on = store.get(&key).unwrap();
    let view_on = env_view(cx, &ws, &root, "dev");
    assert_eq!(state(cx, &ed), names(&[]), "the list follows the toggle");
    click_lock(cx, win, 1);
    answer_confirm(cx, "confirm-make-plain");
    let held_off = store.get(&key).unwrap();
    let file = dev_file(&col);
    let listed = state(cx, &ed);
    assert!(view_on.variables[1].secret);
    assert_eq!(
        held_on.as_deref(),
        Some("s3cret"),
        "the lock keeps the value"
    );
    assert_eq!(held_off, None, "the value left the store for the file");
    assert!(file.contains("s3cret"), "{file}");
    assert_eq!(listed, names(&[]), "no stale row is left");
}

#[gpui_kit::test]
fn a_toggle_lists_again(cx: &mut TestAppContext) {
    let col = plain_tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    let before = store.lists.load(Ordering::SeqCst);
    click_lock(cx, win, 1);
    assert_eq!(store.lists.load(Ordering::SeqCst), before + 1);
}

const PLAIN_G: &str =
    "name: g\nvariables:\n  - name: host\n    value: a\n  - name: old\n    value: ''\n";

#[gpui_kit::test]
fn the_global_lock_on_then_off_leaves_no_stale_row(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, globals, _s, win, ed, key) = global_editor_of(cx, store.clone(), PLAIN_G);
    assert_eq!(global_state(cx, &ed), names(&["old"]));
    click_lock(cx, win, 1);
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret"));
    assert_eq!(global_state(cx, &ed), names(&[]));
    click_lock(cx, win, 1);
    answer_confirm(cx, "confirm-make-plain");
    assert_eq!(store.get(&key).unwrap(), None);
    assert_eq!(global_state(cx, &ed), names(&[]));
    let file = std::fs::read_to_string(globals.path().join("g.yml")).unwrap();
    assert!(file.contains("s3cret"), "{file}");
    assert!(!present(cx, win, "env-orphans"));
}

const SECRET_OLD: &str =
    "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: old\n    secret: true\n";

fn line(cx: &mut TestAppContext, ed: &Entity<EnvEditor>) -> Option<String> {
    ed.read_with(cx, |e, _| e.error())
}

#[gpui_kit::test]
fn a_stale_row_the_guard_refuses_deletes_nothing_and_choosing_clears_its_line(
    cx: &mut TestAppContext,
) {
    let col = tree();
    let (store, key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    // The file declares the name now, and nothing has told the editor.
    std::fs::write(col.path().join("environments/dev.yml"), SECRET_OLD).unwrap();
    assert!(present(cx, win, "env-orphans"));
    ask_remove(cx, win, &ed);
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
    let shown = line(cx, &ed).expect("the refusal shows");
    assert!(shown.contains(NOT_ORPHANED), "{shown}");

    // A selection the editor makes by itself leaves the line; a person's reselection clears it.
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.select("dev", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(line(cx, &ed).is_some());
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.choose("dev", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(line(cx, &ed), None);
}

#[gpui_kit::test]
fn choosing_another_environment_clears_the_line(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    std::fs::write(col.path().join("environments/dev.yml"), SECRET_OLD).unwrap();
    ask_remove(cx, win, &ed);
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(line(cx, &ed).is_some());
    cx.update_window(win, |_, w, cx| {
        ed.update(cx, |e, cx| e.choose("prod", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(line(cx, &ed), None);
}

#[gpui_kit::test]
fn a_create_landing_with_a_failed_one_keeps_the_failures_line(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (_win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    ed.update(cx, |e, cx| {
        e.create("---", cx);
        e.create("qa", cx);
    });
    cx.run_until_parked();
    assert_eq!(selected_name(cx, &ed).as_deref(), Some("qa"));
    assert!(line(cx, &ed).is_some(), "the failed create still shows");
}

#[gpui_kit::test]
fn a_global_stale_row_the_guard_refuses_clears_on_choosing(cx: &mut TestAppContext) {
    let store = Arc::new(Listing::default());
    let (_c, globals, _s, win, ed, key) = global_editor(cx, store.clone());
    let secret = "name: g\nvariables:\n  - name: old\n    secret: true\n";
    std::fs::write(globals.path().join("g.yml"), secret).unwrap();
    assert!(present(cx, win, "env-orphans"));
    global_ask_remove(cx, win, &ed);
    answer_confirm(cx, "confirm-delete");
    cx.run_until_parked();
    assert!(store.get(&key).unwrap().is_some());
    assert!(present(cx, win, "global-env-error"));

    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.select("g", w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, win, "global-env-error"));
    cx.update_window(win, |_, w, cx| ed.update(cx, |e, cx| e.choose("g", w, cx)))
        .unwrap();
    cx.run_until_parked();
    assert!(!present(cx, win, "global-env-error"));
}

fn draft_flag(cx: &mut TestAppContext, ed: &Entity<EnvEditor>, ix: usize) -> bool {
    cx.update(|cx| ed.read(cx).table().read(cx).rows(cx)[ix].secret)
}

fn file_flag(
    cx: &mut TestAppContext,
    ws: &Entity<apinata_app::workspace::Workspace>,
    root: &Path,
) -> bool {
    env_view(cx, ws, root, "dev").variables[1].secret
}

#[gpui_kit::test]
fn the_lock_answers_on_a_row_named_like_an_orphan(cx: &mut TestAppContext) {
    let col = plain_tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    for want in [true, false, true] {
        click_lock(cx, win, 1);
        if !want {
            answer_confirm(cx, "confirm-make-plain");
        }
        assert_eq!(file_flag(cx, &ws, &root), want);
    }
}

#[gpui_kit::test]
fn the_lock_answers_after_an_outside_edit_and_a_reload(cx: &mut TestAppContext) {
    let col = plain_tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    let edited = PLAIN_OLD.replace("value: x", "value: z");
    std::fs::write(col.path().join("environments/dev.yml"), edited).unwrap();
    ws.update(cx, |w, cx| w.send(&root, Command::ReloadAll, cx));
    cx.run_until_parked();
    click_lock(cx, win, 1);
    assert!(file_flag(cx, &ws, &root));
}

#[gpui_kit::test]
fn the_lock_waits_for_a_clean_environment_on_such_a_row(cx: &mut TestAppContext) {
    let col = plain_tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-value", 0usize), "y");
    cx.run_until_parked();
    click_lock(cx, win, 1);
    assert!(!file_flag(cx, &ws, &root), "a dirty environment holds it");
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    click_lock(cx, win, 1);
    assert!(file_flag(cx, &ws, &root), "Save frees it");
}

#[gpui_kit::test]
fn a_new_row_named_like_an_orphan_flips_locally_and_hides_it(cx: &mut TestAppContext) {
    let col = tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-name", 1usize), "old");
    cx.run_until_parked();
    assert!(present(cx, win, "env-orphans"));
    click_lock(cx, win, 1);
    assert!(draft_flag(cx, &ed, 1));
    assert!(!present(cx, win, "env-orphans"));
}

fn draft_name(cx: &mut TestAppContext, ed: &Entity<EnvEditor>, ix: usize) -> String {
    cx.update(|cx| ed.read(cx).table().read(cx).rows(cx)[ix].name.clone())
}

/// Clicks row `ix`'s name, presses Backspace at its end and types `text`.
fn edit_name(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, ix: usize, text: &str) {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click(("env-name", ix), cx);
        w.press("end", cx);
        w.press("backspace", cx);
        w.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_row_added_after_a_saved_removal_locks_and_its_name_stays_editable(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", SECRET_OLD),
    ]);
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-delete", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-name", 1usize), "old");
    cx.run_until_parked();
    click_lock(cx, win, 1);
    assert!(draft_flag(cx, &ed, 1), "the lock answers");
    edit_name(cx, win, 1, "x");
    assert_eq!(
        draft_name(cx, &ed, 1),
        "olx",
        "the name takes Backspace and typing"
    );
}

#[gpui_kit::test]
fn a_locked_new_row_named_like_an_orphan_is_renamed_and_save_asks_replace(cx: &mut TestAppContext) {
    let col = tree();
    let (store, skey) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-name", 1usize), "olx");
    cx.run_until_parked();
    click_lock(cx, win, 1);
    assert!(draft_flag(cx, &ed, 1));
    edit_name(cx, win, 1, "d");
    assert_eq!(draft_name(cx, &ed, 1), "old");
    frame(cx, win);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx), "the save asks Replace");
    answer_confirm(cx, "confirm-cancel");
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("s3cret"));
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    answer_confirm(cx, "confirm-replace");
    cx.run_until_parked();
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("typed"));
}

/// Types `typed` into row `ix`'s value, then clicks the first row's value to blur it.
fn type_and_blur(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, ix: usize) {
    frame(cx, win);
    type_at_end(cx, win, None, ("env-value", ix), "typed");
    cx.run_until_parked();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-value", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
}

/// `dev` declares the secret `old`, stored as `s3cret`, after `host`.
fn saved_secret(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<Listing>,
    SecretKey,
    gpui_kit::AnyWindowHandle,
    Entity<EnvEditor>,
) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", SECRET_OLD),
    ]);
    let (store, skey) = seeded(&col);
    let (state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    (col, state, store, skey, win, ed)
}

fn olds(cx: &mut TestAppContext, ed: &Entity<EnvEditor>) -> usize {
    let rows = cx.update(|cx| ed.read(cx).table().read(cx).rows(cx));
    rows.iter().filter(|r| r.name == "old").count()
}

/// Locks a new row `ol` at `ix` and types `d` into its name: the name stays `ol` and a note
/// on the row names the saved secret.
fn rename_onto_old_is_refused(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    ed: &Entity<EnvEditor>,
    ix: usize,
) {
    type_at_end(cx, win, None, ("env-name", ix), "ol");
    cx.run_until_parked();
    click_lock(cx, win, ix);
    assert!(draft_flag(cx, ed, ix));
    frame(cx, win);
    type_at_end(cx, win, None, ("env-name", ix), "d");
    cx.run_until_parked();
    assert_eq!(draft_name(cx, ed, ix), "ol", "the name is refused");
    assert!(present(cx, win, "env-name-refused"), "the row says why");
    let note = cx.update(|cx| ed.read(cx).table().read(cx).name_note().map(str::to_owned));
    assert!(note.is_some_and(|n| n.contains("\"old\"")));
}

/// Blurs typing in row `ix`, then saves, cancelling any Replace prompt: the typing never
/// lands on the stored value unasked.
fn type_blur_and_save_store_nothing(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    store: &Listing,
    skey: &SecretKey,
    ix: usize,
) {
    type_and_blur(cx, win, ix);
    assert_eq!(
        store.get(skey).unwrap().as_deref(),
        Some("s3cret"),
        "a blur stores nothing"
    );
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    if confirm_open(cx) {
        answer_confirm(cx, "confirm-cancel");
    }
    // A saved removal may delete the value; it never stores the typing over it.
    assert_ne!(
        store.get(skey).unwrap().as_deref(),
        Some("typed"),
        "a save stores nothing"
    );
}

#[gpui_kit::test]
fn a_locked_new_row_cannot_take_a_saved_secrets_name(cx: &mut TestAppContext) {
    let (_col, _state, store, skey, win, ed) = saved_secret(cx);
    rename_onto_old_is_refused(cx, win, &ed, 2);
    type_blur_and_save_store_nothing(cx, win, &store, &skey, 2);
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("s3cret"));
}

#[gpui_kit::test]
fn a_locked_new_row_cannot_take_the_name_of_a_saved_secret_deleted_in_the_draft(
    cx: &mut TestAppContext,
) {
    let (_col, _state, store, skey, win, ed) = saved_secret(cx);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-delete", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    rename_onto_old_is_refused(cx, win, &ed, 1);
    type_blur_and_save_store_nothing(cx, win, &store, &skey, 1);
}

#[gpui_kit::test]
fn undo_and_redo_after_a_refused_name_store_nothing(cx: &mut TestAppContext) {
    use apinata_app::request_editor::{RedoEdit, UndoEdit};
    let (_col, _state, store, skey, win, ed) = saved_secret(cx);
    rename_onto_old_is_refused(cx, win, &ed, 2);
    cx.dispatch_action(win, UndoEdit);
    cx.run_until_parked();
    cx.dispatch_action(win, RedoEdit);
    cx.run_until_parked();
    assert_eq!(olds(cx, &ed), 1, "only the saved row is named `old`");
    type_blur_and_save_store_nothing(cx, win, &store, &skey, 2);
}

#[gpui_kit::test]
fn deleting_and_undoing_after_a_refused_name_store_nothing(cx: &mut TestAppContext) {
    use apinata_app::request_editor::UndoEdit;
    let (_col, _state, store, skey, win, ed) = saved_secret(cx);
    rename_onto_old_is_refused(cx, win, &ed, 2);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-delete", 2usize), cx))
        .unwrap();
    cx.run_until_parked();
    cx.dispatch_action(win, UndoEdit);
    cx.run_until_parked();
    assert_eq!(olds(cx, &ed), 1, "only the saved row is named `old`");
    type_blur_and_save_store_nothing(cx, win, &store, &skey, 2);
}

/// Types `ol` into the new row at `ix` and lets it settle, then types `rest` and clicks the
/// row's lock in one update, before that last edit settles.
fn finish_name_and_lock(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    ix: usize,
    rest: &str,
) {
    type_at_end(cx, win, None, ("env-name", ix), "ol");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.press("end", cx);
        w.input(rest, cx);
        w.click(("env-secret", ix), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_lock_on_a_new_row_just_named_like_a_saved_secret_is_refused(cx: &mut TestAppContext) {
    let (_col, _state, store, skey, win, ed) = saved_secret(cx);
    finish_name_and_lock(cx, win, 2, "d");
    assert!(!draft_flag(cx, &ed, 2), "the lock is refused");
    assert!(present(cx, win, "env-name-refused"), "the row says why");
    let note = cx.update(|cx| ed.read(cx).table().read(cx).name_note().map(str::to_owned));
    assert!(note.is_some_and(|n| n.contains("\"old\" is a saved secret")));
    type_blur_and_save_store_nothing(cx, win, &store, &skey, 2);
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("s3cret"));
}

/// Locks a new row `start` at row 2, then presses `clear` on its name and types `old` in one
/// update, as an automation host does: the name stays and the note names `old`.
fn one_input_onto_old_is_refused(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    ed: &Entity<EnvEditor>,
    start: &str,
    clear: &[&str],
) {
    let ix = 2;
    type_at_end(cx, win, None, ("env-name", ix), start);
    cx.run_until_parked();
    click_lock(cx, win, ix);
    assert!(draft_flag(cx, ed, ix));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click(("env-name", ix), cx);
        for keys in clear {
            w.press(keys, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    let kept = draft_name(cx, ed, ix);
    frame(cx, win);
    // Every key lands before any Change is handled, so each handler reads the whole name.
    cx.update_window(win, |_, w, cx| {
        for c in "old".chars() {
            let mut key = gpui_kit::Keystroke::parse(&c.to_string()).unwrap();
            key.key_char = Some(c.to_string());
            w.dispatch_keystroke(key, cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(draft_name(cx, ed, ix), kept, "the name is refused");
    assert!(present(cx, win, "env-name-refused"), "the row says why");
    let note = note_text(cx, ed);
    assert!(note.is_some_and(|n| n.contains("\"old\" is a saved secret")));
}

#[gpui_kit::test]
fn a_saved_secrets_name_replacing_a_selected_name_in_one_input_shows_the_note(
    cx: &mut TestAppContext,
) {
    let (_col, _state, store, skey, win, ed) = saved_secret(cx);
    one_input_onto_old_is_refused(cx, win, &ed, "olx", &["ctrl-a"]);
    type_blur_and_save_store_nothing(cx, win, &store, &skey, 2);
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("s3cret"));
}

#[gpui_kit::test]
fn a_saved_secrets_name_entered_in_one_input_into_an_emptied_name_shows_the_note(
    cx: &mut TestAppContext,
) {
    let (_col, _state, store, skey, win, ed) = saved_secret(cx);
    one_input_onto_old_is_refused(cx, win, &ed, "olx", &["ctrl-a", "backspace"]);
    type_blur_and_save_store_nothing(cx, win, &store, &skey, 2);
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("s3cret"));
}

#[gpui_kit::test]
fn an_allowed_name_after_a_refusal_clears_the_note(cx: &mut TestAppContext) {
    let (_col, _state, _store, _skey, win, ed) = saved_secret(cx);
    one_input_onto_old_is_refused(cx, win, &ed, "olx", &["ctrl-a"]);
    type_at_end(cx, win, None, ("env-name", 2usize), "y");
    cx.run_until_parked();
    assert_eq!(draft_name(cx, &ed, 2), "olxy");
    assert!(!present(cx, win, "env-name-refused"), "the note is gone");
}

const TWO_SECRETS: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: old\n    secret: true\n  - name: tok\n    secret: true\n";

fn note_text(cx: &mut TestAppContext, ed: &Entity<EnvEditor>) -> Option<String> {
    cx.update(|cx| ed.read(cx).table().read(cx).name_note().map(str::to_owned))
}

#[gpui_kit::test]
fn a_refused_row_renamed_onto_a_second_saved_secret_names_that_one(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", TWO_SECRETS),
    ]);
    let (store, _skey) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    finish_name_and_lock(cx, win, 3, "d");
    assert!(note_text(cx, &ed).is_some_and(|n| n.contains("\"old\"")));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click(("env-name", 3usize), cx);
        w.press("ctrl-a", cx);
        w.input("tok", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let note = note_text(cx, &ed);
    assert!(
        note.as_deref().is_some_and(|n| n.contains("\"tok\"")),
        "the note names the name the row was refused: {note:?}"
    );
}

const SECRET_THEN_PLAIN: &str = "name: dev\nvariables:\n  - name: old\n    secret: true\n  - name: a\n    value: 1\n  - name: b\n    value: 2\n";

/// The rows whose name input is the last one above the `env-name-refused` note.
fn rows_with_note(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle, n: usize) -> Vec<usize> {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let Some(note) = w.try_find("env-name-refused") else {
            return Vec::new();
        };
        let top = note.bounds().origin.y;
        let tops: Vec<_> = (0..n)
            .map(|ix| w.try_find(("env-name", ix)).map(|r| r.bounds().origin.y))
            .collect();
        (0..n)
            .filter(|&ix| {
                tops[ix].is_some_and(|y| y <= top)
                    && tops
                        .get(ix + 1)
                        .copied()
                        .flatten()
                        .is_none_or(|next| top < next)
            })
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn an_undo_shifting_the_refused_row_leaves_no_note_on_the_row_that_took_its_place(
    cx: &mut TestAppContext,
) {
    use apinata_app::request_editor::UndoEdit;
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", SECRET_THEN_PLAIN),
    ]);
    let (store, _skey) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    finish_name_and_lock(cx, win, 3, "d");
    assert_eq!(
        rows_with_note(cx, win, 4),
        vec![3],
        "the note sits under its row"
    );
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-delete", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        rows_with_note(cx, win, 3),
        vec![2],
        "the note moved with its row"
    );
    cx.dispatch_action(win, UndoEdit);
    cx.run_until_parked();
    let rows = cx.update(|cx| ed.read(cx).table().read(cx).rows(cx));
    for ix in rows_with_note(cx, win, rows.len()) {
        assert!(
            rows[ix].name == "old" && !rows[ix].secret,
            "the note sits under row {ix} named {:?}",
            rows[ix].name
        );
    }
}

#[gpui_kit::test]
fn a_lock_on_a_settled_plain_row_named_like_a_saved_secret_says_why(cx: &mut TestAppContext) {
    let (_col, _state, store, skey, win, ed) = saved_secret(cx);
    type_at_end(cx, win, None, ("env-name", 2usize), "old");
    cx.run_until_parked();
    click_lock(cx, win, 2);
    assert!(!draft_flag(cx, &ed, 2), "the row stays plain");
    assert!(present(cx, win, "env-name-refused"), "the row says why");
    let note = note_text(cx, &ed);
    assert!(note.is_some_and(|n| n.contains("\"old\" is a saved secret")));
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("s3cret"));
}

#[gpui_kit::test]
fn a_lock_on_a_new_row_just_named_otherwise_still_locks(cx: &mut TestAppContext) {
    let (_col, _state, _store, _skey, win, ed) = saved_secret(cx);
    finish_name_and_lock(cx, win, 2, "x");
    assert!(draft_flag(cx, &ed, 2));
    assert!(!present(cx, win, "env-name-refused"));
}

#[gpui_kit::test]
fn a_lock_on_a_new_row_just_named_like_an_orphan_locks_and_save_asks_replace(
    cx: &mut TestAppContext,
) {
    let col = tree();
    let (store, skey) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    finish_name_and_lock(cx, win, 1, "d");
    assert!(
        draft_flag(cx, &ed, 1),
        "an orphan's name is not a saved secret"
    );
    frame(cx, win);
    type_at_end(cx, win, None, ("env-value", 1usize), "typed");
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx), "the save asks Replace");
    answer_confirm(cx, "confirm-replace");
    cx.run_until_parked();
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("typed"));
}

#[gpui_kit::test]
fn renaming_a_locked_row_named_like_an_orphan_before_save_brings_the_orphan_back(
    cx: &mut TestAppContext,
) {
    let col = tree();
    let (store, skey) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-name", 1usize), "old");
    cx.run_until_parked();
    click_lock(cx, win, 1);
    assert!(!present(cx, win, "env-orphans"));
    edit_name(cx, win, 1, "x");
    assert_eq!(draft_name(cx, &ed, 1), "olx");
    assert!(
        present(cx, win, "env-orphans"),
        "the stored value shows as an orphan again"
    );
    assert_eq!(store.get(&skey).unwrap().as_deref(), Some("s3cret"));
}

/// Deletes the `old` row and types its name into the empty row; both go to the draft.
fn retype_old(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("env-delete", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    type_at_end(cx, win, None, ("env-name", 1usize), "old");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_row_retyped_as_the_file_has_it_is_clean_and_the_lock_answers(cx: &mut TestAppContext) {
    let col = plain_tree();
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, _ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    retype_old(cx, win);
    assert!(!env_view(cx, &ws, &root, "dev").dirty);
    click_lock(cx, win, 1);
    assert!(file_flag(cx, &ws, &root));
}

#[gpui_kit::test]
fn a_row_retyped_without_its_value_waits_for_save(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: old\n    value: y\n",
        ),
    ]);
    let (store, _key) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store);
    let (win, ed) = env_editor(cx, &ws, &root);
    cx.run_until_parked();
    retype_old(cx, win);
    assert!(env_view(cx, &ws, &root, "dev").dirty);
    click_lock(cx, win, 1);
    assert!(
        !draft_flag(cx, &ed, 1),
        "a saved name waits, as saved rows do"
    );
    assert!(present(cx, win, "env-orphans"));
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    cx.run_until_parked();
    click_lock(cx, win, 1);
    assert!(file_flag(cx, &ws, &root), "Save frees it");
}

#[gpui_kit::test]
fn remove_stays_inside_a_narrow_window(cx: &mut TestAppContext) {
    let col = tree();
    let (store, key_old) = seeded(&col);
    let (_state, ws, root) = open_dir_with(cx, &col, store.clone());
    let (win, _ed) = env_editor(cx, &ws, &root);
    gpui_kit::VisualTestContext::from_window(win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(400.), gpui_kit::px(400.)));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let button = w.find(("env-orphan-remove", 0usize)).bounds();
        assert!(
            button.left() >= gpui_kit::px(0.) && button.right() <= w.viewport_size().width,
            "{button:?} inside {:?}",
            w.viewport_size()
        );
        w.click(("env-orphan-remove", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx), "the button takes a click");
    answer_confirm(cx, "confirm-cancel");
    assert!(store.get(&key_old).unwrap().is_some());
}
