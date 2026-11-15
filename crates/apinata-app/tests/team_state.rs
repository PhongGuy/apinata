//! The workspace's team state: the read-only gate, clashes the editor cannot open, and problem events.

use crate::git_support::{Sandbox, write};
use crate::support::team::*;
use crate::support::*;
use apinata_app::command::{Command, Target};
use apinata_app::workspace::{Clash, ClashAnswer, ClashKind, WorkspaceEvent};
use apinata_core::team::{Block, Role, TeamState};
use gpui_kit::TestAppContext;
use std::time::Duration;

const FAST: Duration = Duration::from_millis(50);

fn app(cx: &mut TestAppContext, sb: &Sandbox, col: &tempfile::TempDir) -> Started {
    http_ready(cx);
    team_started(cx, sb, col, |_, _| {}).0
}

fn clashes(cx: &mut TestAppContext, s: &Started) -> Vec<Clash> {
    s.ws.read_with(cx, |w, _| w.team_clashes(&s.root))
}

fn copy_clash(rel: &str) -> Clash {
    Clash {
        rel: rel.into(),
        kind: ClashKind::Copy,
    }
}

fn with_notes() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("notes.txt", "theirs\n"),
    ])
}

fn put_copy(s: &Started, rel: &str, text: &str) {
    std::fs::create_dir_all(copies_of(s)).unwrap();
    std::fs::write(copies_of(s).join(rel), text).unwrap();
}

#[gpui_kit::test]
fn the_read_role_refuses_edits_and_saves_and_says_so_once(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Read, FAST);
    let seen = events(cx, &s.ws);
    let root = s.root.clone();
    let sent = s.ws.update(cx, |w, cx| {
        [
            w.send(&root, edit_url("a.yml", "http://x/no"), cx),
            w.send(&root, edit_url("a.yml", "http://x/no2"), cx),
            w.send(&root, Command::Save(Target::Request(ip("a.yml"))), cx),
        ]
    });
    assert_eq!(sent, [None, None, None]);
    assert!(s.ws.read_with(cx, |w, _| w.is_team_read_only(&root)));
    let told = (seen.borrow().iter())
        .filter(|e| matches!(e, WorkspaceEvent::TeamReadOnly { .. }))
        .count();
    assert_eq!(told, 1);
    assert!(
        std::fs::read_to_string(col.path().join("a.yml"))
            .unwrap()
            .contains("x/0")
    );
}

#[gpui_kit::test]
fn a_role_the_server_lowered_turns_read_only(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let root = s.root.clone();
    assert!(
        s.ws.update(cx, |w, cx| w.send(
            &root,
            edit_url("a.yml", "http://x/1"),
            cx
        ))
        .is_some()
    );
    s.ws.update(cx, |w, cx| w.set_team_role(&root, Role::Read, cx));
    assert!(s.ws.read_with(cx, |w, _| w.is_team_read_only(&root)));
    assert!(
        s.ws.update(cx, |w, cx| w.send(
            &root,
            edit_url("a.yml", "http://x/2"),
            cx
        ))
        .is_none()
    );
}

#[gpui_kit::test]
fn a_copy_the_editor_cannot_open_is_a_clash_and_use_mine_pushes_it(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    let bare = shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    put_copy(&s, "notes.txt", "mine\n");
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    wait_for(cx, "the clash", |cx| {
        clashes(cx, &s) == [copy_clash("notes.txt")]
    });
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.settle_clash(&root, "notes.txt", ClashAnswer::UseMine, cx)
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(col.path().join("notes.txt")).unwrap(),
        "mine\n"
    );
    assert!(!copies_of(&s).join("notes.txt").exists());
    assert_eq!(clashes(cx, &s), []);
    wait_for(cx, "mine pushed", |_| {
        origin_has(&sb, &bare, "notes.txt", "mine")
    });
}

#[gpui_kit::test]
fn keep_theirs_drops_the_copy_and_leaves_the_file(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    put_copy(&s, "notes.txt", "mine\n");
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    wait_for(cx, "the clash", |cx| clashes(cx, &s).len() == 1);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.settle_clash(&root, "notes.txt", ClashAnswer::KeepTheirs, cx)
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(col.path().join("notes.txt")).unwrap(),
        "theirs\n"
    );
    assert!(!copies_of(&s).join("notes.txt").exists());
    assert_eq!(clashes(cx, &s), []);
}

#[gpui_kit::test]
fn an_adopted_copy_is_not_a_clash(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    put_copy(&s, "a.yml", &request("a", "http://x/mine"));
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    settle(cx);
    assert_eq!(clashes(cx, &s), []);
    assert!(
        copies_of(&s).join("a.yml").exists(),
        "the banner still owns it"
    );
}

#[gpui_kit::test]
fn a_file_deleted_here_and_changed_there_offers_delete_again(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let mate = sb.path().join("mate");
    write(&mate, "a.yml", &request("a", "http://x/theirs"));
    sb.commit_all(&mate, "theirs");
    sb.git(&mate, &["push", "-q"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    let deleted = Clash {
        rel: "a.yml".into(),
        kind: ClashKind::DeletedLocally,
    };
    wait_for(cx, "the clash", |cx| clashes(cx, &s) == [deleted.clone()]);
    assert!(col.path().join("a.yml").exists(), "their file came back");
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.settle_clash(&root, "a.yml", ClashAnswer::DeleteAgain, cx)
    })
    .unwrap();
    assert!(!col.path().join("a.yml").exists());
    assert_eq!(clashes(cx, &s), []);
    wait_for(cx, "the delete pushed", |_| {
        !origin_has(&sb, &bare, "a.yml", "http")
    });
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_hook_refusal_is_one_problem_event(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let hooks = col.path().join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(
        hooks.join("pre-commit"),
        "#!/bin/sh\necho nope >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(
        hooks.join("pre-commit"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let seen = events(cx, &s.ws);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the hook block", |cx| {
        s.ws.read_with(cx, |w, _| {
            matches!(
                w.team_status(&s.root).map(|t| t.state),
                Some(TeamState::Blocked(Block::Hook { .. }))
            )
        })
    });
    settle(cx);
    let problems = (seen.borrow().iter())
        .filter(|e| matches!(e, WorkspaceEvent::TeamProblem { .. }))
        .count();
    assert_eq!(problems, 1);
}
