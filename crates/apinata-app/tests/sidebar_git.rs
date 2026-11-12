//! Git letters on sidebar rows, named in words, and the row menu's Show changes entry.

use crate::git_support;
use crate::support;

use std::path::Path;

use apinata_app::changes_dialog::current;
use apinata_app::session::Session;
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use apinata_app::sidebar::{Sidebar, marker_id};
use apinata_app::workspace::Workspace;
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext, WindowOptions,
};
use support::*;

fn committed(sb: &Sandbox, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = write_tree(files);
    sb.git(dir.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(dir.path(), "init");
    dir
}

fn sidebar_refs<'a>(files: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    files.iter().map(|(p, t)| (*p, t.as_str())).collect()
}

struct GitSidebar {
    sb: Sandbox,
    _state: tempfile::TempDir,
    _col: tempfile::TempDir,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ws: Entity<Workspace>,
}

/// The sidebar files, committed when `repo` (else a plain directory that git cannot place in
/// any repository), changed by `change`, opened with git on.
fn git_sidebar(cx: &mut TestAppContext, repo: bool, change: impl FnOnce(&Path)) -> GitSidebar {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let sb = Sandbox::new();
    let files = sidebar_files();
    let col = if repo {
        committed(&sb, &sidebar_refs(&files))
    } else {
        write_tree(&sidebar_refs(&files))
    };
    change(col.path());
    let mut options = sb.options();
    if !repo {
        options = options.env("GIT_CEILING_DIRECTORIES", col.path().parent().unwrap());
    }
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    let root = col.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Sidebar::new(shared, window, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    GitSidebar {
        sb,
        _state: state,
        _col: col,
        root,
        win,
        ws,
    }
}

fn label(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> Option<String> {
    let id = id.into();
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn git_marker(path: &str) -> gpui_kit::SharedString {
    marker_id("git", &row(path))
}

#[test]
fn show_changes_is_named_in_words() {
    assert_eq!(MenuEntry::ShowChanges.label(), "Show changes");
    assert!(!menu_entries(NodeKind::Request).contains(&MenuEntry::ShowChanges));
}

#[gpui_kit::test]
fn changed_rows_carry_a_letter_named_in_words(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(root, "health.yml", &request("Health", "http://x/changed"));
        write(root, "users/folder.yml", &folder("People"));
        write(root, "new.yml", &request("New", "http://x/new"));
    });
    assert_eq!(
        label(cx, g.win, git_marker("health.yml")).as_deref(),
        Some("Modified since last commit")
    );
    assert_eq!(
        label(cx, g.win, git_marker("users")).as_deref(),
        Some("Modified since last commit")
    );
    assert_eq!(
        label(cx, g.win, git_marker("new.yml")).as_deref(),
        Some("Untracked")
    );
    assert!(!shown(cx, g.win, git_marker("status.yml")));
}

fn rollup_marker(path: &str) -> gpui_kit::SharedString {
    marker_id("rollup", &row(path))
}

#[gpui_kit::test]
fn a_collapsed_folder_rolls_up_a_changed_request_inside(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(
            root,
            "users/list.yml",
            &request("List users", "http://x/changed"),
        )
    });
    assert_eq!(
        label(cx, g.win, rollup_marker("users")).as_deref(),
        Some("Changes inside")
    );
    // The folder's own file is unchanged, so it carries no letter of its own.
    assert!(!shown(cx, g.win, git_marker("users")));
}

#[gpui_kit::test]
fn an_expanded_folder_marks_the_request_only(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(
            root,
            "users/list.yml",
            &request("List users", "http://x/changed"),
        )
    });
    cx.update_window(g.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    assert!(shown(cx, g.win, row("users/list.yml")));
    assert_eq!(
        label(cx, g.win, git_marker("users/list.yml")).as_deref(),
        Some("Modified since last commit")
    );
    assert!(!shown(cx, g.win, git_marker("users")));
    assert!(!shown(cx, g.win, rollup_marker("users")));
    // Collapsing again brings the roll-up back.
    cx.update_window(g.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    assert!(shown(cx, g.win, rollup_marker("users")));
}

// A selected folder shows its run button; the roll-up mark keeps its own room beside it.
#[gpui_kit::test]
fn the_rollup_mark_does_not_sit_under_the_run_button(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(
            root,
            "users/list.yml",
            &request("List users", "http://x/changed"),
        )
    });
    cx.update_window(g.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    cx.update_window(g.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    frame(cx, g.win);
    let (dot, run) = cx
        .update_window(g.win, |_, w, _| {
            (
                w.find(rollup_marker("users")).bounds(),
                w.find(marker_id("run-folder", &row("users"))).bounds(),
            )
        })
        .unwrap();
    assert!(dot.right() <= run.left(), "dot {dot:?}, run {run:?}");
}

#[gpui_kit::test]
fn a_clean_collapsed_folder_has_no_rollup(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(root, "health.yml", &request("Health", "http://x/changed"))
    });
    assert!(!shown(cx, g.win, rollup_marker("users")));
}

#[gpui_kit::test]
fn a_rollup_clears_with_a_commit(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(
            root,
            "users/list.yml",
            &request("List users", "http://x/changed"),
        )
    });
    assert!(shown(cx, g.win, rollup_marker("users")));
    g.sb.commit_all(&g.root, "save");
    g.ws.update(cx, |w, cx| w.refresh_git(&g.root, cx));
    assert!(!shown(cx, g.win, rollup_marker("users")));
}

#[gpui_kit::test]
fn a_collapsed_folder_rolls_up_an_unsaved_request_inside(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |_| {});
    assert!(!shown(cx, g.win, rollup_marker("users")));
    g.ws.update(cx, |w, cx| {
        w.send(&g.root, edit_url("users/list.yml", "http://x/draft"), cx)
    });
    cx.run_until_parked();
    assert!(shown(cx, g.win, rollup_marker("users")));
}

#[gpui_kit::test]
fn a_collapsed_collection_rolls_up_a_change_inside(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(root, "health.yml", &request("Health", "http://x/changed"))
    });
    assert!(!shown(cx, g.win, rollup_marker("")));
    cx.update_window(g.win, |_, w, cx| w.click(row(""), cx))
        .unwrap();
    assert!(!shown(cx, g.win, row("health.yml")), "the click collapsed");
    assert!(shown(cx, g.win, rollup_marker("")));
}

#[gpui_kit::test]
fn a_commit_clears_the_letter_on_the_next_refresh(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(root, "health.yml", &request("Health", "http://x/changed"))
    });
    assert!(shown(cx, g.win, git_marker("health.yml")));
    g.sb.commit_all(&g.root, "save");
    g.ws.update(cx, |w, cx| w.refresh_git(&g.root, cx));
    assert!(!shown(cx, g.win, git_marker("health.yml")));
}

#[gpui_kit::test]
fn outside_a_repository_no_row_is_marked(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, false, |root| {
        write(root, "health.yml", &request("Health", "http://x/changed"))
    });
    assert!(!shown(cx, g.win, git_marker("health.yml")));
    assert!(!shown(cx, g.win, git_marker("status.yml")));
}

#[gpui_kit::test]
fn without_git_no_row_is_marked(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    std::fs::write(
        h.root.join("health.yml"),
        request("Health", "http://x/changed"),
    )
    .unwrap();
    assert!(!shown(cx, h.win, git_marker("health.yml")));
}

/// The real app over the sidebar files with `health.yml` changed and git on.
fn started_changed(cx: &mut TestAppContext, sb: &Sandbox) -> (tempfile::TempDir, Started) {
    let files = sidebar_files();
    let col = committed(sb, &sidebar_refs(&files));
    write(
        col.path(),
        "health.yml",
        &request("Health", "http://x/changed"),
    );
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    (col, s)
}

#[gpui_kit::test]
fn show_changes_opens_the_dialog_on_the_row(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = started_changed(cx, &sb);
    cx.update_window(s.win, |_, w, cx| w.right_click(row("health.yml"), cx))
        .unwrap();
    frame(cx, s.win);
    click_menu(cx, s.win, "Show changes");
    frame(cx, s.win);
    let selected = cx.update(|cx| {
        current(cx).and_then(|v| {
            v.read(cx)
                .selected()
                .map(|p| p.to_string_lossy().into_owned())
        })
    });
    assert_eq!(selected.as_deref(), Some("health.yml"));
}

#[gpui_kit::test]
fn an_unchanged_row_offers_no_show_changes(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = started_changed(cx, &sb);
    cx.update_window(s.win, |_, w, cx| w.right_click(row("status.yml"), cx))
        .unwrap();
    frame(cx, s.win);
    let (last, extra) = cx
        .update_window(s.win, |_, w, _| {
            (
                try_menu_index(w, "Delete").is_some(),
                try_menu_index(w, "Show changes").is_some(),
            )
        })
        .unwrap();
    assert!(last, "the menu is open");
    assert!(!extra);
}

// A selected row's edge overlay takes the kit's suffix slot, so the dot and the letter still
// render inside the row, on its centre line, the letter before the dot as on the tabs.
#[gpui_kit::test]
fn a_selected_dirty_row_keeps_its_dot_and_letter_aligned(cx: &mut TestAppContext) {
    let g = git_sidebar(cx, true, |root| {
        write(root, "health.yml", &request("Health", "http://x/changed"));
    });
    g.ws.update(cx, |w, cx| {
        w.send(&g.root, edit_url("health.yml", "http://x/draft"), cx)
    });
    cx.run_until_parked();
    let id = row("health.yml");
    cx.update_window(g.win, |_, w, cx| w.click(id.clone(), cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, g.win);
    let (row_box, dot, letter) = cx
        .update_window(g.win, |_, w, _| {
            (
                w.find(id.clone()).bounds(),
                w.find(marker_id("dirty", &id)).bounds(),
                w.find(git_marker("health.yml")).bounds(),
            )
        })
        .unwrap();
    assert!(
        letter.right() <= dot.left(),
        "letter {letter:?}, dot {dot:?}"
    );
    assert!(dot.right() <= row_box.right(), "dot {dot:?} in {row_box:?}");
    let (a, b) = (dot.center().y, letter.center().y);
    assert!((a - b).abs() <= gpui_kit::px(1.), "dot {a:?}, letter {b:?}");
    assert!((row_box.center().y - b).abs() <= gpui_kit::px(1.));
}
