//! The Changes dialog's stage controls: checkbox rows, the summary, stage all, the busy guard
//! and keyboard.

use crate::git_support;
use crate::h;
use crate::support;

use apinata_app::git_status::GitStage::{Both, Staged, Unstaged};
use git_support::{Sandbox, write};
use gpui_kit::{AppContext as _, TestAppContext};
use h::*;
use support::*;

fn cached(sb: &Sandbox, col: &tempfile::TempDir) -> String {
    sb.git(col.path(), &["diff", "--cached", "--name-only"])
        .trim()
        .replace('\n', ",")
}

#[gpui_kit::test]
fn rows_show_their_stage_state_and_the_summary_counts(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    sb.git(col.path(), &["add", "a.yml"]);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Staged, Unstaged]);
    assert_eq!(
        label(cx, &s, "changes-summary").as_deref(),
        Some("1 staged of 2")
    );
    assert!(!present(cx, s.win, ("changes-partly", 0usize)));
}

#[gpui_kit::test]
fn a_checkbox_click_stages_and_unstages_that_file(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-stage", 1usize));
    idle(cx, &s);
    wait_for(cx, "staged", |cx| stages(cx) == [Unstaged, Staged]);
    assert_eq!(cached(&sb, &col), "new.yml");
    assert_eq!(
        label(cx, &s, "changes-summary").as_deref(),
        Some("1 staged of 2")
    );
    click(cx, &s, ("changes-stage", 1usize));
    idle(cx, &s);
    wait_for(cx, "unstaged", |cx| stages(cx) == [Unstaged, Unstaged]);
    assert_eq!(cached(&sb, &col), "");
}

#[gpui_kit::test]
fn a_partly_staged_file_is_unchecked_says_so_and_a_click_stages_the_rest(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    sb.git(col.path(), &["add", "a.yml"]);
    write(col.path(), "a.yml", &request("a", "http://x/again"));
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert_eq!(stages(cx), [Both, Unstaged]);
    assert!(present(cx, s.win, ("changes-partly", 0usize)));
    click(cx, &s, ("changes-stage", 0usize));
    idle(cx, &s);
    wait_for(cx, "fully staged", |cx| stages(cx)[0] == Staged);
    assert_eq!(sb.git(col.path(), &["diff", "--name-only"]).trim(), "");
}

#[gpui_kit::test]
fn stage_all_and_unstage_all_flip_on_one_button(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    wait_for(cx, "all staged", |cx| stages(cx) == [Staged, Staged]);
    assert_eq!(cached(&sb, &col), "a.yml,new.yml");
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    wait_for(cx, "none staged", |cx| stages(cx) == [Unstaged, Unstaged]);
}

#[gpui_kit::test]
fn a_second_toggle_while_one_runs_is_ignored(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    view(cx).update(cx, |v, cx| {
        v.toggle(0, cx);
        assert!(v.busy());
        v.toggle(1, cx);
    });
    idle(cx, &s);
    wait_for(cx, "the first file staged", |cx| {
        stages(cx) == [Staged, Unstaged]
    });
    assert_eq!(cached(&sb, &col), "a.yml");
}

#[gpui_kit::test]
fn space_toggles_the_selected_row(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    cx.update_window(s.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click(("changes-file", 1usize), cx)
    })
    .unwrap();
    cx.simulate_keystrokes(s.win, "space");
    idle(cx, &s);
    wait_for(cx, "the second file staged", |cx| {
        stages(cx) == [Unstaged, Staged]
    });
}

#[gpui_kit::test]
fn an_unborn_repository_stages_from_the_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    wait_for(cx, "staged", |cx| stages(cx).iter().all(|s| *s == Staged));
}

#[gpui_kit::test]
fn the_quiet_and_empty_states_have_no_controls(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert!(present(cx, s.win, "changes-toggle-all"));
    s.ws.update(cx, |w, cx| w.set_git(None, cx));
    frame(cx, s.win);
    assert!(present(cx, s.win, "changes-quiet"));
    assert!(!present(cx, s.win, "changes-toggle-all"));
    assert!(!present(cx, s.win, "changes-summary"));
}
