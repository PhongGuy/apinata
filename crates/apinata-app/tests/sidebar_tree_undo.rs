//! Ctrl+Z and Ctrl+Shift+Z in the sidebar tree undo and redo its moves and renames, newest
//! first, and never reach the request editor behind it.

use crate::support;

use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext, point};
use support::*;

// A request with a body, an assertion and settings, so every editor section loads something.
const CREATE: &str = "info:\n  name: Create todo\n  type: http\n  seq: 3\n\nhttp:\n  method: POST\n  url: \"http://x/todos?a=1\"\n  headers:\n    - name: H\n      value: v\n  body:\n    type: json\n    data: |-\n      {\n        \"title\": \"t\"\n      }\n  auth: inherit\n\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"201\"\n\nsettings:\n  encodeUrl: true\n  timeout: inherit\n  followRedirects: true\n  maxRedirects: 5\n";

fn files() -> Vec<(&'static str, String)> {
    vec![
        ("opencollection.yml", ROOT.to_string()),
        ("todos/folder.yml", folder("Todos")),
        ("todos/Create todo.yml", CREATE.to_string()),
        (
            "todos/List todos.yml",
            request("List todos", "http://x/todos/1"),
        ),
        ("users/folder.yml", folder("Users")),
        (
            "users/Get user.yml",
            request("Get user", "http://x/users/1"),
        ),
    ]
}

fn start(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let refs: Vec<(&str, String)> = files();
    let refs: Vec<(&str, &str)> = refs.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let col = write_tree(&refs);
    let s = started(cx, &col);
    frame(cx, s.win);
    (col, s)
}

fn sidebar(cx: &mut TestAppContext, s: &Started) -> Entity<apinata_app::sidebar::Sidebar> {
    s.shell.read_with(cx, |sh, _| sh.sidebar().clone())
}

fn expand(cx: &mut TestAppContext, s: &Started, folder: &str) {
    cx.update_window(s.win, |_, w, cx| w.click(row(folder), cx))
        .unwrap();
    frame(cx, s.win);
}

// Renames through the tree's own inline rename, as F2 and Enter do.
fn rename(cx: &mut TestAppContext, s: &Started, path: &str, name: &str) {
    let view = sidebar(cx, s);
    let root = s.root.clone();
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| v.start_rename(&root, &ip(path), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.input(name, cx);
        w.press("enter", cx);
    })
    .unwrap();
    frame(cx, s.win);
}

fn drag_into(cx: &mut TestAppContext, s: &Started, from: &str, onto: &str) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row(from)).bounds().center();
        let b = w.find(row(onto)).bounds();
        w.drag(start, point(b.center().x, b.center().y), cx);
    })
    .unwrap();
    frame(cx, s.win);
}

fn focus_tree(cx: &mut TestAppContext, s: &Started) {
    let tree = sidebar(cx, s).read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
}

fn press(cx: &mut TestAppContext, s: &Started, key: &str) {
    cx.update_window(s.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    frame(cx, s.win);
}

fn dirty(cx: &mut TestAppContext, s: &Started, rel: &str) -> bool {
    let snap = ws_snap(cx, &s.ws, &s.root);
    let row = snap.content.rows.iter().find(|r| r.path == ip(rel));
    row.expect("a row").dirty
}

fn exists(s: &Started, rel: &str) -> bool {
    s.root.join(rel).exists()
}

fn shown_now(cx: &mut TestAppContext, s: &Started, id: &'static str) -> bool {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

// The item paths of `dir`'s rows, in sidebar order.
fn children(cx: &mut TestAppContext, s: &Started, dir: &str) -> Vec<String> {
    let snap = ws_snap(cx, &s.ws, &s.root);
    (snap.content.rows.iter())
        .filter(|r| r.path.parent().is_some_and(|p| p.as_str() == dir))
        .map(|r| r.path.as_str().to_owned())
        .collect()
}

fn renamed_and_moved(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let (col, s) = start(cx);
    expand(cx, &s, "todos");
    expand(cx, &s, "users");
    rename(cx, &s, "todos/Create todo.yml", "Add todo");
    assert!(exists(&s, "todos/Add todo.yml"), "renamed");
    drag_into(cx, &s, "todos/Add todo.yml", "users");
    assert!(exists(&s, "users/Add todo.yml"), "moved");
    focus_tree(cx, &s);
    (col, s)
}

// The editor's own Undo after a tree rename and move has no step to take back, so the moved
// request stays as saved.
#[gpui_kit::test]
fn the_editors_undo_after_a_rename_and_move_leaves_the_request_clean(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    expand(cx, &s, "todos");
    expand(cx, &s, "users");
    let tab = open_tab(cx, &s.ws, &s.root, "todos/Create todo.yml");
    frame(cx, s.win);
    rename(cx, &s, "todos/Create todo.yml", "Add todo");
    drag_into(cx, &s, "todos/Add todo.yml", "users");
    assert!(exists(&s, "users/Add todo.yml"), "moved");
    let editor = editor_of(cx, &s, tab);
    cx.update_window(s.win, |_, w, cx| {
        editor.update(cx, |e, cx| e.undo_edit(w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    assert!(!dirty(cx, &s, "users/Add todo.yml"), "nothing to undo");
}

#[gpui_kit::test]
fn ctrl_z_undoes_the_move_then_the_rename_and_redo_reapplies_both(cx: &mut TestAppContext) {
    let (_col, s) = renamed_and_moved(cx);
    let before = std::fs::read_to_string(s.root.join("users/Add todo.yml")).unwrap();
    press(cx, &s, &secondary("z"));
    assert!(exists(&s, "todos/Add todo.yml"), "the move was undone");
    assert!(!exists(&s, "users/Add todo.yml"));
    press(cx, &s, &secondary("z"));
    assert!(exists(&s, "todos/Create todo.yml"), "the rename was undone");
    assert!(!exists(&s, "todos/Add todo.yml"));
    let back = std::fs::read_to_string(s.root.join("todos/Create todo.yml")).unwrap();
    assert!(back.contains("name: Create todo"), "the old name is back");
    press(cx, &s, &secondary("z"));
    assert!(exists(&s, "todos/Create todo.yml"), "nothing older to undo");
    press(cx, &s, &secondary("shift-z"));
    assert!(exists(&s, "todos/Add todo.yml"), "the rename was redone");
    press(cx, &s, &secondary("shift-z"));
    assert!(exists(&s, "users/Add todo.yml"), "the move was redone");
    assert!(!exists(&s, "todos/Add todo.yml"));
    let again = std::fs::read_to_string(s.root.join("users/Add todo.yml")).unwrap();
    assert_eq!(again, before, "redo leaves the file as the move did");
    assert!(!dirty(cx, &s, "users/Add todo.yml"));
}

// With nothing to undo in the tree the key does nothing: the active tab's saved edit stays.
#[gpui_kit::test]
fn ctrl_z_in_the_tree_with_nothing_to_undo_leaves_the_open_request_alone(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    let (_col, s) = start(cx);
    expand(cx, &s, "todos");
    open_tab(cx, &s.ws, &s.root, "todos/Create todo.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("headers").click(("kv-enabled", 0usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    press(cx, &s, &secondary("s"));
    settle(cx);
    let rel = "todos/Create todo.yml";
    assert!(!dirty(cx, &s, rel), "saved");
    let saved = std::fs::read_to_string(s.root.join(rel)).unwrap();
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    settle(cx);
    assert!(!dirty(cx, &s, rel), "the request stays clean");
    assert!(!view_of(cx, &s.ws, &s.root, rel).headers[0].enabled);
    assert_eq!(std::fs::read_to_string(s.root.join(rel)).unwrap(), saved);
}

#[gpui_kit::test]
fn a_multi_selection_move_undoes_together_in_its_old_order(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    expand(cx, &s, "todos");
    expand(cx, &s, "users");
    let order = children(cx, &s, "todos");
    cx.update_window(s.win, |_, w, cx| w.click(row("todos/Create todo.yml"), cx))
        .unwrap();
    secondary_click(cx, s.win, row("todos/List todos.yml"));
    drag_into(cx, &s, "todos/List todos.yml", "users");
    assert!(exists(&s, "users/Create todo.yml") && exists(&s, "users/List todos.yml"));
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    assert!(exists(&s, "todos/Create todo.yml") && exists(&s, "todos/List todos.yml"));
    assert!(!exists(&s, "users/Create todo.yml") && !exists(&s, "users/List todos.yml"));
    assert_eq!(children(cx, &s, "todos"), order, "the old order is back");
}

// A file now at the old path blocks the undo: an error shows and both files stay.
#[gpui_kit::test]
fn an_undo_onto_a_taken_path_shows_an_error_and_changes_nothing(cx: &mut TestAppContext) {
    let (_col, s) = renamed_and_moved(cx);
    let moved = std::fs::read_to_string(s.root.join("users/Add todo.yml")).unwrap();
    let other = request("Someone else", "http://x/other");
    std::fs::write(s.root.join("todos/Add todo.yml"), &other).unwrap();
    press(cx, &s, &secondary("z"));
    assert!(
        shown_now(cx, &s, "toast-error"),
        "the undo says why it failed"
    );
    assert_eq!(
        std::fs::read_to_string(s.root.join("todos/Add todo.yml")).unwrap(),
        other
    );
    assert_eq!(
        std::fs::read_to_string(s.root.join("users/Add todo.yml")).unwrap(),
        moved
    );
}
