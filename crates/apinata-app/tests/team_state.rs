//! The workspace's team state: the read-only gate, clashes the editor cannot open, and problem events.

use crate::git_support::{Sandbox, write};
use crate::support::team::*;
use crate::support::*;
use apinata_app::command::{Command, Target, TreeOp};
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
fn the_clash_list_is_read_from_memory_and_refreshed_by_the_loop(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    put_copy(&s, "notes.txt", "mine\n");
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    wait_for(cx, "the clash", |cx| {
        clashes(cx, &s) == [copy_clash("notes.txt")]
    });
    // The copy goes behind the loop's back, so only a disk check on read would notice now.
    std::fs::remove_file(copies_of(&s).join("notes.txt")).unwrap();
    assert_eq!(clashes(cx, &s), [copy_clash("notes.txt")]);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.send(&root, edit_url("a.yml", "http://x/1"), cx);
        w.send(&root, Command::Save(Target::Request(ip("a.yml"))), cx);
    });
    wait_for(cx, "the next status drops it", |cx| {
        clashes(cx, &s).is_empty()
    });
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

fn rename_a_to_b() -> Command {
    Command::Tree(TreeOp::Rename {
        item: ip("a.yml"),
        name: "b".into(),
        rename_file: true,
    })
}

#[gpui_kit::test]
fn an_adopted_copy_follows_its_request_through_a_rename(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    put_copy(&s, "a.yml", &request("a", "http://x/mine"));
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    settle(cx);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.send(&root, rename_a_to_b(), cx));
    settle(cx);
    assert!(col.path().join("b.yml").exists());
    assert!(!copies_of(&s).join("a.yml").exists());
    let kept = std::fs::read_to_string(copies_of(&s).join("b.yml")).unwrap();
    assert!(kept.contains("x/mine"), "{kept}");
    // A fresh loop opens the moved copy as the renamed request's edits, not as a clash.
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    settle(cx);
    assert_eq!(clashes(cx, &s), []);
    assert!(copies_of(&s).join("b.yml").exists());
}

#[gpui_kit::test]
fn a_delete_again_clash_follows_its_file_through_a_rename(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let mate = sb.path().join("mate");
    write(&mate, "a.yml", &request("a", "http://x/theirs"));
    sb.commit_all(&mate, "theirs");
    sb.git(&mate, &["push", "-q"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
    let deleted = |rel: &str| Clash {
        rel: rel.into(),
        kind: ClashKind::DeletedLocally,
    };
    wait_for(cx, "the clash", |cx| clashes(cx, &s) == [deleted("a.yml")]);
    // The watcher reloads the returned file before the editor can rename it.
    settle(cx);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.send(&root, rename_a_to_b(), cx));
    wait_for(cx, "the renamed file", |_| {
        col.path().join("b.yml").exists()
    });
    settle(cx);
    assert_eq!(clashes(cx, &s), [deleted("b.yml")]);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    settle(cx);
    assert_eq!(clashes(cx, &s), [deleted("b.yml")]);
}

// Installs `body` as the executable hook `name` in `dir`, after a line that counts its runs
// in `runs`, so the hook can number its output by the run.
#[cfg(unix)]
fn hook(dir: &std::path::Path, name: &str, runs: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(dir).unwrap();
    let script = format!(
        "#!/bin/sh\necho run >> '{}'\nn=$(wc -l < '{}')\n{body}",
        runs.display(),
        runs.display()
    );
    std::fs::write(dir.join(name), script).unwrap();
    std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn runs_of(runs: &std::path::Path) -> usize {
    std::fs::read_to_string(runs).map_or(0, |t| t.lines().count())
}

fn problems(seen: &std::cell::RefCell<Vec<WorkspaceEvent>>) -> usize {
    (seen.borrow().iter())
        .filter(|e| matches!(e, WorkspaceEvent::TeamProblem { .. }))
        .count()
}

fn blocked(cx: &mut TestAppContext, s: &Started, is: fn(&Block) -> bool) -> bool {
    s.ws.read_with(cx, |w, _| {
        matches!(w.team_status(&s.root).map(|t| t.state), Some(TeamState::Blocked(b)) if is(&b))
    })
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_hook_refusal_is_one_problem_event(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let runs = sb.path().join("runs");
    hook(
        &col.path().join(".git/hooks"),
        "pre-commit",
        &runs,
        "echo nope >&2\nexit 1\n",
    );
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let seen = events(cx, &s.ws);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the hook block", |cx| {
        blocked(cx, &s, |b| matches!(b, Block::Hook { .. }))
    });
    settle(cx);
    assert_eq!(problems(&seen), 1);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_hook_whose_output_changes_on_each_save_is_one_problem_event(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let runs = sb.path().join("runs");
    hook(
        &col.path().join(".git/hooks"),
        "pre-commit",
        &runs,
        "echo \"refused, run $n\" >&2\nexit 1\n",
    );
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let seen = events(cx, &s.ws);
    for (i, rel) in ["b.yml", "c.yml", "d.yml"].into_iter().enumerate() {
        std::fs::write(col.path().join(rel), request(rel, "http://x/b")).unwrap();
        wait_for(cx, "the hook to run again", |_| runs_of(&runs) > i);
        wait_for(cx, "the hook block", |cx| {
            blocked(cx, &s, |b| matches!(b, Block::Hook { .. }))
        });
    }
    settle(cx);
    assert_eq!(problems(&seen), 1);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_server_refusal_whose_text_changes_on_retry_is_one_problem_event(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let runs = sb.path().join("runs");
    hook(
        &bare.join("hooks"),
        "pre-receive",
        &runs,
        "echo \"refused, run $n\" >&2\nexit 1\n",
    );
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let seen = events(cx, &s.ws);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    for i in 0..3 {
        wait_for(cx, "the server hook to run", |_| runs_of(&runs) > i);
        wait_for(cx, "the failed block", |cx| {
            blocked(cx, &s, |b| matches!(b, Block::Failed { .. }))
        });
        s.ws.read_with(cx, |w, _| w.team_sync(&s.root))
            .unwrap()
            .retry();
    }
    wait_for(cx, "the last retry", |_| runs_of(&runs) > 3);
    settle(cx);
    assert_eq!(problems(&seen), 1);
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_secret_refusal_with_new_findings_is_a_new_problem_event(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    let bare = shared_origin(&sb, col.path());
    let runs = sb.path().join("runs");
    // Every run lists a finding under a new rule, beside a line that changes too.
    hook(
        &bare.join("hooks"),
        "pre-receive",
        &runs,
        "echo \"checked, run $n\" >&2\n\
         echo \"potluck-reject secret_env_file refs%2Fheads%2Fmain b.yml rule$n\" >&2\nexit 1\n",
    );
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let seen = events(cx, &s.ws);
    std::fs::write(col.path().join("b.yml"), request("b", "http://x/b")).unwrap();
    wait_for(cx, "the secret block", |cx| {
        blocked(cx, &s, |b| matches!(b, Block::Secret { .. }))
    });
    s.ws.read_with(cx, |w, _| w.team_sync(&s.root))
        .unwrap()
        .retry();
    wait_for(cx, "the second refusal", |_| runs_of(&runs) > 1);
    wait_for(cx, "the secret block", |cx| {
        blocked(cx, &s, |b| matches!(b, Block::Secret { .. }))
    });
    settle(cx);
    assert_eq!(problems(&seen), 2);
}

#[gpui_kit::test]
fn the_read_role_refuses_a_replace_and_its_reply_closes(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Read, FAST);
    let root = s.root.clone();
    let (reply, answer) = async_channel::bounded(1);
    let command = Command::Replace {
        query: "x/0".into(),
        replacement: "x/no".into(),
        options: Default::default(),
        scope: apinata_core::ReplaceScope::All,
        reply,
    };
    assert!(
        s.ws.update(cx, |w, cx| w.send(&root, command, cx))
            .is_none()
    );
    assert!(answer.try_recv().is_err_and(|e| e.is_closed()));
}

#[gpui_kit::test]
fn each_refused_replace_import_and_conversion_says_read_only(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Read, FAST);
    let seen = events(cx, &s.ws);
    let root = s.root.clone();
    let expected = apinata_core::Collection::open(col.path())
        .unwrap()
        .conversion_preview(&ip("a.yml"), apinata_core::RequestKind::Graphql)
        .unwrap();
    let env = apinata_core::import::parse_environment(
        None,
        br#"{"name":"dev","values":[],"_postman_variable_scope":"environment"}"#,
    )
    .unwrap();
    let sent = s.ws.update(cx, |w, cx| {
        // A refused save first spends the once-per-loop notice an autosave would raise.
        let save = w.send(&root, Command::Save(Target::Request(ip("a.yml"))), cx);
        let replace = Command::Replace {
            query: "x/0".into(),
            replacement: "x/no".into(),
            options: Default::default(),
            scope: apinata_core::ReplaceScope::All,
            reply: async_channel::bounded(1).0,
        };
        let convert = Command::ConvertKind {
            item: ip("a.yml"),
            kind: apinata_core::RequestKind::Graphql,
            expected,
            reply: async_channel::bounded(1).0,
        };
        let import = Command::ImportEnvironment {
            env,
            reply: async_channel::bounded(1).0,
        };
        [
            save,
            w.send(&root, replace, cx),
            w.send(&root, convert, cx),
            w.send(&root, import, cx),
        ]
    });
    assert_eq!(sent, [None, None, None, None]);
    let told = (seen.borrow().iter())
        .filter(|e| matches!(e, WorkspaceEvent::TeamReadOnly { .. }))
        .count();
    assert_eq!(told, 4);
}

#[gpui_kit::test]
fn the_read_role_still_drops_drafts(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Read, FAST);
    let seen = events(cx, &s.ws);
    let root = s.root.clone();
    let sent = s.ws.update(cx, |w, cx| {
        [
            w.send(&root, Command::Revert(Target::Request(ip("a.yml"))), cx),
            w.send(
                &root,
                Command::DiscardStale(Target::Request(ip("a.yml"))),
                cx,
            ),
            w.send(
                &root,
                Command::Tree(TreeOp::DiscardDraft { draft: ip("a.yml") }),
                cx,
            ),
        ]
    });
    assert!(sent.iter().all(Option::is_some), "{sent:?}");
    assert!(!(seen.borrow().iter()).any(|e| matches!(e, WorkspaceEvent::TeamReadOnly { .. })));
}

#[gpui_kit::test]
fn a_read_only_folder_only_keeps_theirs(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    put_copy(&s, "notes.txt", "mine\n");
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    wait_for(cx, "the clash", |cx| clashes(cx, &s).len() == 1);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| w.set_team_role(&root, Role::Read, cx));
    for answer in [ClashAnswer::UseMine, ClashAnswer::DeleteAgain] {
        let err =
            s.ws.update(cx, |w, cx| w.settle_clash(&root, "notes.txt", answer, cx));
        assert_eq!(
            err.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
    assert_eq!(
        std::fs::read_to_string(col.path().join("notes.txt")).unwrap(),
        "theirs\n"
    );
    s.ws.update(cx, |w, cx| {
        w.settle_clash(&root, "notes.txt", ClashAnswer::KeepTheirs, cx)
    })
    .unwrap();
    assert_eq!(clashes(cx, &s), []);
}

#[gpui_kit::test]
fn an_open_delete_again_clash_survives_a_restart(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
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
    // A fresh loop holds nothing in memory from the last one, as after an app restart.
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    settle(cx);
    assert_eq!(clashes(cx, &s), [deleted]);
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.settle_clash(&root, "a.yml", ClashAnswer::DeleteAgain, cx)
    })
    .unwrap();
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    settle(cx);
    assert_eq!(clashes(cx, &s), []);
}

// Deletes `a.yml` here while a teammate changes it to `url` there, so the next pull clashes.
fn delete_here_change_there(sb: &Sandbox, col: &tempfile::TempDir, url: &str) {
    let mate = sb.path().join("mate");
    sb.git(&mate, &["pull", "-q"]);
    write(&mate, "a.yml", &request("a", url));
    sb.commit_all(&mate, url);
    sb.git(&mate, &["push", "-q"]);
    std::fs::remove_file(col.path().join("a.yml")).unwrap();
}

#[gpui_kit::test]
fn the_same_delete_clash_shows_again_after_it_was_dismissed(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    let deleted = Clash {
        rel: "a.yml".into(),
        kind: ClashKind::DeletedLocally,
    };
    delete_here_change_there(&sb, &col, "http://x/theirs1");
    wait_for(cx, "the first clash", |cx| {
        clashes(cx, &s) == [deleted.clone()]
    });
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.settle_clash(&root, "a.yml", ClashAnswer::KeepTheirs, cx)
    })
    .unwrap();
    assert_eq!(clashes(cx, &s), []);
    settle(cx);
    // The second pull conflicts on the very same path, so its conflict list is unchanged.
    delete_here_change_there(&sb, &col, "http://x/theirs2");
    wait_for(cx, "the second clash", |cx| {
        clashes(cx, &s) == [deleted.clone()]
    });
}

#[cfg(unix)]
#[gpui_kit::test]
fn use_mine_never_writes_through_a_symlink(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), with_notes());
    shared_origin(&sb, col.path());
    let s = app(cx, &sb, &col);
    put_copy(&s, "notes.txt", "mine\n");
    std::fs::create_dir_all(copies_of(&s).join("sub")).unwrap();
    std::fs::write(copies_of(&s).join("sub/n.txt"), "mine\n").unwrap();
    start_team_as(cx, &sb, &s, Role::Write, FAST);
    wait_for(cx, "the clashes", |cx| clashes(cx, &s).len() == 2);
    let outside = sb.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("target.txt"), "outside\n").unwrap();
    std::fs::remove_file(col.path().join("notes.txt")).unwrap();
    std::os::unix::fs::symlink(outside.join("target.txt"), col.path().join("notes.txt")).unwrap();
    std::os::unix::fs::symlink(&outside, col.path().join("sub")).unwrap();
    let root = s.root.clone();
    for rel in ["notes.txt", "sub/n.txt"] {
        let done = s.ws.update(cx, |w, cx| {
            w.settle_clash(&root, rel, ClashAnswer::UseMine, cx)
        });
        assert!(done.is_err(), "{rel}");
    }
    assert_eq!(
        std::fs::read_to_string(outside.join("target.txt")).unwrap(),
        "outside\n"
    );
    assert!(!outside.join("n.txt").exists());
}
