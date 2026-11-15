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
        ("users/get.yml", request("Get user", "http://x/users/1")),
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

// Every file under `root` with its bytes.
fn all_bytes(root: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn send_tree(cx: &mut TestAppContext, s: &Started, op: apinata_app::command::TreeOp) {
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.send(&root, apinata_app::command::Command::Tree(op), cx)
    });
    frame(cx, s.win);
}

// A collection the app wrote itself: two folders with numbered requests, as saved files are.
fn written(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    use apinata_app::command::TreeOp;
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    for name in ["Todos", "Users"] {
        let op = TreeOp::CreateFolder {
            parent: apinata_core::ItemPath::root(),
            name: name.into(),
        };
        send_tree(cx, &s, op);
    }
    for (dir, name) in [
        ("Todos", "One"),
        ("Todos", "Two"),
        ("Todos", "Three"),
        ("Users", "Four"),
        ("Users", "Five"),
    ] {
        let op = TreeOp::CreateRequest {
            parent: ip(dir),
            name: name.into(),
        };
        send_tree(cx, &s, op);
    }
    expand(cx, &s, "Todos");
    expand(cx, &s, "Users");
    (col, s)
}

#[gpui_kit::test]
fn undoing_a_rename_restores_the_exact_file_name_and_redo_its_new_one(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    expand(cx, &s, "users");
    rename(cx, &s, "users/get.yml", "Fetch user");
    assert!(exists(&s, "users/Fetch user.yml"));
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    assert!(exists(&s, "users/get.yml"), "the old file name is back");
    assert!(!exists(&s, "users/Get user.yml") && !exists(&s, "users/Fetch user.yml"));
    let text = std::fs::read_to_string(s.root.join("users/get.yml")).unwrap();
    assert!(text.contains("name: Get user"));
    press(cx, &s, &secondary("shift-z"));
    assert!(
        exists(&s, "users/Fetch user.yml"),
        "redo gives the new name again"
    );
    assert!(!exists(&s, "users/get.yml"));
}

#[gpui_kit::test]
fn undoing_a_folder_rename_restores_its_exact_directory_name(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    rename(cx, &s, "users", "People");
    assert!(exists(&s, "People/folder.yml"), "renamed");
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    assert!(
        exists(&s, "users/folder.yml"),
        "the old directory name is back"
    );
    assert!(!exists(&s, "People"));
    assert!(exists(&s, "users/get.yml"));
    press(cx, &s, &secondary("shift-z"));
    assert!(exists(&s, "People/get.yml"), "redo renames it again");
    assert!(!exists(&s, "users"));
}

// A move and its undo leave every file as it was, numbers and all.
#[gpui_kit::test]
fn a_move_and_its_undo_leave_every_file_byte_identical(cx: &mut TestAppContext) {
    let (_col, s) = written(cx);
    let before = all_bytes(&s.root);
    drag_into(cx, &s, "Todos/Two.yml", "Users");
    assert!(exists(&s, "Users/Two.yml"), "moved");
    let moved = all_bytes(&s.root);
    assert_ne!(moved, before);
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    assert_eq!(all_bytes(&s.root), before, "the undo put every byte back");
    press(cx, &s, &secondary("shift-z"));
    assert_eq!(all_bytes(&s.root), moved, "the redo is the move again");
}

#[gpui_kit::test]
fn a_multi_selection_move_and_its_undo_leave_every_file_byte_identical(cx: &mut TestAppContext) {
    let (_col, s) = written(cx);
    let before = all_bytes(&s.root);
    cx.update_window(s.win, |_, w, cx| w.click(row("Todos/One.yml"), cx))
        .unwrap();
    secondary_click(cx, s.win, row("Todos/Three.yml"));
    drag_into(cx, &s, "Todos/Three.yml", "Users");
    assert!(exists(&s, "Users/One.yml") && exists(&s, "Users/Three.yml"));
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    assert_eq!(all_bytes(&s.root), before);
}

// A reorder inside one folder is undone as exactly.
#[gpui_kit::test]
fn a_reorder_and_its_undo_leave_every_file_byte_identical(cx: &mut TestAppContext) {
    let (_col, s) = written(cx);
    let before = all_bytes(&s.root);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let start = w.find(row("Todos/Three.yml")).bounds().center();
        let b = w.find(row("Todos/One.yml")).bounds();
        w.drag(
            start,
            point(b.center().x, b.origin.y + b.size.height * 0.1),
            cx,
        );
    })
    .unwrap();
    frame(cx, s.win);
    assert_ne!(all_bytes(&s.root), before, "reordered");
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    assert_eq!(all_bytes(&s.root), before);
}

// Redo's keys in a text field are the field's: they never redo a tree step.
fn undone_rename(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let (col, s) = start(cx);
    expand(cx, &s, "users");
    rename(cx, &s, "users/get.yml", "Fetch user");
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    assert!(exists(&s, "users/get.yml"));
    (col, s)
}

fn redo_keys(cx: &mut TestAppContext, s: &Started) {
    let mut keys = vec![secondary("shift-z")];
    if !cfg!(target_os = "macos") {
        keys.push("ctrl-y".into());
    }
    for key in keys {
        press(cx, s, &key);
        assert!(exists(s, "users/get.yml"), "{key} left the tree alone");
    }
}

#[gpui_kit::test]
fn redo_keys_in_the_filter_field_leave_the_tree_alone(cx: &mut TestAppContext) {
    let (_col, s) = undone_rename(cx);
    press(cx, &s, &secondary("f"));
    cx.update_window(s.win, |_, w, cx| w.input("get", cx))
        .unwrap();
    frame(cx, s.win);
    redo_keys(cx, &s);
}

#[gpui_kit::test]
fn redo_keys_in_the_rename_field_leave_the_tree_alone(cx: &mut TestAppContext) {
    let (_col, s) = undone_rename(cx);
    let view = sidebar(cx, &s);
    let root = s.root.clone();
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| {
            v.start_rename(&root, &ip("users/get.yml"), w, cx)
        })
    })
    .unwrap();
    frame(cx, s.win);
    redo_keys(cx, &s);
    assert!(
        view.read_with(cx, |v, _| v.renaming()).is_some(),
        "still renaming"
    );
}

fn palette_lists(cx: &mut TestAppContext, s: &Started, label: &str) -> bool {
    cx.simulate_keystrokes(s.win, &secondary("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input(label, cx))
        .unwrap();
    frame(cx, s.win);
    let found = cx
        .update_window(s.win, |_, w, _| {
            gpui_kit::base::test_support::snapshots(w)
                .iter()
                .filter(|n| n.role() == Some(gpui_kit::Role::ListBoxOption))
                .any(|n| n.label().is_some_and(|l| l.starts_with(label)))
        })
        .unwrap();
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    found
}

#[gpui_kit::test]
fn the_palette_offers_undo_and_redo_in_sidebar_while_each_has_a_step(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    assert!(!palette_lists(cx, &s, "Undo in sidebar"), "nothing to undo");
    assert!(!palette_lists(cx, &s, "Redo in sidebar"), "nothing to redo");
    expand(cx, &s, "users");
    rename(cx, &s, "users/get.yml", "Fetch user");
    assert!(palette_lists(cx, &s, "Undo in sidebar"), "a rename to undo");
    assert!(!palette_lists(cx, &s, "Redo in sidebar"));
    focus_tree(cx, &s);
    press(cx, &s, &secondary("z"));
    assert!(
        !palette_lists(cx, &s, "Undo in sidebar"),
        "the rename was undone"
    );
    assert!(
        palette_lists(cx, &s, "Redo in sidebar"),
        "and can be redone"
    );
}
