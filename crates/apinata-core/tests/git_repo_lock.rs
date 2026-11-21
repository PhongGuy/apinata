//! The per-repository lock that keeps the app's pull and switch and MCP's writes from
//! overlapping: a second holder waits, then reports busy; a linked worktree and a folder outside
//! any repository never contend with the main repository.

use crate::git_support;

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use apinata_core::{GitOptions, RepoBusy, RepoLock, repo_lock};
use git_support::{SCRUBBED, Sandbox, write};

const SHORT: Duration = Duration::from_millis(200);

fn committed(sb: &Sandbox) -> std::path::PathBuf {
    let repo = sb.repo("r");
    write(&repo, "a.yml", "a: 1\n");
    sb.commit_all(&repo, "init");
    repo
}

fn held(repo: &Path, options: &GitOptions) -> RepoLock {
    let lock = repo_lock(repo, Duration::ZERO, options).expect("the lock is free");
    assert!(lock.is_held(), "a repository takes a real lock");
    lock
}

#[test]
fn a_second_holder_waits_then_is_busy_and_succeeds_once_the_first_drops() {
    let sb = Sandbox::new();
    let repo = committed(&sb);
    let options = sb.options();
    let first = held(&repo, &options);

    let started = Instant::now();
    let second = repo_lock(&repo, SHORT, &options);
    assert!(
        matches!(second, Err(RepoBusy { .. })),
        "the second holder is busy"
    );
    assert!(started.elapsed() >= SHORT, "busy only after the wait");

    // A sibling test may be mid-spawn with a copy of the dropped descriptor until its exec, so
    // the release can lag the drop; take the lock with the lock's own wait, not a bare try.
    drop(first);
    let again = repo_lock(&repo, Duration::from_secs(30), &options).expect("free after the drop");
    assert!(again.is_held(), "a repository takes a real lock");
}

#[test]
fn a_lock_released_during_the_wait_is_taken() {
    let sb = Sandbox::new();
    let repo = committed(&sb);
    let options = sb.options();
    let first = held(&repo, &options);
    let release = std::thread::spawn(move || {
        std::thread::sleep(SHORT);
        drop(first);
    });

    let second = repo_lock(&repo, Duration::from_secs(5), &options).expect("taken after release");
    assert!(second.is_held());
    release.join().unwrap();
}

#[test]
fn a_folder_outside_any_repository_takes_no_lock() {
    let sb = Sandbox::new();
    let plain = sb.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    let options = sb.options();

    let first = repo_lock(&plain, SHORT, &options).expect("no repository, no contention");
    let second = repo_lock(&plain, SHORT, &options).expect("no repository, no contention");
    assert!(!first.is_held() && !second.is_held());
    assert!(!plain.join("apinata.lock").exists());
}

#[test]
fn a_linked_worktree_locks_apart_from_its_main_repository() {
    let sb = Sandbox::new();
    let repo = committed(&sb);
    let linked = sb.path().join("linked");
    sb.git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            linked.to_str().unwrap(),
        ],
    );
    let options = sb.options();

    let _main = held(&repo, &options);
    let _side = held(&linked, &options);
}

#[test]
fn a_subfolder_of_the_repository_shares_its_lock() {
    let sb = Sandbox::new();
    let repo = committed(&sb);
    write(&repo, "col/opencollection.yml", "x: 1\n");
    let options = sb.options();

    let _top = held(&repo, &options);
    assert!(repo_lock(&repo.join("col"), SHORT, &options).is_err());
}

// The child half of the cross-process test: with the variables set it takes the lock, says so,
// and holds it until told to exit. Run on its own, it does nothing.
#[test]
fn child_holds_the_repository_lock() {
    let (Some(repo), Some(dir)) = (
        std::env::var_os("APINATA_LOCK_CHILD_REPO"),
        std::env::var_os("APINATA_LOCK_CHILD_DIR"),
    ) else {
        return;
    };
    let dir = Path::new(&dir);
    let _lock = repo_lock(Path::new(&repo), Duration::ZERO, &GitOptions::new()).expect("free");
    std::fs::write(dir.join("held"), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !dir.join("release").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn_holder(sb: &Sandbox, repo: &Path) -> Child {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "git_repo_lock::child_holds_the_repository_lock",
        "--exact",
        "--test-threads=1",
    ])
    .envs(sb.env())
    .env("APINATA_LOCK_CHILD_REPO", repo)
    .env("APINATA_LOCK_CHILD_DIR", sb.path());
    for name in SCRUBBED {
        cmd.env_remove(name);
    }
    cmd.stdout(std::process::Stdio::null());
    cmd.spawn().unwrap()
}

#[test]
fn another_process_holding_the_lock_makes_this_one_busy_until_it_exits() {
    let sb = Sandbox::new();
    let repo = committed(&sb);
    let options = sb.options();
    let mut child = spawn_holder(&sb, &repo);
    let deadline = Instant::now() + Duration::from_secs(30);
    while !sb.path().join("held").exists() {
        assert!(Instant::now() < deadline, "the child never took the lock");
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(
        repo_lock(&repo, SHORT, &options).is_err(),
        "held by the child"
    );

    std::fs::write(sb.path().join("release"), "").unwrap();
    assert!(child.wait().unwrap().success());
    held(&repo, &options);
}
