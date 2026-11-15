//! Clashes in the sync dialog: Compare shows the saved side against the file, and each answer settles it.

use crate::git_support::{Sandbox, write};
use crate::support::team::*;
use crate::support::*;
use apinata_app::compare_dialog;
use apinata_core::team::Role;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::time::Duration;

const FAST: Duration = Duration::from_millis(50);

fn with_notes() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("notes.txt", "theirs\n"),
    ])
}

fn app_with_copy(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    app_with_copy_as(cx, sb, col, Role::Write)
}

fn app_with_copy_as(
    cx: &mut TestAppContext,
    sb: &Sandbox,
    col: &tempfile::TempDir,
    role: Role,
) -> Started {
    http_ready(cx);
    let s = team_started(cx, sb, col, |_, _| {}).0;
    std::fs::create_dir_all(copies_of(&s)).unwrap();
    std::fs::write(copies_of(&s).join("notes.txt"), "mine\n").unwrap();
    start_team_as(cx, sb, &s, role, FAST);
    wait_for(cx, "the clash", |cx| {
        s.ws.read_with(cx, |w, _| w.team_clashes(&s.root).len() == 1)
    });
    s
}

fn open_sync(cx: &mut TestAppContext, s: &Started) {
    let (ws, root) = (s.ws.clone(), s.root.clone());
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::team_sync_dialog::open(&ws, root, w, cx)
    })
    .unwrap();
    frame(cx, s.win);
}

fn click(cx: &mut TestAppContext, s: &Started, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn compare_shows_the_file_against_the_saved_side_and_use_mine_pushes_it(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    let bare = shared_origin(&sb, col.path());
    let s = app_with_copy(cx, &sb, &col);
    open_sync(cx, &s);
    assert!(shown(cx, s.win, ("team-clash", 0usize)));
    click(cx, &s, ("team-compare", 0usize));
    assert!(shown(cx, s.win, "compare-diff"));
    click(cx, &s, "compare-use-mine");
    assert_eq!(
        std::fs::read_to_string(col.path().join("notes.txt")).unwrap(),
        "mine\n"
    );
    assert!(!copies_of(&s).join("notes.txt").exists());
    wait_for(cx, "mine pushed", |_| {
        origin_has(&sb, &bare, "notes.txt", "mine")
    });
}

#[gpui_kit::test]
fn open_copy_holds_the_disk_text_and_the_saved_side(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    shared_origin(&sb, col.path());
    let s = app_with_copy(cx, &sb, &col);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let dialog = cx
        .update_window(s.win, |_, w, cx| {
            compare_dialog::open_copy(
                ws,
                root,
                "notes.txt".into(),
                "theirs\n".into(),
                "mine\n".into(),
                w,
                cx,
            )
        })
        .unwrap();
    frame(cx, s.win);
    let (disk, mine, rows) = dialog.read_with(cx, |d, _| {
        (d.disk().to_owned(), d.draft().to_owned(), d.rows().len())
    });
    assert_eq!((disk.as_str(), mine.as_str()), ("theirs\n", "mine\n"));
    assert!(rows >= 2);
    click(cx, &s, "compare-keep-theirs");
    assert_eq!(
        std::fs::read_to_string(col.path().join("notes.txt")).unwrap(),
        "theirs\n"
    );
    assert!(!copies_of(&s).join("notes.txt").exists());
}

#[gpui_kit::test]
fn keep_theirs_in_the_row_drops_the_copy(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    shared_origin(&sb, col.path());
    let s = app_with_copy(cx, &sb, &col);
    open_sync(cx, &s);
    click(cx, &s, ("team-keep-theirs", 0usize));
    assert!(!copies_of(&s).join("notes.txt").exists());
    assert!(!shown(cx, s.win, ("team-clash", 0usize)));
}

#[gpui_kit::test]
fn a_read_only_folder_offers_only_keep_theirs(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    shared_origin(&sb, col.path());
    let s = app_with_copy_as(cx, &sb, &col, Role::Read);
    open_sync(cx, &s);
    assert!(shown(cx, s.win, ("team-clash", 0usize)));
    assert!(shown(cx, s.win, ("team-compare", 0usize)));
    assert!(!shown(cx, s.win, ("team-use-mine", 0usize)));
    click(cx, &s, ("team-compare", 0usize));
    assert!(shown(cx, s.win, "compare-diff"));
    assert!(!shown(cx, s.win, "compare-use-mine"));
    assert!(shown(cx, s.win, "compare-keep-theirs"));
}

#[gpui_kit::test]
fn delete_again_removes_the_file_teammates_changed(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let s = team_started(cx, &sb, &col, |_, _| {}).0;
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let mate = sb.path().join("mate");
    write(&mate, "a.yml", &request("a", "http://x/theirs"));
    sb.commit_all(&mate, "theirs");
    sb.git(&mate, &["push", "-q"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    wait_for(cx, "the clash", |cx| {
        s.ws.read_with(cx, |w, _| w.team_clashes(&s.root).len() == 1)
    });
    open_sync(cx, &s);
    assert!(!shown(cx, s.win, ("team-compare", 0usize)));
    click(cx, &s, ("team-delete-again", 0usize));
    assert!(!col.path().join("a.yml").exists());
    assert!(!shown(cx, s.win, ("team-clash", 0usize)));
    wait_for(cx, "the delete pushed", |_| {
        !origin_has(&sb, &bare, "a.yml", "http")
    });
}

#[gpui_kit::test]
fn many_clashes_scroll_and_keep_close_in_the_window(cx: &mut TestAppContext) {
    http_ready(cx);
    let (sb, col) = (Sandbox::new(), with_notes());
    shared_origin(&sb, col.path());
    let s = team_started(cx, &sb, &col, |_, _| {}).0;
    std::fs::create_dir_all(copies_of(&s)).unwrap();
    for i in 0..80 {
        std::fs::write(copies_of(&s).join(format!("n{i}.txt")), "mine\n").unwrap();
    }
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    wait_for(cx, "the clashes", |cx| {
        s.ws.read_with(cx, |w, _| w.team_clashes(&s.root).len() == 80)
    });
    open_sync(cx, &s);
    let (close, window) = cx
        .update_window(s.win, |_, w, _| {
            (
                w.try_find("team-sync-close").map(|e| e.bounds()),
                w.viewport_size(),
            )
        })
        .unwrap();
    let close = close.expect("Close is shown");
    assert!(
        close.bottom() <= window.height,
        "{close:?} outside {window:?}"
    );
}

// A symlink at the clash path, to a file holding the saved side's text: reading through it
// would show the two sides as the same.
#[cfg(unix)]
#[gpui_kit::test]
fn compare_never_reads_through_a_symlink(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("target.txt"), "mine\n").unwrap();
    std::fs::remove_file(col.path().join("notes.txt")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("target.txt"),
        col.path().join("notes.txt"),
    )
    .unwrap();
    shared_origin(&sb, col.path());
    let s = app_with_copy(cx, &sb, &col);
    open_sync(cx, &s);
    click(cx, &s, ("team-compare", 0usize));
    assert!(shown(cx, s.win, "compare-diff"));
    assert!(!shown(cx, s.win, "compare-same"));
}
