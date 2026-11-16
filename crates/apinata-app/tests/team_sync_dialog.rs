//! The team sync dialog: the live problem lines and the action that fixes each one.

use crate::git_support::Sandbox;
use crate::support::team::*;
use crate::support::*;
use apinata_app::team_sync_dialog::{self, SyncView};
use apinata_core::team::{Block, Role, TeamState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, px};
use std::time::Duration;

const FAST: Duration = Duration::from_millis(50);

fn app(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    http_ready(cx);
    team_started(cx, sb, col, |_, _| {}).0
}

fn open(cx: &mut TestAppContext, s: &Started) -> Entity<SyncView> {
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let view = cx
        .update_window(s.win, |_, w, cx| team_sync_dialog::open(&ws, root, w, cx))
        .unwrap();
    frame(cx, s.win);
    view
}

fn lines(cx: &mut TestAppContext, view: &Entity<SyncView>) -> Vec<String> {
    view.read_with(cx, |v, cx| v.lines(cx))
}

fn state(cx: &mut TestAppContext, s: &Started) -> Option<TeamState> {
    s.ws.read_with(cx, |w, _| w.team_status(&s.root).map(|t| t.state))
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_secret_refusal_lists_the_findings_and_retry_pushes_the_fix(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    sb.fake_tool("origin.git/hooks", "pre-receive", "#!/bin/sh\nwhile read old new ref; do\n  if git log -p \"$new\" --not --all | grep -q s3cr3t; then\n    echo \"potluck-reject secret_value $ref a.yml secret_value\" >&2\n    exit 1\n  fi\ndone\n");
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/s3cr3t")).unwrap();
    wait_for(cx, "the refusal", |cx| {
        matches!(
            state(cx, &s),
            Some(TeamState::Blocked(Block::Secret { .. }))
        )
    });
    let view = open(cx, &s);
    let text = lines(cx, &view);
    assert_eq!(
        text[1],
        "a.yml: secret_value on refs/\u{2060}heads/\u{2060}main"
    );
    assert!(text.iter().all(|l| !l.contains("s3cr3t")), "{text:?}");
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/fixed")).unwrap();
    wait_for(cx, "the fix commit", |cx| {
        s.ws.read_with(cx, |w, _| {
            w.team_status(&s.root).is_some_and(|t| t.unpushed == 2)
        })
    });
    click(cx, &s, "team-retry");
    wait_for(cx, "the squashed push", |_| {
        origin_has(&sb, &bare, "a.yml", "x/fixed")
    });
    wait_for(cx, "in sync", |cx| lines(cx, &view) == ["In sync."]);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_refusing_hook_shows_its_output_and_retry_commits_once_it_passes(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let hook = col.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho nope >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the hook block", |cx| {
        matches!(state(cx, &s), Some(TeamState::Blocked(Block::Hook { .. })))
    });
    let view = open(cx, &s);
    assert_eq!(
        lines(cx, &view),
        ["A commit hook refused the commit:", "nope"]
    );
    std::fs::remove_file(&hook).unwrap();
    click(cx, &s, "team-retry");
    wait_for(cx, "the push", |_| origin_has(&sb, &bare, "b.yml", "x/b"));
}

#[gpui_kit::test]
fn discard_drops_outside_edits_in_a_read_only_folder(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Read, FAST);
    std::fs::write(col.path().join("a.yml"), request("a", "http://x/outside")).unwrap();
    wait_for(cx, "paused", |cx| {
        state(cx, &s) == Some(TeamState::ReadOnly { paused: true })
    });
    let view = open(cx, &s);
    assert!(shown(cx, s.win, "team-discard"));
    assert_eq!(lines(cx, &view).len(), 2);
    click(cx, &s, "team-discard");
    wait_for(cx, "resumed", |cx| {
        state(cx, &s) == Some(TeamState::ReadOnly { paused: false })
    });
    assert!(
        std::fs::read_to_string(col.path().join("a.yml"))
            .unwrap()
            .contains("x/0")
    );
}

#[gpui_kit::test]
fn an_idle_folder_offers_no_action(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let view = open(cx, &s);
    assert_eq!(lines(cx, &view), ["In sync."]);
    assert!(!shown(cx, s.win, "team-retry"));
    assert!(!shown(cx, s.win, "team-discard"));
    assert!(cx.update(|cx| team_sync_dialog::current(cx)).is_some());
}

#[cfg(unix)]
#[gpui_kit::test]
fn long_hook_output_scrolls_and_keeps_retry_in_the_window(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let hook = col.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    let script = "#!/bin/sh\ni=0\nwhile [ $i -lt 200 ]; do echo \"refused line $i\" >&2; i=$((i+1)); done\nexit 1\n";
    std::fs::write(&hook, script).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the hook block", |cx| {
        matches!(state(cx, &s), Some(TeamState::Blocked(Block::Hook { .. })))
    });
    let view = open(cx, &s);
    let last = lines(cx, &view).len() - 1;
    let (retry, window, list, tail, cap) = cx
        .update_window(s.win, |_, w, _| {
            (
                w.try_find("team-retry").map(|e| e.bounds()),
                w.viewport_size(),
                w.try_find("team-lines").map(|e| e.bounds()),
                w.try_find(("team-line", last)).map(|e| e.bounds()),
                apinata_app::dialog_frame::titled_body_room(w) * team_sync_dialog::OUTPUT_SHARE,
            )
        })
        .unwrap();
    let retry = retry.expect("Retry is shown");
    assert!(
        retry.bottom() <= window.height,
        "{retry:?} outside {window:?}"
    );
    // The list fills its cap exactly and the output runs on below it, so the rest scrolls.
    let (list, tail) = (list.expect("the lines"), tail.expect("the output"));
    assert!(
        (list.size.height - cap).abs() <= px(1.),
        "{list:?} against a cap of {cap:?}"
    );
    assert!(tail.bottom() > list.bottom(), "{tail:?} fits in {list:?}");
}

// The first problem line as the open dialog last drew it, not as the workspace holds it now.
fn drawn_headline(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(("team-line", 0usize))
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[cfg(unix)]
#[gpui_kit::test]
fn the_open_dialog_follows_the_sync_state_until_in_sync(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let hook = col.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho nope >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the hook block", |cx| {
        matches!(state(cx, &s), Some(TeamState::Blocked(Block::Hook { .. })))
    });
    open(cx, &s);
    assert_eq!(
        drawn_headline(cx, &s).as_deref(),
        Some("A commit hook refused the commit:")
    );
    std::fs::remove_file(&hook).unwrap();
    click(cx, &s, "team-retry");
    wait_for(cx, "the push", |_| origin_has(&sb, &bare, "b.yml", "x/b"));
    wait_for(cx, "idle", |cx| state(cx, &s) == Some(TeamState::Idle));
    assert_eq!(drawn_headline(cx, &s).as_deref(), Some("In sync."));
}
