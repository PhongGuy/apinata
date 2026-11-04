//! A commit message typed in Changes survives every close as a per-collection draft until a
//! commit lands.

use crate::git_support;
use crate::h;
use crate::support;

use git_support::Sandbox;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, point, px};
use h::*;
use support::*;

// The ways the dialog is closed without committing.
#[derive(Clone, Copy, Debug)]
enum Close {
    Button,
    Escape,
    Backdrop,
}

fn close(cx: &mut TestAppContext, win: AnyWindowHandle, how: Close) {
    match how {
        Close::Button => cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.click("changes-close", cx)
            })
            .unwrap(),
        Close::Escape => cx
            .update_window(win, |_, w, cx| w.press("escape", cx))
            .unwrap(),
        // The bottom left corner of the dialog's full-window layer is backdrop.
        Close::Backdrop => cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                let layer = w.find("dialog").bounds();
                w.click_at("dialog", point(px(4.), layer.size.height - px(4.)), cx);
            })
            .unwrap(),
    }
    cx.run_until_parked();
    assert!(
        cx.update(|cx| apinata_app::changes_dialog::current(cx))
            .is_none(),
        "{how:?} closes the dialog"
    );
}

fn type_message(cx: &mut TestAppContext, s: &Started, text: &str) {
    type_at_end(cx, s.win, None, "changes-message", text);
    frame(cx, s.win);
}

// The message as drawn in the first frame after opening.
fn message(cx: &mut TestAppContext) -> String {
    view(cx).read_with(cx, |v, cx| v.message(cx))
}

fn commits(sb: &Sandbox, root: &std::path::Path) -> String {
    sb.git(root, &["rev-list", "--count", "HEAD"])
        .trim()
        .to_owned()
}

#[gpui_kit::test]
fn a_typed_message_survives_every_close(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    for how in [Close::Escape, Close::Button, Close::Backdrop] {
        open_on(cx, &s, col.path());
        type_message(cx, &s, "x");
        close(cx, s.win, how);
        open_on(cx, &s, col.path());
        assert!(message(cx).ends_with('x'), "{how:?}: {:?}", message(cx));
        close(cx, s.win, Close::Escape);
    }
    open_on(cx, &s, col.path());
    assert_eq!(message(cx), "xxx");
}

#[gpui_kit::test]
fn a_landed_commit_clears_the_draft(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    type_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    assert_eq!(commits(&sb, col.path()), "2");
    close(cx, s.win, Close::Escape);
    open_on(cx, &s, col.path());
    assert_eq!(message(cx), "");
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_commit_keeps_the_draft(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    h::hook(col.path(), "pre-commit", "#!/bin/sh\nexit 1\n");
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    type_message(cx, &s, "Edit a");
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    assert_eq!(commits(&sb, col.path()), "1");
    close(cx, s.win, Close::Escape);
    open_on(cx, &s, col.path());
    assert_eq!(message(cx), "Edit a");
}

// The hook holds the commit until the test creates the gate file, so the dialog can close first.
#[cfg(unix)]
#[gpui_kit::test]
fn a_commit_that_lands_after_close_clears_the_draft(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let sb = Sandbox::new();
    let col = changed(&sb);
    let gate = sb.path().join("gate");
    let body = format!(
        "#!/bin/sh\ni=0\nwhile [ ! -f '{}' ] && [ $i -lt 300 ]; do sleep 0.05; i=$((i+1)); done\n",
        gate.display()
    );
    h::hook(col.path(), "pre-commit", &body);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    type_message(cx, &s, "Edit a");
    // No park until the gate opens: parking waits for the held commit.
    cx.update_window(s.win, |_, w, cx| {
        w.click("changes-commit", cx);
        w.press("escape", cx);
    })
    .unwrap();
    assert!(
        cx.update(|cx| apinata_app::changes_dialog::current(cx))
            .is_none(),
        "the dialog closed while the commit was held"
    );
    assert_eq!(commits(&sb, col.path()), "1");
    std::fs::write(&gate, "").unwrap();
    wait_for(cx, "the commit", |_| commits(&sb, col.path()) == "2");
    settle(cx);
    open_on(cx, &s, col.path());
    assert_eq!(message(cx), "");
}

#[gpui_kit::test]
fn a_draft_belongs_to_its_collection(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let a = changed(&sb);
    let b = changed(&sb);
    let s = git_started(cx, &sb, &a);
    let root_b = b.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.open(root_b.clone(), cx));
    wait_for(cx, "git state for b", |cx| {
        s.ws.read_with(cx, |w, _| w.git_changes(&root_b).is_some())
    });
    open_on(cx, &s, a.path());
    type_message(cx, &s, "For a");
    close(cx, s.win, Close::Escape);
    open_on(cx, &s, &root_b);
    assert_eq!(message(cx), "");
    close(cx, s.win, Close::Escape);
    open_on(cx, &s, a.path());
    assert_eq!(message(cx), "For a");
}

#[gpui_kit::test]
fn typing_after_reopen_appends_to_the_draft(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    let v = view(cx);
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| v.set_message("one\ntwo", w, cx))
    })
    .unwrap();
    drop(v);
    close(cx, s.win, Close::Escape);
    open_on(cx, &s, col.path());
    // Focus moves into the field without a click, so the caret is where the open left it.
    let v = view(cx);
    cx.update_window(s.win, |_, w, cx| {
        v.update(cx, |v, cx| v.focus_message(w, cx));
        w.input("!", cx);
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(message(cx), "one\ntwo!");
}

// Reopening with a kept draft still focuses the file list, as an open without one does: files
// changed since, and Tab reaches the message with its caret after the draft.
#[gpui_kit::test]
fn a_kept_draft_leaves_focus_on_the_file_list(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    type_message(cx, &s, "Edit a");
    close(cx, s.win, Close::Escape);
    open_on(cx, &s, col.path());
    assert_eq!(message(cx), "Edit a");
    assert!(
        holds_focus(cx, s.win, "changes-files"),
        "the list holds focus"
    );
    assert!(!holds_focus(cx, s.win, "changes-message"));
}
