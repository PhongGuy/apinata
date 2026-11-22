//! `.env` on the Changes dialog: its row is marked as holding secrets, Stage all leaves it out,
//! and ticking it by hand asks first.

use crate::git_support;
use crate::h;
use crate::support;

use apinata_app::git_status::GitStage::{Staged, Unstaged};
use git_support::{Sandbox, write};
use gpui_kit::TestAppContext;
use h::*;
use support::*;

fn cached(sb: &Sandbox, col: &tempfile::TempDir) -> String {
    sb.git(col.path(), &["diff", "--cached", "--name-only"])
        .trim()
        .replace('\n', ",")
}

// A tracked `.env` and `a.yml`, both edited since the commit; rows `.env` then `a.yml`.
fn tracked_dotenv(sb: &Sandbox) -> tempfile::TempDir {
    let col = committed(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
            (".env", "API_KEY=abc\n"),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    write(col.path(), ".env", "API_KEY=new\n");
    col
}

#[gpui_kit::test]
fn the_dotenv_row_is_marked_and_stage_all_leaves_it_out(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = tracked_dotenv(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    assert!(present(cx, s.win, ("changes-secrets", 0usize)));
    assert!(!present(cx, s.win, ("changes-secrets", 1usize)));
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    wait_for(cx, "a.yml staged", |cx| stages(cx) == [Unstaged, Staged]);
    assert_eq!(cached(&sb, &col), "a.yml");
}

#[gpui_kit::test]
fn an_untracked_dotenv_whose_rule_was_removed_stays_out_of_stage_all(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    write(col.path(), ".env", "API_KEY=abc\n");
    let s = git_started(cx, &sb, &col);
    wait_for(cx, "the rule", |_| col.path().join(".gitignore").exists());
    std::fs::remove_file(col.path().join(".gitignore")).unwrap();
    s.ws.update(cx, |w, cx| w.refresh_git(col.path(), cx));
    wait_for(cx, ".env listed", |cx| {
        s.ws.read_with(cx, |w, _| w.git_changes(col.path()))
            .is_some_and(|g| g.files.contains_key(std::path::Path::new(".env")))
    });
    open_on(cx, &s, col.path());
    click(cx, &s, "changes-toggle-all");
    idle(cx, &s);
    wait_for(cx, "the rest staged", |cx| {
        stages(cx) == [Unstaged, Staged, Staged]
    });
    assert_eq!(cached(&sb, &col), "a.yml,new.yml");
}

#[gpui_kit::test]
fn ticking_dotenv_asks_once_and_cancel_leaves_it(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = tracked_dotenv(&sb);
    let s = git_started(cx, &sb, &col);
    open_on(cx, &s, col.path());
    click(cx, &s, ("changes-stage", 0usize));
    assert!(confirm_open(cx), "it asks first");
    answer_confirm(cx, "confirm-cancel");
    idle(cx, &s);
    assert_eq!(stages(cx), [Unstaged, Unstaged]);
    assert_eq!(cached(&sb, &col), "");

    click(cx, &s, ("changes-stage", 0usize));
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-stage-secrets");
    idle(cx, &s);
    wait_for(cx, ".env staged", |cx| stages(cx) == [Staged, Unstaged]);
    assert_eq!(cached(&sb, &col), ".env");

    // Once confirmed, the same dialog does not ask again.
    click(cx, &s, ("changes-stage", 0usize));
    idle(cx, &s);
    wait_for(cx, ".env unstaged", |cx| stages(cx) == [Unstaged, Unstaged]);
    click(cx, &s, ("changes-stage", 0usize));
    assert!(!confirm_open(cx));
    idle(cx, &s);
    wait_for(cx, ".env staged again", |cx| {
        stages(cx) == [Staged, Unstaged]
    });
}
