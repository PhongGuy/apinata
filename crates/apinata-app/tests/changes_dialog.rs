//! The Changes dialog: opening, the changed files with their words, the selected file's diff,
//! and selection.

use crate::git_support;
use crate::support;

use std::path::PathBuf;

use apinata_app::changes_dialog::{ChangesView, current, open_changes};
use apinata_app::diff_doc::{DiffDoc, RowKind};
use apinata_app::git_status::GitChange;
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
fn it_lists_changed_files_and_shows_the_first_diff(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    assert_eq!(
        files(cx),
        [
            ("a.yml".to_owned(), GitChange::Modified),
            ("new.yml".to_owned(), GitChange::Untracked),
        ]
    );
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
    assert_eq!(doc(cx).sections, ["URL"]);
    assert_eq!(
        label(cx, &s, ("changes-file", 0usize)).as_deref(),
        Some("modified a.yml")
    );
    assert_eq!(
        label(cx, &s, ("changes-file", 1usize)).as_deref(),
        Some("untracked new.yml")
    );
    assert!(present(cx, s.win, ("diff-line", 0usize)));
    assert!(present(cx, s.win, ("diff-gutter", 1usize)));
    assert!(!present(cx, s.win, "changes-note"));
}

#[gpui_kit::test]
fn opening_on_a_file_selects_it_and_a_click_switches(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("new.yml"));
    assert_eq!(selected(cx).as_deref(), Some("new.yml"));
    let kinds: Vec<RowKind> = match doc(cx).body {
        apinata_app::diff_doc::DiffBody::Lines(rows) => rows.into_iter().map(|r| r.kind).collect(),
        other => panic!("expected lines, got {other:?}"),
    };
    assert!(
        kinds
            .iter()
            .all(|k| matches!(k, RowKind::Header | RowKind::Added))
    );
    cx.update_window(s.win, |_, w, cx| w.click(("changes-file", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
    assert_eq!(doc(cx).sections, ["URL"]);
}

// Random scheduling lets the first selection's diff land after the second's in some iterations.
#[gpui_kit::test(iterations = 10)]
fn a_quick_reselection_shows_the_last_file(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    let v = view(cx);
    v.update(cx, |v, cx| {
        v.select(1, cx);
        v.select(0, cx);
    });
    frame(cx, s.win);
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
    assert_eq!(doc(cx).sections, ["URL"]);
}

#[gpui_kit::test]
fn an_out_of_range_selection_does_nothing(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    let v = view(cx);
    v.update(cx, |v, cx| v.select(9, cx));
    frame(cx, s.win);
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
    assert_eq!(doc(cx).sections, ["URL"]);
}

#[gpui_kit::test]
fn without_git_state_nothing_opens(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = started(cx, &col);
    open_on(cx, &s, None);
    assert!(cx.update(|cx| current(cx)).is_none());
}

#[gpui_kit::test]
fn the_first_selection_prefers_a_request_over_a_note_file(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    write(col.path(), "README.md", "notes\n");
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    assert_eq!(files(cx)[0].0, "README.md");
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
}

#[gpui_kit::test]
fn without_a_request_the_first_file_is_selected(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "README.md", "notes\n");
    write(col.path(), "notes.txt", "more\n");
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    assert_eq!(selected(cx).as_deref(), Some("README.md"));
}

fn bounds(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<gpui_kit::ElementId>,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

// The index of the first diff row of `kind`.
fn first_row(cx: &mut TestAppContext, kind: RowKind) -> usize {
    match doc(cx).body {
        apinata_app::diff_doc::DiffBody::Lines(rows) => {
            rows.iter().position(|r| r.kind == kind).expect("a row")
        }
        other => panic!("expected lines, got {other:?}"),
    }
}

#[gpui_kit::test]
fn added_and_removed_tints_fill_the_row(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    let pane = bounds(cx, &s, "changes-diff");
    for kind in [RowKind::Added, RowKind::Removed] {
        let ix = first_row(cx, kind);
        let row = bounds(cx, &s, ("diff-line", ix));
        // Only the pane's one-pixel border on each side may show past the tint.
        assert!(
            row.size.width + gpui_kit::px(2.5) >= pane.size.width,
            "{kind:?} row {row:?} in pane {pane:?}"
        );
    }
}

fn resolve(cx: &mut TestAppContext, s: &Started, len: gpui_kit::Rems) -> gpui_kit::Pixels {
    cx.update_window(s.win, |_, w, _| apinata_app::style::resolve(len, w))
        .unwrap()
}

#[gpui_kit::test]
fn line_numbers_keep_off_the_left_border(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    let pane = bounds(cx, &s, "changes-diff");
    let ix = first_row(cx, RowKind::Context);
    let number = bounds(cx, &s, ("diff-old", ix));
    let room = gpui_kit::px(1.) + resolve(cx, &s, apinata_app::style::SPACE_1);
    assert!(
        number.left() - pane.left() >= room,
        "number {number:?} in pane {pane:?}"
    );
}

#[gpui_kit::test]
fn file_names_line_up_whatever_the_status_word(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    let modified = bounds(cx, &s, ("changes-file-name", 0usize));
    let untracked = bounds(cx, &s, ("changes-file-name", 1usize));
    assert_eq!(modified.left(), untracked.left());
}

#[gpui_kit::test]
fn the_pane_top_holds_without_section_chips(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    write(col.path(), "README.md", "notes\n");
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    assert_eq!(doc(cx).sections, ["URL"]);
    let with_chips = bounds(cx, &s, "changes-diff").top();
    view(cx).update(cx, |v, cx| v.select(0, cx));
    frame(cx, s.win);
    assert!(doc(cx).sections.is_empty());
    let without = bounds(cx, &s, "changes-note").top();
    assert_eq!(with_chips, without);
}

#[gpui_kit::test]
fn the_dialog_closes_from_one_footer_button(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    assert!(!present(cx, s.win, "close"), "no corner close beside Close");
    let footer = bounds(cx, &s, "dialog-footer");
    // Commit is the primary button at the right edge; Close sits before it.
    assert_eq!(bounds(cx, &s, "changes-commit").right(), footer.right());
    assert!(bounds(cx, &s, "changes-close").right() <= bounds(cx, &s, "changes-commit").left());
}

#[gpui_kit::test]
fn the_selected_file_carries_the_tree_edge(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    assert!(present(cx, s.win, ("changes-file-focus-edge", 0usize)));
    // With focus moved on to Close, the edge is the neutral one.
    press(cx, &s, "tab");
    assert!(present(cx, s.win, ("changes-file-edge", 0usize)));
    assert!(!present(cx, s.win, ("changes-file-focus-edge", 0usize)));
    assert!(!present(cx, s.win, ("changes-file-edge", 1usize)));
    // A click focuses the list, so the edge turns to the accent as in the tree.
    cx.update_window(s.win, |_, w, cx| w.click(("changes-file", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(present(cx, s.win, ("changes-file-focus-edge", 1usize)));
    assert!(!present(cx, s.win, ("changes-file-edge", 0usize)));
    assert!(!present(cx, s.win, ("changes-file-focus-edge", 0usize)));
    assert_eq!(
        bounds(cx, &s, ("changes-file-focus-edge", 1usize)),
        bounds(cx, &s, ("changes-file", 1usize)),
        "the edge sits on the row's own box"
    );
}

// A narrow window stacks the file list over the diff, so the diff keeps the dialog's width
// instead of the list's leftovers.
#[gpui_kit::test]
fn a_narrow_dialog_stacks_the_list_over_the_diff(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(320.), gpui_kit::px(700.)));
    open_on(cx, &s, Some("a.yml"));
    frame(cx, s.win);
    let dialog = bounds(cx, &s, "changes-dialog");
    let list = bounds(cx, &s, "changes-files");
    let diff = bounds(cx, &s, "changes-diff");
    assert!(
        list.bottom() <= diff.top(),
        "list {list:?} over diff {diff:?}"
    );
    assert!(
        diff.size.width + gpui_kit::px(1.) >= dialog.size.width,
        "diff {diff:?} in dialog {dialog:?}"
    );
    assert!(list.size.width + gpui_kit::px(1.) >= dialog.size.width);
    assert!(diff.size.height >= resolve(cx, &s, apinata_app::style::ROW_HEIGHT) * 4.);
}

#[gpui_kit::test]
fn a_wide_dialog_keeps_the_list_beside_the_diff(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    frame(cx, s.win);
    let list = bounds(cx, &s, "changes-files");
    let diff = bounds(cx, &s, "changes-diff");
    assert!(
        list.right() <= diff.left(),
        "list {list:?} beside diff {diff:?}"
    );
    assert_eq!(
        list.size.width,
        resolve(cx, &s, apinata_app::style::CHANGES_FILES_WIDTH)
    );
}

fn press(cx: &mut TestAppContext, s: &Started, key: &str) {
    cx.update_window(s.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn up_and_down_in_the_list_move_the_selection_and_its_diff(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    let names: Vec<String> = files(cx).into_iter().map(|(p, _)| p).collect();
    assert_eq!(names, ["a.yml", "new.yml"]);
    cx.update_window(s.win, |_, w, cx| w.click(("changes-file", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    let listed = cx
        .update_window(s.win, |_, w, _| w.find("changes-files").focused())
        .unwrap();
    assert_eq!(listed, Some(true), "the click leaves the list focused");
    press(cx, &s, "up");
    assert_eq!(selected(cx).as_deref(), Some("a.yml"), "no wrap at the top");
    press(cx, &s, "down");
    assert_eq!(selected(cx).as_deref(), Some("new.yml"));
    let added = match doc(cx).body {
        apinata_app::diff_doc::DiffBody::Lines(rows) => rows
            .iter()
            .all(|r| matches!(r.kind, RowKind::Header | RowKind::Added)),
        other => panic!("expected lines, got {other:?}"),
    };
    assert!(added, "the new file's diff loaded");
    press(cx, &s, "down");
    assert_eq!(
        selected(cx).as_deref(),
        Some("new.yml"),
        "no wrap at the end"
    );
    press(cx, &s, "up");
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
    assert_eq!(doc(cx).sections, ["URL"]);
}

#[test]
fn the_stacked_list_is_two_fifths_with_three_rows_at_least() {
    use apinata_app::changes_dialog::stacked_list_height;
    use gpui_kit::px;
    let near = |a: gpui_kit::Pixels, b: f32| (f32::from(a) - b).abs() < 0.01;
    assert!(near(stacked_list_height(px(500.), px(28.)), 200.));
    assert!(near(stacked_list_height(px(100.), px(28.)), 84.));
}

#[gpui_kit::test]
fn the_stacked_list_takes_its_share_of_the_panes(cx: &mut TestAppContext) {
    use apinata_app::changes_dialog::stacked_list_height;
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    let row = resolve(cx, &s, apinata_app::style::ROW_HEIGHT);
    for height in [700., 420.] {
        gpui_kit::VisualTestContext::from_window(s.win, cx)
            .simulate_resize(gpui_kit::size(gpui_kit::px(320.), gpui_kit::px(height)));
        open_on(cx, &s, Some("a.yml"));
        frame(cx, s.win);
        let panes = bounds(cx, &s, "changes-panes");
        let list = bounds(cx, &s, "changes-files");
        let want = stacked_list_height(panes.size.height, row);
        assert!(
            (f32::from(list.size.height) - f32::from(want)).abs() < 0.5,
            "{height}: list {list:?}, want {want:?} of {panes:?}"
        );
        assert!(list.size.height + gpui_kit::px(0.5) >= row * 3.);
        cx.update_window(s.win, |_, w, cx| {
            gpui_kit::component::WindowExt::close_dialog(w, cx)
        })
        .unwrap();
        frame(cx, s.win);
    }
}

fn list_focused(cx: &mut TestAppContext, s: &Started) -> Option<bool> {
    cx.update_window(s.win, |_, w, _| w.find("changes-files").focused())
        .unwrap()
}

// The dialog opens with its list focused, so Up and Down work at once.
#[gpui_kit::test]
fn the_list_is_focused_when_the_dialog_opens(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    frame(cx, s.win);
    assert_eq!(list_focused(cx, &s), Some(true), "the list holds focus");
    press(cx, &s, "down");
    assert_eq!(selected(cx).as_deref(), Some("new.yml"));
}

// The list is a Tab stop: Tab moves on through Stage all and the message field to Close (Commit
// is disabled with nothing staged) and comes back round to the list.
#[gpui_kit::test]
fn tab_cycles_between_the_list_and_close(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    frame(cx, s.win);
    press(cx, &s, "tab");
    let bulk = cx
        .update_window(s.win, |_, w, _| w.find("changes-toggle-all").focused())
        .unwrap();
    assert_eq!(bulk, Some(true), "Tab moves from the list to Stage all");
    press(cx, &s, "tab");
    assert_eq!(list_focused(cx, &s), Some(false), "Tab leaves the list");
    press(cx, &s, "tab");
    let close = cx
        .update_window(s.win, |_, w, _| w.find("changes-close").focused())
        .unwrap();
    assert_eq!(close, Some(true), "Tab reaches Close");
    press(cx, &s, "tab");
    assert_eq!(
        list_focused(cx, &s),
        Some(true),
        "Tab comes back to the list"
    );
}

fn clean(sb: &Sandbox) -> tempfile::TempDir {
    committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    )
}

// Opened on a clean tree, the dialog focuses its list once a refresh fills it, while focus
// is still where the dialog put it.
#[gpui_kit::test]
fn a_list_filled_after_open_takes_focus(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = clean(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    assert!(files(cx).is_empty());
    // A clean tree draws its note in place of the list, so nothing there can hold focus.
    assert!(!present(cx, s.win, "changes-files"));
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    assert_eq!(files(cx).len(), 1);
    assert_eq!(
        list_focused(cx, &s),
        Some(true),
        "the filled list takes focus"
    );
    press(cx, &s, "down");
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
}

// Focus moved on inside the dialog stays where the user put it when the list fills.
#[gpui_kit::test]
fn a_list_filled_after_focus_moved_leaves_focus_alone(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = clean(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, None);
    tab_to(cx, s.win, "changes-close");
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    assert_eq!(files(cx).len(), 1);
    assert_eq!(list_focused(cx, &s), Some(false), "focus stays on Close");
}

fn opened_on(sb: &Sandbox, col: &tempfile::TempDir, cx: &mut TestAppContext) -> Started {
    let s = git_started(cx, sb, col);
    open_on(cx, &s, None);
    s
}

#[gpui_kit::test]
fn a_changed_environment_file_is_selected_before_a_note(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            ("environments/dev.yml", "name: dev\nvariables: []\n"),
        ],
    );
    write(col.path(), "README.md", "notes\n");
    write(
        col.path(),
        "environments/dev.yml",
        "name: dev2\nvariables: []\n",
    );
    opened_on(&sb, &col, cx);
    assert_eq!(files(cx)[0].0, "README.md");
    assert_eq!(selected(cx).as_deref(), Some("environments/dev.yml"));
}

#[gpui_kit::test]
fn a_changed_folder_file_is_selected_before_a_note(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("f/folder.yml", &folder("F")),
            ("f/a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "README.md", "notes\n");
    write(col.path(), "f/folder.yml", &folder("G"));
    opened_on(&sb, &col, cx);
    assert_eq!(selected(cx).as_deref(), Some("f/folder.yml"));
}

#[gpui_kit::test]
fn a_request_not_loaded_yet_is_selected_before_a_note(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = clean(&sb);
    let s = git_started(cx, &sb, &col);
    // Written after the collection loaded, so the workspace may not hold it when the
    // dialog opens; the list still counts it as a request.
    write(col.path(), "README.md", "notes\n");
    write(col.path(), "c.yml", &request("c", "http://x/c"));
    s.ws.update(cx, |w, cx| w.refresh_git(&s.root, cx));
    frame(cx, s.win);
    open_on(cx, &s, None);
    assert_eq!(files(cx)[0].0, "README.md");
    assert_eq!(selected(cx).as_deref(), Some("c.yml"));
}

#[gpui_kit::test]
fn opening_with_unchanged_status_reads_the_diff_once(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    opened_on(&sb, &col, cx);
    let v = view(cx);
    assert_eq!(v.read_with(cx, |v, _| v.diff_loads()), 1);
}

#[gpui_kit::test]
fn enter_on_the_list_keeps_the_dialog_and_the_typed_message(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    let v = view(cx);
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| v.set_message("wip", w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(list_focused(cx, &s), Some(true));
    press(cx, &s, "enter");
    cx.run_until_parked();
    let open = cx
        .update_window(s.win, |_, w, cx| {
            gpui_kit::component::WindowExt::has_active_dialog(w, cx)
        })
        .unwrap();
    assert!(open, "the dialog stays open");
    assert_eq!(view(cx).read_with(cx, |v, cx| v.message(cx)), "wip");
}

// The kit dialog body clips flush with the list's top, so the ring must be drawn far enough
// inside the list that its outside band is not cut.
#[gpui_kit::test]
fn the_focused_list_ring_keeps_inside_the_dialog_body(cx: &mut TestAppContext) {
    use apinata_app::style::FOCUS_RING_ROOM;
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    assert_eq!(list_focused(cx, &s), Some(true));
    let body = bounds(cx, &s, "changes-dialog");
    let list = bounds(cx, &s, "changes-files");
    let ring = bounds(cx, &s, "changes-files-ring");
    assert!(
        ring.top() - FOCUS_RING_ROOM >= body.top(),
        "{ring:?} in {body:?}"
    );
    assert!(
        ring.top() - FOCUS_RING_ROOM >= list.top(),
        "{ring:?} in {list:?}"
    );
}

// The list takes ring room above its header; beside it the diff takes the same, so the chip
// row and the list header share a top and the diff box keeps a band's room under the clip.
#[gpui_kit::test]
fn the_diff_chips_line_up_with_the_list_header_beside_it(cx: &mut TestAppContext) {
    use apinata_app::style::FOCUS_RING_ROOM;
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, Some("a.yml"));
    let body = bounds(cx, &s, "changes-dialog");
    let list = bounds(cx, &s, "changes-files");
    let chips = bounds(cx, &s, "changes-sections");
    assert_eq!(
        chips.top(),
        list.top() + FOCUS_RING_ROOM,
        "{chips:?} beside {list:?}"
    );
    assert!(
        chips.top() - FOCUS_RING_ROOM >= body.top(),
        "{chips:?} in {body:?}"
    );
}
