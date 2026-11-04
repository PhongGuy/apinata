//! `Workspace::run_git_write`: stage, unstage and commit through core, with hooks, identity and
//! the outside-staged refusal, all in sandboxed repositories.

use crate::git_support;
use crate::support;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use apinata_app::git_status::GitStage;
use apinata_app::session::Session;
use apinata_app::workspace::{GitWrite, GitWriteDone, Workspace};
use apinata_core::GitOptions;
use git_support::{Sandbox, write};
use gpui_kit::{Entity, TestAppContext};
use support::*;

fn two(sb: &Sandbox, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = write_tree(files);
    sb.git(dir.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(dir.path(), "init");
    dir
}

fn simple(sb: &Sandbox) -> tempfile::TempDir {
    let col = two(
        sb,
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &request("a", "http://x/a")),
        ],
    );
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    col
}

fn opened(cx: &mut TestAppContext, options: GitOptions, root: &Path) -> Entity<Workspace> {
    init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    ws.update(cx, |w, cx| w.open(root.to_path_buf(), cx));
    cx.run_until_parked();
    std::mem::forget(state);
    ws
}

fn run(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path, op: GitWrite) -> GitWriteDone {
    let slot = Rc::new(RefCell::new(None));
    let out = slot.clone();
    ws.update(cx, |w, cx| {
        let task = w.run_git_write(root, op, cx).expect("git is on");
        cx.spawn(async move |_, _| *out.borrow_mut() = Some(task.await))
            .detach();
    });
    wait_for(cx, "the git write", |_| slot.borrow().is_some());
    slot.take().unwrap()
}

fn stage_of(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    rel: &str,
) -> Option<GitStage> {
    ws.read_with(cx, |w, _| w.git_changes(root))
        .and_then(|g| g.stage.get(Path::new(rel)).copied())
}

/// Writes an executable hook with a child shell, so no descriptor of this process is open on
/// it when another test forks (the same reason `Sandbox::fake_git` does it).
#[cfg(unix)]
fn hook(repo: &Path, name: &str, body: &str) {
    let path = repo.join(".git/hooks").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let ok = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"")
        .arg("sh")
        .arg(body)
        .arg(&path)
        .status()
        .unwrap();
    assert!(ok.success());
}

/// The sandbox environment with every author and committer variable removed.
fn without_identity(sb: &Sandbox) -> GitOptions {
    for (name, _) in std::env::vars() {
        assert!(
            !name.starts_with("GIT_AUTHOR_") && !name.starts_with("GIT_COMMITTER_"),
            "{name} is set in the test process; unset it to run the identity tests"
        );
    }
    sb.env()
        .into_iter()
        .filter(|(k, _)| {
            let k = k.to_string_lossy();
            !k.starts_with("GIT_AUTHOR_") && !k.starts_with("GIT_COMMITTER_")
        })
        .fold(GitOptions::new(), |o, (k, v)| o.env(k, v))
}

#[gpui_kit::test]
fn stage_and_unstage_round_trip_into_the_status(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = simple(&sb);
    let ws = opened(cx, sb.options(), col.path());
    assert_eq!(
        stage_of(cx, &ws, col.path(), "a.yml"),
        Some(GitStage::Unstaged)
    );
    let done = run(cx, &ws, col.path(), GitWrite::Stage(vec!["a.yml".into()]));
    assert_eq!(done, GitWriteDone::Done);
    wait_for(cx, "staged", |cx| {
        stage_of(cx, &ws, col.path(), "a.yml") == Some(GitStage::Staged)
    });
    run(cx, &ws, col.path(), GitWrite::Unstage(vec!["a.yml".into()]));
    wait_for(cx, "unstaged", |cx| {
        stage_of(cx, &ws, col.path(), "a.yml") == Some(GitStage::Unstaged)
    });
}

#[gpui_kit::test]
fn a_commit_lands_and_the_status_empties(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = simple(&sb);
    let ws = opened(cx, sb.options(), col.path());
    run(cx, &ws, col.path(), GitWrite::Stage(vec![".".into()]));
    let done = run(cx, &ws, col.path(), GitWrite::Commit("Edit a".into()));
    assert_eq!(
        done,
        GitWriteDone::Committed {
            short_id: sb.short_head(col.path()),
            subject: "Edit a".into()
        }
    );
    assert_eq!(
        sb.git(col.path(), &["log", "-1", "--format=%s"]).trim(),
        "Edit a"
    );
    wait_for(cx, "an empty status", |cx| {
        ws.read_with(cx, |w, _| w.git_changes(col.path()))
            .is_some_and(|g| g.files.is_empty())
    });
}

#[gpui_kit::test]
fn an_unborn_repository_stages_and_commits(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    let ws = opened(cx, sb.options(), col.path());
    run(cx, &ws, col.path(), GitWrite::Stage(vec![".".into()]));
    let done = run(cx, &ws, col.path(), GitWrite::Commit("first".into()));
    assert!(matches!(done, GitWriteDone::Committed { .. }), "{done:?}");
    assert_eq!(
        sb.git(col.path(), &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
}

#[gpui_kit::test]
fn an_empty_message_or_nothing_staged_is_a_failure_in_words(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = simple(&sb);
    let ws = opened(cx, sb.options(), col.path());
    run(cx, &ws, col.path(), GitWrite::Stage(vec![".".into()]));
    let empty = run(cx, &ws, col.path(), GitWrite::Commit("  \n ".into()));
    assert_eq!(
        empty,
        GitWriteDone::Failed("The commit message is empty.".into())
    );
    run(cx, &ws, col.path(), GitWrite::Unstage(vec![".".into()]));
    let none = run(cx, &ws, col.path(), GitWrite::Commit("x".into()));
    assert_eq!(none, GitWriteDone::Failed("Nothing is staged.".into()));
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_failing_pre_commit_hook_blocks_and_its_output_comes_back(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = simple(&sb);
    hook(
        col.path(),
        "pre-commit",
        "#!/bin/sh\necho 'hook says no' >&2\nexit 1\n",
    );
    let ws = opened(cx, sb.options(), col.path());
    run(cx, &ws, col.path(), GitWrite::Stage(vec![".".into()]));
    match run(cx, &ws, col.path(), GitWrite::Commit("Edit a".into())) {
        GitWriteDone::Failed(text) => assert!(text.contains("hook says no"), "{text}"),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(
        sb.git(col.path(), &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
}

#[gpui_kit::test]
fn a_missing_identity_fails_with_gits_text_and_a_hint(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = simple(&sb);
    let ws = opened(cx, without_identity(&sb), col.path());
    run(cx, &ws, col.path(), GitWrite::Stage(vec![".".into()]));
    match run(cx, &ws, col.path(), GitWrite::Commit("Edit a".into())) {
        GitWriteDone::Failed(text) => {
            assert!(text.contains("identity"), "{text}");
            assert!(text.contains("git config user.name"), "{text}");
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(
        sb.git(col.path(), &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
}

#[gpui_kit::test]
fn staged_files_outside_a_nested_collection_refuse_the_commit(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let repo = two(
        &sb,
        &[
            ("col/opencollection.yml", ROOT),
            ("col/a.yml", &request("a", "http://x/a")),
            ("other.txt", "one\n"),
        ],
    );
    let col = repo.path().join("col");
    write(&col, "a.yml", &request("a", "http://x/changed"));
    write(repo.path(), "other.txt", "two\n");
    sb.git(repo.path(), &["add", "other.txt"]);
    let ws = opened(cx, sb.options(), &col);
    run(cx, &ws, &col, GitWrite::Stage(vec!["a.yml".into()]));
    match run(cx, &ws, &col, GitWrite::Commit("Edit a".into())) {
        GitWriteDone::Failed(text) => {
            assert!(text.contains("outside"), "{text}");
            assert!(text.contains("other.txt"), "{text}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(
        sb.git(repo.path(), &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
}

#[gpui_kit::test]
fn without_git_options_nothing_runs(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = simple(&sb);
    let ws = opened(cx, sb.options(), col.path());
    ws.update(cx, |w, cx| w.set_git(None, cx));
    let task = ws.update(cx, |w, cx| {
        w.run_git_write(col.path(), GitWrite::Stage(vec![".".into()]), cx)
    });
    assert!(task.is_none());
}
