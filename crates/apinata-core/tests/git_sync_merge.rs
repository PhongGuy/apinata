//! The fast-forward step: autostash off, the post-fetch lock check, and `Partial`.
use crate::git_branch::cloned;
use crate::git_support::Sandbox;
use crate::git_sync::{advance, clean, logged, rev, wrapped};

use std::fs;

use apinata_core::{CancelFlag, SyncError, pull};

#[test]
fn merge_auto_stash_in_config_is_turned_off() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["config", "merge.autoStash", "true"]);
    let tip = advance(&sb, "c.txt");
    let (opts, log) = wrapped(&sb, "wrap", "merge", ":", ":");
    assert_eq!(pull(&root, &CancelFlag::new(), &opts).unwrap().to, tip);
    let text = logged(&log);
    let merges: Vec<&str> = text.lines().filter(|l| l.contains(" merge ")).collect();
    assert_eq!(merges.len(), 1, "{merges:?}");
    assert!(
        merges[0].contains("--ff-only") && merges[0].contains("--no-autostash"),
        "{merges:?}"
    );
    assert!(sb.git(&root, &["stash", "list"]).is_empty());
}

#[test]
fn a_file_created_in_the_way_during_the_fetch_is_failed_not_partial() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let head = rev(&sb, &root, "HEAD");
    advance(&sb, "c.txt");
    let after = format!("printf 'mine\\n' > '{}'", root.join("c.txt").display());
    let (opts, _) = wrapped(&sb, "wrap", "fetch", ":", &after);
    let r = pull(&root, &CancelFlag::new(), &opts);
    assert!(
        matches!(&r, Err(SyncError::Failed { stderr }) if stderr.contains("untracked")),
        "{r:?}"
    );
    assert_eq!(fs::read_to_string(root.join("c.txt")).unwrap(), "mine\n");
    assert_eq!(rev(&sb, &root, "HEAD"), head);
    sb.git(&root, &["diff", "--cached", "--quiet"]);
}

#[test]
fn a_lock_taken_during_the_fetch_is_busy_and_kept() {
    for name in ["index.lock", "HEAD.lock", "refs/heads/main.lock"] {
        let sb = Sandbox::new();
        let root = cloned(&sb);
        let head = rev(&sb, &root, "HEAD");
        advance(&sb, "c.txt");
        let held = root.join(".git").join(name);
        let after = format!(": > '{}'", held.display());
        let (opts, log) = wrapped(&sb, "wrap", "fetch", ":", &after);
        match pull(&root, &CancelFlag::new(), &opts) {
            Err(SyncError::Busy { lock }) => {
                assert_eq!(
                    lock.canonicalize().unwrap(),
                    held.canonicalize().unwrap(),
                    "{name}"
                )
            }
            other => panic!("{name}: expected Busy, got {other:?}"),
        }
        assert!(held.exists(), "{name} was deleted");
        // Refused before the merge runs, not by the merge itself failing on the lock.
        assert!(!logged(&log).contains(" merge "), "{name}");
        assert_eq!(rev(&sb, &root, "HEAD"), head, "{name}");
        assert!(!root.join("c.txt").exists(), "{name}");
    }
}

#[test]
fn a_branch_lock_taken_as_the_merge_starts_is_partial_and_reset_merge_recovers() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let head = rev(&sb, &root, "HEAD");
    advance(&sb, "c.txt");
    let held = root.join(".git/refs/heads/main.lock");
    let before = format!(": > '{}'", held.display());
    let (opts, _) = wrapped(&sb, "wrap", "merge", &before, ":");
    match pull(&root, &CancelFlag::new(), &opts) {
        Err(SyncError::Partial { upstream, .. }) => assert_eq!(upstream, "origin/main"),
        other => panic!("expected Partial, got {other:?}"),
    }
    assert_eq!(rev(&sb, &root, "HEAD"), head);
    assert!(root.join("c.txt").is_file());
    assert!(held.exists());
    fs::remove_file(&held).unwrap();
    sb.git(&root, &["reset", "--merge", "HEAD"]);
    assert!(clean(&sb, &root));
    assert!(!root.join("c.txt").exists());
}
