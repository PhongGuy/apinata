//! The Changes dialog's states: deleted, binary and non-request files, a list that follows
//! refreshes down to empty, and the quiet state when git goes away.

use crate::git_support;
use crate::support;

use std::path::PathBuf;

use apinata_app::changes_dialog::{ChangesView, current, open_changes};
use apinata_app::diff_doc::{DiffBody, DiffDoc, RowKind};
use apinata_app::git_status::{GitChange, QUIET_NOTE};
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;

fn committed(sb: &Sandbox, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = write_tree(files);
    sb.git(dir.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(dir.path(), "init");
    dir
}

/// The real app over `col` with git on through the sandbox.
fn git_started(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    let s = started(cx, col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    s
}

/// `a.yml` modified, `b.yml` committed unchanged, `new.yml` untracked.
fn changed(sb: &Sandbox) -> tempfile::TempDir {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            ("b.yml", &request("b", "http://x/b")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    write(col.path(), "new.yml", &request("n", "http://x/n"));
    col
}

fn open_on(cx: &mut TestAppContext, s: &Started, select: Option<&str>) {
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let select = select.map(PathBuf::from);
    cx.update_window(s.win, |_, w, cx| open_changes(&ws, &root, select, w, cx))
        .unwrap();
    frame(cx, s.win);
}

fn view(cx: &mut TestAppContext) -> Entity<ChangesView> {
    cx.update(|cx| current(cx)).expect("the dialog is open")
}

fn files(cx: &mut TestAppContext) -> Vec<(String, GitChange)> {
    let v = view(cx);
    v.read_with(cx, |v, _| {
        v.files()
            .iter()
            .map(|(p, c)| (p.to_string_lossy().into_owned(), c.clone()))
            .collect()
    })
}

fn selected(cx: &mut TestAppContext) -> Option<String> {
    let v = view(cx);
    v.read_with(cx, |v, _| {
        v.selected().map(|p| p.to_string_lossy().into_owned())
    })
}

fn doc(cx: &mut TestAppContext) -> DiffDoc {
    let v = view(cx);
    v.read_with(cx, |v, _| v.doc().cloned())
        .expect("the diff loaded")
}

fn label(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
) -> Option<String> {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_deleted_file_is_listed_with_its_removed_lines(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    std::fs::remove_file(col.path().join("b.yml")).unwrap();
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("b.yml"));
    assert!(files(cx).contains(&("b.yml".to_owned(), GitChange::Deleted)));
    assert_eq!(selected(cx).as_deref(), Some("b.yml"));
    let removed_only = match doc(cx).body {
        DiffBody::Lines(rows) => rows
            .iter()
            .all(|r| matches!(r.kind, RowKind::Header | RowKind::Removed)),
        _ => false,
    };
    assert!(removed_only);
}

#[gpui_kit::test]
fn a_binary_file_shows_a_note_in_words(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    std::fs::write(col.path().join("b.yml"), [0xffu8, 0xfe, b'\n']).unwrap();
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("b.yml"));
    assert_eq!(
        label(cx, &s, "changes-note").as_deref(),
        Some("Binary file changed")
    );
    assert!(!present(cx, s.win, ("diff-line", 0usize)));
}

#[gpui_kit::test]
fn a_changed_non_request_file_is_listed_with_a_fixed_note(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    write(col.path(), "README.md", "notes\n");
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("README.md"));
    assert!(files(cx).contains(&("README.md".to_owned(), GitChange::Untracked)));
    assert_eq!(selected(cx).as_deref(), Some("README.md"));
    assert_eq!(
        label(cx, &s, "changes-note").as_deref(),
        Some("Only request files (.yml, .yaml) show their changes here.")
    );
}

#[gpui_kit::test]
fn the_list_follows_refreshes_down_to_empty(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    write(col.path(), "a.yml", &request("a", "http://x/a"));
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    assert_eq!(files(cx), [("new.yml".to_owned(), GitChange::Untracked)]);
    assert_eq!(selected(cx).as_deref(), Some("new.yml"));
    std::fs::remove_file(col.path().join("new.yml")).unwrap();
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    assert!(files(cx).is_empty());
    assert!(present(cx, s.win, "changes-empty"));
}

#[gpui_kit::test]
fn git_going_away_shows_the_quiet_state(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    s.ws.update(cx, |w, cx| w.set_git(None, cx));
    frame(cx, s.win);
    assert!(files(cx).is_empty());
    assert_eq!(selected(cx), None);
    assert_eq!(label(cx, &s, "changes-quiet").as_deref(), Some(QUIET_NOTE));
    assert!(!present(cx, s.win, "changes-empty"));
    assert!(!present(cx, s.win, ("diff-line", 0usize)));
}

// A file removed after the last refresh is still listed; its diff is core's error in words.
#[gpui_kit::test]
fn a_file_neither_on_disk_nor_committed_shows_the_error_note(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    std::fs::remove_file(col.path().join("new.yml")).unwrap();
    let v = view(cx);
    v.update(cx, |v, cx| v.select(1, cx));
    frame(cx, s.win);
    assert_eq!(selected(cx).as_deref(), Some("new.yml"));
    assert_eq!(
        label(cx, &s, "changes-note").as_deref(),
        Some("Could not read the changes: new.yml is not on disk or in HEAD")
    );
    assert!(!present(cx, s.win, ("diff-line", 0usize)));
}

// Whether the loaded diff has an added line containing `text`.
fn adds(cx: &mut TestAppContext, text: &str) -> bool {
    match doc(cx).body {
        DiffBody::Lines(rows) => rows
            .iter()
            .any(|r| r.kind == RowKind::Added && r.text.contains(text)),
        _ => false,
    }
}

#[gpui_kit::test]
fn a_refresh_with_the_same_status_reloads_the_diff(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    assert!(adds(cx, "http://x/changed"));
    write(col.path(), "a.yml", &request("a", "http://x/again"));
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
    assert!(adds(cx, "http://x/again"));
}

#[gpui_kit::test]
fn an_unrelated_workspace_notify_leaves_the_diff_alone(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    // A refresh that finds the same status must not leave a pending reload behind.
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    write(col.path(), "a.yml", &request("a", "http://x/again"));
    s.ws.update(cx, |_, cx| cx.notify());
    frame(cx, s.win);
    assert!(adds(cx, "http://x/changed"));
}

#[gpui_kit::test]
fn the_selection_follows_its_path_when_the_list_shifts(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("new.yml"));
    assert_eq!(files(cx)[1].0, "new.yml");
    write(col.path(), "a.yml", &request("a", "http://x/a"));
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    assert_eq!(files(cx), [("new.yml".to_owned(), GitChange::Untracked)]);
    assert_eq!(selected(cx).as_deref(), Some("new.yml"));
    assert!(adds(cx, "http://x/n"));
}

// A commit made outside the app, with no save or focus change to refresh status, must not
// leave the dialog listing files that no longer differ.
#[gpui_kit::test]
fn opening_refreshes_a_status_committed_elsewhere(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    sb.commit_all(col.path(), "outside");
    open_on(cx, &s, None);
    assert!(files(cx).is_empty());
    assert!(present(cx, s.win, "changes-empty"));
}

#[gpui_kit::test]
fn the_empty_note_keeps_to_one_line_in_the_list(cx: &mut TestAppContext) {
    use apinata_app::style::{SPACE_2, at_zoom};
    use gpui_kit::px;
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    write(col.path(), "a.yml", &request("a", "http://x/a"));
    std::fs::remove_file(col.path().join("new.yml")).unwrap();
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    assert!(files(cx).is_empty());
    cx.update_window(s.win, |_, w, _| {
        let list = w.find("changes-files").bounds();
        let note = w.find("changes-empty").bounds();
        assert!(
            note.right() <= list.right() + px(0.5),
            "{note:?} leaves {list:?}"
        );
        assert!(
            note.size.height <= at_zoom(SPACE_2, 1.0) * 2. + px(24.),
            "the note wraps: {note:?}"
        );
    })
    .unwrap();
}
