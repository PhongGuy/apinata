//! The branch picker dialog over a clone of a local bare repository.

use crate::git_support::{Sandbox, path_with, write};
use crate::support::*;

use apinata_app::branch_dialog;
use apinata_app::branch_picker::{Activity, Notice};
use apinata_app::changes_dialog;
use apinata_app::session::Session;
use apinata_app::workspace::Workspace;
use gpui_kit::AppContext as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext};

struct Rig {
    dir: tempfile::TempDir,
    ws: Entity<Workspace>,
    win: gpui_kit::AnyWindowHandle,
}

/// A clone of the sandbox origin, opened as a collection with git on, in a test window;
/// `prep` runs after the origin exists and before the clone.
fn rig(cx: &mut TestAppContext, sb: &Sandbox, prep: impl FnOnce(&Sandbox)) -> Rig {
    crate::support::init(cx);
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let bare = sb.origin();
    prep(sb);
    sb.git(
        sb.path(),
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            dir.path().to_str().unwrap(),
        ],
    );
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    std::mem::forget(state);
    let ws = workspace(cx, &session);
    let options = sb.options();
    ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    let root = dir.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    wait_for(cx, "the first git status", |cx| {
        ws.read_with(cx, |w, _| w.git_changes(&root).is_some())
    });
    let (win, _) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Blank)
            })
        })
        .unwrap();
    Rig { dir, ws, win }
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

fn open_dialog(cx: &mut TestAppContext, r: &Rig) {
    let (ws, root) = (r.ws.clone(), r.dir.path().to_path_buf());
    cx.update_window(r.win, |_, w, cx| {
        branch_dialog::open(&ws, root, w, cx);
    })
    .unwrap();
    wait_for(cx, "the branch list", |cx| {
        cx.update(|cx| branch_dialog::current(cx))
            .is_some_and(|p| p.read_with(cx, |p, _| !p.branches().is_empty()))
    });
    frame(cx, r.win);
}

/// The row of local branch `name`, else the remote-only row of `origin/<name>`.
fn row_of(cx: &mut TestAppContext, name: &str) -> usize {
    let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
    picker.read_with(cx, |p, _| {
        let rows = p.branches();
        rows.iter()
            .position(|b| b.local.as_deref() == Some(name))
            .or_else(|| {
                rows.iter().position(|b| {
                    b.local.is_none() && b.upstream.as_ref().is_some_and(|u| u.name == name)
                })
            })
            .expect("listed")
    })
}

fn label_of(cx: &mut TestAppContext, r: &Rig, id: impl Into<gpui_kit::ElementId>) -> String {
    let id = id.into();
    cx.update_window(r.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
    .unwrap_or_default()
}

/// Writes an untracked file into the clone and waits for the status to count it.
fn dirty(cx: &mut TestAppContext, r: &Rig) {
    write(r.dir.path(), "scratch.txt", "mine\n");
    let root = r.dir.path().to_path_buf();
    r.ws.update(cx, |w, cx| w.refresh_all_git(cx));
    wait_for(cx, "the status to see the file", |cx| {
        r.ws.read_with(cx, |w, _| {
            w.git_changes(&root).is_some_and(|g| !g.files.is_empty())
        })
    });
}

fn click(cx: &mut TestAppContext, r: &Rig, id: impl Into<gpui_kit::ElementId>) {
    cx.update_window(r.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, r.win);
}

fn dialog_open(cx: &mut TestAppContext, r: &Rig) -> bool {
    cx.update_window(r.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

#[gpui_kit::test]
fn picking_a_branch_switches_and_closes_the_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    open_dialog(cx, &r);
    assert!(present(cx, r.win, "branch-dialog"));
    let ix = row_of(cx, "feature");
    click(cx, &r, ("branch-row", ix));
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &r));
    assert!(r.dir.path().join("b.txt").is_file());
    assert_eq!(
        sb.git(r.dir.path(), &["branch", "--show-current"]).trim(),
        "feature"
    );
}

#[gpui_kit::test]
fn a_dirty_tree_shows_the_refusal_and_a_button_to_the_changes_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    dirty(cx, &r);
    open_dialog(cx, &r);
    let ix = row_of(cx, "feature");
    click(cx, &r, ("branch-row", ix));
    wait_for(cx, "the refusal", |cx| {
        present(cx, r.win, "branch-open-changes")
    });
    assert!(present(cx, r.win, "branch-notice"));
    click(cx, &r, "branch-open-changes");
    assert!(cx.update(|cx| changes_dialog::current(cx).is_some()));
    assert_eq!(
        std::fs::read_to_string(r.dir.path().join("scratch.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        sb.git(r.dir.path(), &["branch", "--show-current"]).trim(),
        "main"
    );
}

#[gpui_kit::test]
fn fetch_lists_a_branch_pushed_after_the_clone(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "-c", "later", "main"]);
    write(&seed, "later.txt", "x\n");
    sb.commit_all(&seed, "later");
    sb.git(&seed, &["push", "-q", "origin", "later"]);
    open_dialog(cx, &r);
    click(cx, &r, "branch-fetch");
    wait_for(cx, "later to be listed", |cx| {
        let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
        picker.read_with(cx, |p, _| {
            p.branches()
                .iter()
                .any(|b| b.upstream.as_ref().is_some_and(|u| u.name == "later"))
        })
    });
}

#[gpui_kit::test]
fn the_dialog_takes_no_input_while_switching(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    // A git held on the checkout keeps the picker in Switching until the test releases it.
    let bin = latched_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    r.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    open_dialog(cx, &r);
    let ix = row_of(cx, "feature");
    click(cx, &r, ("branch-row", ix));
    let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
    assert_eq!(
        picker.read_with(cx, |p, _| p.activity()),
        Activity::Switching
    );
    // Fetch, the other rows and Close are inert while the switch runs.
    click(cx, &r, "branch-fetch");
    click(cx, &r, ("branch-row", ix));
    click(cx, &r, "branch-close");
    assert!(dialog_open(cx, &r));
    cx.update_window(r.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, r.win);
    assert!(dialog_open(cx, &r), "Escape is inert while switching");
    assert_eq!(
        picker.read_with(cx, |p, _| p.activity()),
        Activity::Switching
    );
    release_git(&bin);
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &r));
    assert_eq!(
        sb.git(r.dir.path(), &["branch", "--show-current"]).trim(),
        "feature"
    );
}

#[gpui_kit::test]
fn closing_the_window_mid_switch_leaves_a_consistent_session(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    let bin = latched_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    r.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    open_dialog(cx, &r);
    let ix = row_of(cx, "feature");
    click(cx, &r, ("branch-row", ix));
    let root = r.dir.path().to_path_buf();
    assert!(r.ws.read_with(cx, |w, _| w.is_switching(&root)));
    wait_for(cx, "git to reach the checkout", |_| {
        bin.join("reached").is_file()
    });
    // The window goes while git is mid-checkout; the dialog and its model go with it.
    cx.update_window(r.win, |_, w, _| w.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(cx.update(|cx| branch_dialog::current(cx)).is_none());
    assert!(!r.ws.read_with(cx, |w, _| w.is_switching(&root)));
    release_git(&bin);
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert!(!r.ws.read_with(cx, |w, _| w.is_switching(&root)));
    assert!(!r.ws.read_with(cx, |w, _| w.collections().is_empty()));
    let edit = r.ws.update(cx, |w, cx| {
        w.send(
            &root,
            apinata_app::command::Command::Edit {
                target: apinata_app::command::Target::Collection,
                edit: apinata_app::command::Edit::Docs("later".into()),
            },
            cx,
        )
    });
    assert!(edit.is_some(), "edits work again once the lock is gone");
}

#[gpui_kit::test]
fn enter_on_a_focused_branch_row_switches_to_it(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    cx.update(apinata_app::keymap::init);
    open_dialog(cx, &r);
    let ix = row_of(cx, "feature");
    enter_on(cx, r.win, ("branch-row", ix));
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &r));
    assert_eq!(
        sb.git(r.dir.path(), &["branch", "--show-current"]).trim(),
        "feature",
        "Enter pressed the row"
    );
}

#[gpui_kit::test]
fn enter_while_switching_keeps_the_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    let bin = latched_git(&sb);
    let options = sb.options().env("PATH", path_with(&bin));
    r.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    open_dialog(cx, &r);
    let ix = row_of(cx, "feature");
    click(cx, &r, ("branch-row", ix));
    let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
    assert_eq!(
        picker.read_with(cx, |p, _| p.activity()),
        Activity::Switching
    );
    cx.update_window(r.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, r.win);
    assert!(dialog_open(cx, &r), "Enter is inert while switching");
    release_git(&bin);
    wait_for(cx, "the dialog to close", |cx| !dialog_open(cx, &r));
}

#[gpui_kit::test]
fn the_rows_leave_room_for_the_focus_ring_inside_the_dialog_body(cx: &mut TestAppContext) {
    use apinata_app::style::FOCUS_RING_ROOM;
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    open_dialog(cx, &r);
    cx.update_window(r.win, |_, w, cx| {
        w.render_frame(cx);
        // The kit's dialog body clips with no top padding, so the view's own box is the clip.
        let body = w.find("branch-dialog").bounds();
        let row = w.find(("branch-row", 0usize)).bounds();
        assert!(
            row.top() - FOCUS_RING_ROOM >= body.top(),
            "{row:?} in {body:?}"
        );
        assert!(
            row.left() - FOCUS_RING_ROOM >= body.left(),
            "{row:?} in {body:?}"
        );
        assert!(
            row.right() + FOCUS_RING_ROOM <= body.right(),
            "{row:?} in {body:?}"
        );
    })
    .unwrap();
}

// The footer is the body's last row and the kit body clips flush with its bottom, so the
// footer buttons' rings need room below them inside the view.
#[gpui_kit::test]
fn the_footer_buttons_keep_their_ring_inside_the_dialog_body(cx: &mut TestAppContext) {
    use apinata_app::style::FOCUS_RING_ROOM;
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    open_dialog(cx, &r);
    cx.update_window(r.win, |_, w, cx| {
        w.render_frame(cx);
        let body = w.find("branch-dialog").bounds();
        for id in ["branch-fetch", "branch-close"] {
            let b = w.find(id).bounds();
            assert!(
                b.bottom() + FOCUS_RING_ROOM <= body.bottom(),
                "{id} {b:?} in {body:?}"
            );
            assert!(
                b.top() - FOCUS_RING_ROOM >= body.top(),
                "{id} {b:?} in {body:?}"
            );
        }
    })
    .unwrap();
}

// A click on Fetch leaves Enter to the focused control: it never confirms the list away.
#[gpui_kit::test]
fn enter_after_clicking_fetch_keeps_the_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    cx.update(apinata_app::keymap::init);
    open_dialog(cx, &r);
    click(cx, &r, "branch-fetch");
    wait_for(cx, "the fetch to end", |cx| {
        let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
        picker.read_with(cx, |p, _| p.activity() == Activity::Idle)
    });
    frame(cx, r.win);
    cx.update_window(r.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, r.win);
    assert!(dialog_open(cx, &r), "Enter after a click keeps the dialog");
    cx.update_window(r.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    frame(cx, r.win);
    assert!(!dialog_open(cx, &r), "Escape closes");
}

#[gpui_kit::test]
fn the_changes_button_waits_for_a_dirty_refusal(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    dirty(cx, &r);
    open_dialog(cx, &r);
    assert!(!present(cx, r.win, "branch-open-changes"), "no refusal yet");
    let ix = row_of(cx, "feature");
    click(cx, &r, ("branch-row", ix));
    wait_for(cx, "the refusal", |cx| {
        present(cx, r.win, "branch-open-changes")
    });
    assert_eq!(label_of(cx, &r, "branch-open-changes"), "Open changes");
}

#[gpui_kit::test]
fn a_clean_tree_offers_no_changes_button(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    open_dialog(cx, &r);
    assert!(!present(cx, r.win, "branch-open-changes"));
}

#[gpui_kit::test]
fn a_checked_out_local_only_branch_is_marked_and_locked(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    sb.git(r.dir.path(), &["switch", "-q", "-c", "topic"]);
    open_dialog(cx, &r);
    let ix = row_of(cx, "topic");
    assert_eq!(label_of(cx, &r, ("branch-row", ix)), "topic (checked out)");
    let locked = cx
        .update_window(r.win, |_, w, _| {
            w.within(("branch-row", ix)).try_find("locked").is_some()
        })
        .unwrap();
    assert!(locked, "the checked-out row is locked");
    let main = row_of(cx, "main");
    assert_eq!(label_of(cx, &r, ("branch-row", main)), "main");
}

#[gpui_kit::test]
fn a_local_row_names_its_upstream_only_when_the_names_differ(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    sb.git(
        r.dir.path(),
        &["branch", "--track", "work", "origin/feature"],
    );
    open_dialog(cx, &r);
    let work = row_of(cx, "work");
    assert_eq!(label_of(cx, &r, ("branch-row", work)), "work");
    assert_eq!(
        label_of(cx, &r, ("branch-upstream", work)),
        "origin/feature"
    );
    let main = row_of(cx, "main");
    assert!(!present(cx, r.win, ("branch-upstream", main)));
    // origin/feature is shown through `work`, so it has no row of its own.
    let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
    let rows = picker.read_with(cx, |p, _| p.branches().len());
    assert_eq!(rows, 2);
}

/// Leaves an unsaved in-app edit on the collection, which the picker refuses to switch over.
fn unsaved_edit(cx: &mut TestAppContext, r: &Rig) {
    use apinata_app::command::{Command, Edit, Target};
    let root = r.dir.path().to_path_buf();
    r.ws.update(cx, |w, cx| {
        w.send(
            &root,
            Command::Edit {
                target: Target::Collection,
                edit: Edit::Docs("draft".into()),
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert!(
        !r.ws
            .read_with(cx, |w, cx| w.unsaved(Some(&root), cx))
            .is_empty()
    );
}

#[gpui_kit::test]
fn an_unsaved_edit_shows_its_refusal_in_the_dialog_and_keeps_the_branch(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let r = rig(cx, &sb, |_| {});
    unsaved_edit(cx, &r);
    open_dialog(cx, &r);
    let ix = row_of(cx, "feature");
    click(cx, &r, ("branch-row", ix));
    wait_for(cx, "the refusal", |cx| present(cx, r.win, "branch-notice"));
    let picker = cx.update(|cx| branch_dialog::current(cx)).expect("open");
    assert_eq!(
        picker.read_with(cx, |p, _| p.notice().cloned()),
        Some(Notice::Unsaved)
    );
    assert!(!present(cx, r.win, "branch-open-changes"));
    assert!(dialog_open(cx, &r));
    assert_eq!(
        sb.git(r.dir.path(), &["branch", "--show-current"]).trim(),
        "main"
    );
}
