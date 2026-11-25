//! Discarding a file's changes from the Changes dialog: the confirm says what goes where, the
//! version on disk goes to a fake trash, and the Undo toast brings it back.

use crate::git_support;
use crate::h;
use crate::support;

use std::sync::Arc;

use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use h::*;
use support::fake_trash::FakeTrash;
use support::*;

// The confirm dialog's title and detail.
fn confirm_text(cx: &mut TestAppContext) -> (String, String) {
    let win = confirm_window(cx).expect("a confirm dialog is open");
    cx.update_window(win, |_, w, _| {
        let text = |id: &'static str| {
            w.try_find(id)
                .and_then(|e| e.label().map(str::to_owned))
                .unwrap_or_default()
        };
        (text("confirm-dialog"), text("confirm-detail"))
    })
    .unwrap()
}

fn with_trash(cx: &mut TestAppContext, s: &Started, bin: FakeTrash) -> Arc<FakeTrash> {
    let bin = Arc::new(bin);
    let shared = Arc::clone(&bin);
    s.ws.update(cx, |w, _| w.set_trash(shared));
    bin
}

fn row_of(cx: &mut TestAppContext, rel: &str) -> usize {
    view(cx)
        .read_with(cx, |v, _| {
            v.files()
                .iter()
                .position(|(p, _)| p == std::path::Path::new(rel))
        })
        .expect("listed")
}

fn ask_discard(cx: &mut TestAppContext, s: &Started, rel: &str) {
    let ix = row_of(cx, rel);
    let v = view(cx);
    cx.update_window(s.win, |_, w, cx| v.update(cx, |v, cx| v.discard(ix, w, cx)))
        .unwrap();
    frame(cx, s.win);
}

fn has(cx: &mut TestAppContext, s: &Started, id: &'static str) -> bool {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

#[gpui_kit::test]
fn a_discard_names_the_file_trashes_it_and_undo_brings_it_back(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let mine = std::fs::read(col.path().join("a.yml")).unwrap();
    let s = git_started(cx, &sb, &col);
    let bin = with_trash(cx, &s, FakeTrash::new());
    open_on(cx, &s, col.path());
    ask_discard(cx, &s, "a.yml");
    let (title, detail) = confirm_text(cx);
    assert!(title.contains("a.yml"), "{title}");
    assert!(
        detail.contains("Your version goes to the trash"),
        "{detail}"
    );
    answer_confirm(cx, "confirm-discard-file");
    idle(cx, &s);
    wait_for(cx, "the commit's version", |_| {
        std::fs::read(col.path().join("a.yml")).unwrap() != mine
    });
    assert_eq!(bin.len(), 1);
    assert_eq!(std::fs::read(bin.stash(1)).unwrap(), mine);
    assert!(has(cx, &s, "undo-delete"), "an Undo toast shows");

    cx.update_window(s.win, |_, w, cx| w.click("undo-delete", cx))
        .unwrap();
    wait_for(cx, "the version back", |_| {
        std::fs::read(col.path().join("a.yml")).ok().as_deref() == Some(mine.as_slice())
    });
    assert_eq!(bin.len(), 0);
}

#[gpui_kit::test]
fn without_a_trash_the_confirm_says_it_cannot_be_undone_and_cancel_changes_nothing(
    cx: &mut TestAppContext,
) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let mine = std::fs::read(col.path().join("a.yml")).unwrap();
    let s = git_started(cx, &sb, &col);
    with_trash(cx, &s, FakeTrash::broken());
    open_on(cx, &s, col.path());
    ask_discard(cx, &s, "a.yml");
    let (_, detail) = confirm_text(cx);
    assert!(detail.contains("This cannot be undone"), "{detail}");
    answer_confirm(cx, "confirm-cancel");
    idle(cx, &s);
    assert_eq!(std::fs::read(col.path().join("a.yml")).unwrap(), mine);
    assert!(
        sb.git(col.path(), &["diff", "--name-only"])
            .contains("a.yml")
    );
}

#[gpui_kit::test]
fn without_a_trash_an_untracked_file_is_refused_without_asking(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    with_trash(cx, &s, FakeTrash::broken());
    open_on(cx, &s, col.path());
    ask_discard(cx, &s, "new.yml");
    assert!(!confirm_open(cx));
    assert!(col.path().join("new.yml").exists());
    let notice = view(cx).read_with(cx, |v, _| v.notice().cloned());
    assert!(
        matches!(&notice, Some(apinata_app::changes_dialog::Notice::Failed(t)) if t.contains("no trash")),
        "{notice:?}"
    );
}

#[gpui_kit::test]
fn a_file_with_unsaved_edits_in_a_tab_is_refused(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let mine = std::fs::read(col.path().join("a.yml")).unwrap();
    let s = git_started(cx, &sb, &col);
    let bin = with_trash(cx, &s, FakeTrash::new());
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/unsaved"), cx)
    });
    cx.run_until_parked();
    open_on(cx, &s, col.path());
    ask_discard(cx, &s, "a.yml");
    assert!(!confirm_open(cx), "it refuses rather than asks");
    assert_eq!(std::fs::read(col.path().join("a.yml")).unwrap(), mine);
    assert_eq!(bin.len(), 0);
    let notice = view(cx).read_with(cx, |v, _| v.notice().cloned());
    assert!(
        matches!(&notice, Some(apinata_app::changes_dialog::Notice::Failed(t)) if t.contains("unsaved")),
        "{notice:?}"
    );
}

#[gpui_kit::test]
fn the_confirm_names_a_differing_staged_version_and_secrets(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = committed(
        &sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), ".env", "API_KEY=staged\n");
    sb.git(col.path(), &["add", "-f", ".env"]);
    sb.commit_all(col.path(), "env");
    write(col.path(), ".env", "API_KEY=staged2\n");
    sb.git(col.path(), &["add", "-f", ".env"]);
    write(col.path(), ".env", "API_KEY=mine\n");
    let s = git_started(cx, &sb, &col);
    with_trash(cx, &s, FakeTrash::new());
    open_on(cx, &s, col.path());
    ask_discard(cx, &s, ".env");
    let (title, detail) = confirm_text(cx);
    assert!(title.contains(".env"), "{title}");
    assert!(detail.contains("staged version"), "{detail}");
    assert!(detail.contains("secret"), "{detail}");
    answer_confirm(cx, "confirm-cancel");
}

#[gpui_kit::test]
fn the_row_menu_and_the_button_both_offer_discard(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    with_trash(cx, &s, FakeTrash::new());
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-file", 0usize));
    frame(cx, s.win);
    click(cx, &s, "changes-discard");
    assert!(confirm_open(cx), "the button asks");
    answer_confirm(cx, "confirm-cancel");
    cx.update_window(s.win, |_, w, cx| {
        w.right_click(("changes-file", 0usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    click_menu(cx, s.win, "Discard Changes\u{2026}");
    assert!(confirm_open(cx), "the row menu asks");
    answer_confirm(cx, "confirm-cancel");
}

#[gpui_kit::test]
fn delete_in_the_list_asks_to_discard_the_selected_row(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    with_trash(cx, &s, FakeTrash::new());
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-file", 0usize));
    frame(cx, s.win);
    platform_press(cx, s.win, "delete");
    assert!(confirm_open(cx), "Delete asks");
    answer_confirm(cx, "confirm-cancel");
}
