//! One walkthrough of the whole stage and commit flow in a 1000x500 window.

use crate::git_support;
use crate::h;

use apinata_app::changes_dialog::Notice;
use apinata_app::git_status::GitStage::{Staged, Unstaged};
use git_support::Sandbox;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};
use h::*;

#[cfg(unix)]
#[gpui_kit::test]
fn stage_unstage_fail_then_commit_in_one_small_window(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let hook_path = col.path().join(".git/hooks/pre-commit");
    h::hook(
        col.path(),
        "pre-commit",
        "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n",
    );
    let s = git_started(cx, &sb, &col);
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(500.)));
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    assert_eq!(stages(cx), [Staged, Staged]);
    click(cx, &s, ("changes-stage", 1usize));
    idle(cx, &s);
    assert_eq!(stages(cx), [Staged, Unstaged]);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::changes_dialog::current(cx)
            .expect("the dialog is open")
            .update(cx, |v, cx| v.set_message("Edit a", w, cx))
    })
    .unwrap();
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    let failed = h::view(cx).read_with(cx, |v, _| v.notice().cloned());
    assert!(matches!(failed, Some(Notice::Failed(t)) if t.contains("lint failed")));
    std::fs::remove_file(&hook_path).unwrap();
    click(cx, &s, "changes-commit");
    idle(cx, &s);
    let done = h::view(cx).read_with(cx, |v, _| v.notice().cloned());
    assert!(matches!(done, Some(Notice::Committed(_))));
    assert_eq!(
        sb.git(col.path(), &["log", "-1", "--name-only", "--format=%s"])
            .trim(),
        "Edit a\n\na.yml"
    );
    cx.update_window(s.win, |_, w, _| {
        let b = w.find("changes-commit").bounds();
        assert!(b.bottom() <= px(500.), "{b:?}");
    })
    .unwrap();
}
