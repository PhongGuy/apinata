//! The driver against a local bare origin: debounced commits, pushes, pulls, conflicts,
//! the read role, a secret refusal, offline, a refusing hook and flush.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_branch::cloned;
use crate::git_support::{Sandbox, write};
use crate::git_sync::rev;
use crate::secret_renames::{env, env_with_host, names, row};
use crate::team_auth::TOKEN;
use crate::team_rebase::teammate;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use apinata_core::team::ConflictCopies;
use apinata_core::team::{Block, Identity, Role, TeamAuth, TeamConfig, TeamState, TeamSync};
use apinata_core::{Collection, ConflictKind, MemoryStore, PullRange, SecretStore};

fn config(sb: &Sandbox, col: &Path, role: Role) -> TeamConfig {
    let copies = sb.path().join("copies").join(col.file_name().unwrap());
    TeamConfig::new(
        col.to_path_buf(),
        copies,
        TeamAuth::new("http://127.0.0.1:9", TOKEN),
        Identity::new("Ada", "ada@example.com"),
        role,
    )
    .base_options(sb.options())
    .debounce_unclamped(Duration::from_millis(50))
}

fn start(config: TeamConfig) -> TeamSync {
    TeamSync::start(config, Box::new(|_| {}))
}

fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let begun = Instant::now();
    while !done() {
        assert!(
            begun.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Git in the bare origin; `None` when it fails.
fn origin(sb: &Sandbox, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("--git-dir")
        .arg(sb.path().join("origin.git"))
        .args(args)
        .envs(sb.env())
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn origin_has(sb: &Sandbox, rel: &str, text: &str) -> bool {
    origin(sb, &["show", &format!("main:{rel}")]).is_some_and(|t| t.contains(text))
}

/// A second clone of origin at `<sandbox>/<name>`.
fn clone(sb: &Sandbox, name: &str) -> PathBuf {
    sb.git(
        sb.path(),
        &[
            "clone",
            "-q",
            sb.path().join("origin.git").to_str().unwrap(),
            name,
        ],
    );
    sb.path().join(name)
}

fn settled(sync: &TeamSync) -> bool {
    let s = sync.status();
    s.state == TeamState::Idle && s.unpushed == 0
}

#[test]
fn the_debounce_is_clamped_to_three_to_five_seconds() {
    let sb = Sandbox::new();
    let base = || config(&sb, &sb.path().join("x"), Role::Write);
    assert_eq!(
        base().debounce(Duration::from_secs(1)).debounce_time(),
        Duration::from_secs(3)
    );
    assert_eq!(
        base().debounce(Duration::from_secs(9)).debounce_time(),
        Duration::from_secs(5)
    );
    let fresh = TeamConfig::new(
        sb.path().into(),
        sb.path().into(),
        TeamAuth::new("http://127.0.0.1:9", TOKEN),
        Identity::new("Ada", "a@x"),
        Role::Write,
    );
    assert_eq!(fresh.debounce_time(), Duration::from_secs(4));
}

#[test]
fn a_change_is_committed_after_the_debounce_and_pushed_as_the_user() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let sync = start(config(&sb, &a, Role::Write));
    write(&a, "c.txt", "mine\n");
    sync.changed();
    wait("the push", || origin_has(&sb, "c.txt", "mine"));
    assert_eq!(
        origin(&sb, &["log", "-1", "--format=%an <%ae>", "main"]).unwrap(),
        "Ada <ada@example.com>\n"
    );
    wait("idle", || settled(&sync));
}

#[test]
fn a_remote_move_pulls_a_teammates_change() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let b = clone(&sb, "b");
    let (sa, sb_sync) = (
        start(config(&sb, &a, Role::Write)),
        start(config(&sb, &b, Role::Write)),
    );
    write(&a, "x.txt", "from a\n");
    sa.changed();
    wait("a's push", || origin_has(&sb, "x.txt", "from a"));
    sb_sync.remote_moved(None);
    wait("b's pull", || b.join("x.txt").is_file());
}

#[test]
fn a_conflict_writes_the_copy_and_pushes_the_rest() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let b = clone(&sb, "b");
    let sync = start(config(&sb, &a, Role::Write));
    wait("the start pull", || settled(&sync));
    write(&b, "a.txt", "theirs\n");
    sb.commit_all(&b, "theirs");
    sb.git(&b, &["push", "-q"]);
    write(&a, "a.txt", "mine\n");
    write(&a, "c.txt", "c\n");
    sync.changed();
    wait("the fallback push", || origin_has(&sb, "c.txt", "c"));
    wait("idle", || settled(&sync));
    let conflicts = sync.status().conflicts;
    assert_eq!(
        conflicts
            .iter()
            .map(|c| (c.path.as_str(), c.kind))
            .collect::<Vec<_>>(),
        [("a.txt", ConflictKind::BothChanged)]
    );
    let copy = sb.path().join("copies/col/a.txt");
    assert_eq!(std::fs::read_to_string(copy).unwrap(), "mine\n");
    assert!(origin_has(&sb, "a.txt", "theirs"));
}

#[test]
fn under_the_read_role_a_pull_never_pushes() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let b = clone(&sb, "b");
    write(&a, "kept.txt", "k\n");
    sb.commit_all(&a, "local");
    write(&b, "x.txt", "x\n");
    sb.commit_all(&b, "teammate");
    sb.git(&b, &["push", "-q"]);
    let sync = start(config(&sb, &a, Role::Read));
    wait("the start pull", || a.join("x.txt").is_file());
    std::thread::sleep(Duration::from_millis(300));
    assert!(!origin_has(&sb, "kept.txt", "k"));
    let status = sync.status();
    assert_eq!(
        (status.state, status.unpushed),
        (TeamState::ReadOnly { paused: false }, 1)
    );
}

#[cfg(unix)]
#[test]
fn a_secret_refusal_blocks_until_retry_squashes_it_away() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    sb.fake_tool("origin.git/hooks", "pre-receive", "#!/bin/sh\nwhile read old new ref; do\n  if git log -p \"$new\" --not --all | grep -q s3cr3t; then\n    echo \"potluck-reject secret_value $ref environments/dev.yml secret_value\" >&2\n    exit 1\n  fi\ndone\n");
    let sync = start(config(&sb, &a, Role::Write));
    write(&a, "environments/dev.yml", "value: s3cr3t\n");
    sync.changed();
    wait("the refusal", || {
        matches!(
            sync.status().state,
            TeamState::Blocked(Block::Refused { .. })
        )
    });
    let TeamState::Blocked(Block::Refused { findings, .. }) = sync.status().state else {
        unreachable!()
    };
    assert_eq!(findings[0].path, "environments/dev.yml");
    write(&a, "environments/dev.yml", "value: fixed\n");
    sync.changed();
    wait("the local fix commit", || sync.status().unpushed == 2);
    sync.retry();
    wait("the squashed push", || {
        origin_has(&sb, "environments/dev.yml", "fixed")
    });
    assert!(
        !origin(&sb, &["log", "-p", "main"])
            .unwrap()
            .contains("s3cr3t")
    );
}

#[test]
fn offline_commits_pile_up_and_push_once_a_poll_reaches_the_server() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let bare = sb.path().join("origin.git");
    sb.git(
        &a,
        &[
            "remote",
            "set-url",
            "origin",
            "http://127.0.0.1:1/git/t/w.git",
        ],
    );
    let sync = start(config(&sb, &a, Role::Write));
    wait("offline", || sync.status().state == TeamState::Offline);
    write(&a, "c.txt", "c\n");
    sync.changed();
    wait("one unpushed", || sync.status().unpushed == 1);
    assert_eq!(sync.status().state, TeamState::Offline);
    sb.git(&a, &["remote", "set-url", "origin", bare.to_str().unwrap()]);
    sync.poll();
    wait("the push", || origin_has(&sb, "c.txt", "c"));
}

#[cfg(unix)]
#[test]
fn a_refusing_commit_hook_blocks_until_it_is_fixed() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let hooks = sb.fake_tool(
        "dest/col/.git/hooks",
        "pre-commit",
        "#!/bin/sh\necho nope >&2\nexit 1\n",
    );
    let sync = start(config(&sb, &a, Role::Write));
    write(&a, "c.txt", "c\n");
    sync.changed();
    wait(
        "the hook block",
        || matches!(&sync.status().state, TeamState::Blocked(Block::Hook { output }) if output.contains("nope")),
    );
    std::fs::remove_file(hooks.join("pre-commit")).unwrap();
    sync.changed();
    wait("the push", || origin_has(&sb, "c.txt", "c"));
}

#[test]
fn a_notice_during_the_debounce_commits_before_it_pulls() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let b = clone(&sb, "b");
    let sync = start(config(&sb, &a, Role::Write).debounce_unclamped(Duration::from_secs(60)));
    wait("the start pull", || settled(&sync));
    write(&b, "x.txt", "x\n");
    sb.commit_all(&b, "teammate");
    sb.git(&b, &["push", "-q"]);
    write(&a, "c.txt", "c\n");
    sync.changed();
    sync.remote_moved(None);
    wait("the pull and the push", || {
        a.join("x.txt").is_file() && origin_has(&sb, "c.txt", "c")
    });
    wait("idle", || settled(&sync));
}

#[test]
fn flush_commits_and_pushes_without_waiting_for_the_debounce() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let sync = start(config(&sb, &a, Role::Write).debounce_unclamped(Duration::from_secs(60)));
    wait("the start pull", || settled(&sync));
    write(&a, "c.txt", "c\n");
    sync.changed();
    assert!(sync.flush(Duration::from_secs(20)));
    assert!(origin_has(&sb, "c.txt", "c"));
}

/// Inputs and saves fired from another thread while offline, where a pending flush commits as
/// the push fails and a save can land mid-pull, reach git one call at a time and still settle;
/// a wrapping git records any overlap.
#[cfg(unix)]
#[test]
fn git_calls_never_overlap_under_a_burst_of_inputs() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    sb.git(
        &a,
        &[
            "remote",
            "set-url",
            "origin",
            "http://127.0.0.1:1/git/t/w.git",
        ],
    );
    let real = crate::git_support::real_git();
    let body = format!(
        "#!/bin/sh\ndir=${{0%/*}}\nif [ -n \"$LOCKGIT_IN\" ]; then exec '{real}' \"$@\"; fi\nexport LOCKGIT_IN=1\nmkdir \"$dir/lock\" 2>/dev/null || echo \"$*\" >> \"$dir/overlaps\"\nsleep 0.02\n'{real}' \"$@\"\ncode=$?\nrmdir \"$dir/lock\" 2>/dev/null\nexit $code\n",
        real = real.display()
    );
    let bin = sb.fake_git("lockgit", &body);
    let base = sb
        .options()
        .env("PATH", crate::git_support::path_with(&bin));
    let sync = start(config(&sb, &a, Role::Write).base_options(base));
    for n in 0..15 {
        write(&a, &format!("c{n}.txt"), "c\n");
        sync.changed();
        sync.flush(Duration::from_millis(5));
        sync.retry();
        sync.poll();
        sync.remote_moved(None);
        std::thread::sleep(Duration::from_millis(30));
    }
    assert!(
        sync.flush(Duration::from_secs(60)),
        "settled: {:?}",
        sync.status()
    );
    assert!(
        sync.status().unpushed > 0,
        "the offline commits landed locally"
    );
    assert_eq!(
        std::fs::read_to_string(bin.join("overlaps")).unwrap_or_default(),
        ""
    );
}

#[test]
fn a_change_with_nothing_to_commit_does_not_push() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&seen);
    let sync = TeamSync::start(
        config(&sb, &a, Role::Write),
        Box::new(move |s| sink.lock().unwrap().push(s.state.clone())),
    );
    wait("the start pull", || settled(&sync));
    seen.lock().unwrap().clear();
    sync.changed();
    wait("the commit to settle", || {
        seen.lock().unwrap().contains(&TeamState::Committing) && settled(&sync)
    });
    let states = seen.lock().unwrap().clone();
    assert!(!states.contains(&TeamState::Pushing), "{states:?}");
}

#[test]
fn unanswered_pulls_accumulate_into_one_pending_range() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let sync = start(config(&sb, &a, Role::Write));
    wait("the start pull", || settled(&sync));
    assert_eq!(
        sync.pending_renames(),
        None,
        "a pull that moved nothing records no range"
    );
    let before = rev(&sb, &a, "HEAD");
    let first_tip = teammate(&sb, &[("x.txt", Some("x\n"))]);
    sync.remote_moved(None);
    wait("the first pull", || {
        rev(&sb, &a, "HEAD") == first_tip && settled(&sync)
    });
    let first = sync.pending_renames().expect("recorded");
    assert_eq!(first, PullRange::new(before.clone(), first_tip));
    let tip = teammate(&sb, &[("y.txt", Some("y\n"))]);
    sync.remote_moved(None);
    wait("the second pull", || {
        rev(&sb, &a, "HEAD") == tip && settled(&sync)
    });
    let both = PullRange::new(before, tip);
    assert_eq!(sync.pending_renames(), Some(both.clone()));
    assert!(
        !sync.renames_settled(&first),
        "a stale range clears nothing"
    );
    assert_eq!(sync.pending_renames(), Some(both.clone()));
    assert!(sync.renames_settled(&both));
    assert_eq!(sync.pending_renames(), None);
}

#[test]
fn a_rename_held_back_by_a_clash_survives_a_later_pull_and_is_offered_once_settled() {
    let sb = Sandbox::new();
    let a = cloned(&sb);
    let (dev, qa) = (env("dev", &["token"]), env("qa", &["key"]));
    teammate(
        &sb,
        &[
            ("environments/dev.yml", Some(dev.as_str())),
            ("environments/qa.yml", Some(qa.as_str())),
        ],
    );
    sb.git(&a, &["pull", "-q", "--ff-only"]);
    let cfg = config(&sb, &a, Role::Write);
    let copies = ConflictCopies::new(cfg.copies().to_path_buf());
    let sync = start(cfg);
    wait("the start pull", || settled(&sync));
    let c = Collection::open(&a).unwrap();
    let store = MemoryStore::new();
    store.set(&c.secret_key("dev", "token"), "t1").unwrap();
    store.set(&c.secret_key("qa", "key"), "k1").unwrap();

    // A local edit of dev clashes with the teammate's rename of it.
    let theirs = env_with_host("staging", &["token"], "theirs");
    teammate(
        &sb,
        &[
            ("environments/dev.yml", None),
            ("environments/staging.yml", Some(theirs.as_str())),
        ],
    );
    write(
        &a,
        "environments/dev.yml",
        &env_with_host("dev", &["token"], "mine"),
    );
    sync.changed();
    let dev_copy = "environments/dev.yml".to_owned();
    wait("the clash", || {
        copies.list().unwrap().contains(&dev_copy) && settled(&sync)
    });
    let held_range = sync.pending_renames().expect("the clash pull left a range");
    let c = Collection::open(&a).unwrap();
    let held = c
        .secret_renames(&held_range, Some(&copies), &store, &sb.options())
        .unwrap();
    assert_eq!((held.renames.len(), held.held_back), (0, 1));

    // A later pull renames qa's variable while the clash is still open.
    let renamed_qa = env("qa", &["qa_key"]);
    let tip = teammate(&sb, &[("environments/qa.yml", Some(renamed_qa.as_str()))]);
    sync.remote_moved(None);
    wait("the second pull", || {
        rev(&sb, &a, "HEAD") == tip && settled(&sync)
    });
    let merged = sync.pending_renames().expect("still pending");
    assert_eq!(merged, PullRange::new(held_range.from.clone(), tip));
    let c = Collection::open(&a).unwrap();
    let still = c
        .secret_renames(&merged, Some(&copies), &store, &sb.options())
        .unwrap();
    assert_eq!(names(&still), [row("qa/key", &["qa/qa_key"], false)]);
    assert_eq!(still.held_back, 1);

    copies.remove("environments/dev.yml").unwrap();
    let all = c
        .secret_renames(&merged, Some(&copies), &store, &sb.options())
        .unwrap();
    assert_eq!(all.held_back, 0);
    assert_eq!(
        names(&all)
            .into_iter()
            .map(|(f, t, _)| (f, t))
            .collect::<Vec<_>>(),
        [
            ("dev/token".to_owned(), vec!["staging/token".to_owned()]),
            ("qa/key".to_owned(), vec!["qa/qa_key".to_owned()]),
        ]
    );
}
