#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use crate::git_branch::cloned;
use crate::git_support;

use std::fs;
use std::path::Path;

use apinata_core::{
    Branch, GitError, GitOptions, RemoteBranch, SwitchError, Upstream, branches, commit,
    remote_branches, stage, switch_branch, switch_to_remote,
};
use git_support::{Sandbox, path_with, real_git, write};

fn find(sb: &Sandbox, root: &Path, name: &str) -> RemoteBranch {
    remote_branches(root, &sb.options())
        .unwrap()
        .into_iter()
        .find(|b| b.name == name)
        .unwrap()
}

fn head(sb: &Sandbox, root: &Path) -> String {
    sb.git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_owned()
}

fn tip(sb: &Sandbox, root: &Path, rev: &str) -> String {
    sb.git(root, &["rev-parse", rev]).trim().to_owned()
}

/// The short upstream of local branch `name`, empty when it tracks nothing.
fn upstream(sb: &Sandbox, root: &Path, name: &str) -> String {
    let full = format!("refs/heads/{name}");
    sb.git(root, &["for-each-ref", "--format=%(upstream:short)", &full])
        .trim()
        .to_owned()
}

fn has_branch(sb: &Sandbox, root: &Path, name: &str) -> bool {
    let full = format!("refs/heads/{name}");
    sb.git(root, &["for-each-ref", "--format=%(refname)", &full])
        .trim()
        == full
}

/// Every `branch.<name>.*` line of the repository config.
fn branch_keys(sb: &Sandbox, root: &Path, name: &str) -> Vec<String> {
    let prefix = format!("branch.{name}.");
    sb.git(root, &["config", "--local", "--list"])
        .lines()
        .filter(|l| l.starts_with(&prefix))
        .map(str::to_owned)
        .collect()
}

/// Options whose git is a script: a command line holding `on` runs `action` (with `$REAL` the
/// real git), anything else is the real git.
#[cfg(unix)]
fn wrapped(sb: &Sandbox, on: &str, action: &str) -> GitOptions {
    let body = format!(
        "#!/bin/sh\nREAL='{}'\ncase \"$*\" in *\"{on}\"*) {action} ;; *) exec \"$REAL\" \"$@\" ;; esac\n",
        real_git().display()
    );
    let bin = sb.fake_git("wrapped", &body);
    sb.options().env("PATH", path_with(&bin))
}

/// A switch refused by git without changing anything.
#[cfg(unix)]
const REFUSE: &str = "echo 'fatal: refused' >&2; exit 1";

#[test]
fn creates_a_branch_that_tracks_the_remote_one() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let feature = find(&sb, &root, "feature");
    let done = switch_to_remote(&root, &feature, &sb.options()).unwrap();
    assert_eq!(done.branch, "feature");
    assert_eq!(done.upstream, Upstream::Created);
    assert_eq!(head(&sb, &root), "feature");
    assert_eq!(
        sb.git(&root, &["rev-parse", "--abbrev-ref", "feature@{upstream}"])
            .trim(),
        "origin/feature"
    );
    assert!(root.join("b.txt").is_file());
}

#[test]
fn uses_the_existing_tracking_branch() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(
        &root,
        &["switch", "-q", "--track", "-c", "work", "origin/feature"],
    );
    sb.git(&root, &["switch", "-q", "main"]);
    let feature = find(&sb, &root, "feature");
    assert_eq!(feature.local.as_deref(), Some("work"));
    let done = switch_to_remote(&root, &feature, &sb.options()).unwrap();
    assert_eq!(done.branch, "work");
    assert_eq!(done.upstream, Upstream::Kept);
    assert_eq!(head(&sb, &root), "work");
}

#[test]
fn any_change_refuses_the_switch_and_leaves_files_alone() {
    type Setup = fn(&Sandbox, &Path);
    let cases: [(&str, Setup); 4] = [
        ("unstaged", |_, r| write(r, "a.txt", "changed\n")),
        ("staged", |sb, r| {
            write(r, "a.txt", "changed\n");
            sb.git(r, &["add", "a.txt"]);
        }),
        ("untracked", |_, r| write(r, "new.txt", "new\n")),
        ("outside the root", |_, r| write(r, "../zz.txt", "new\n")),
    ];
    for (name, setup) in cases {
        let sb = Sandbox::new();
        let repo = cloned(&sb);
        fs::create_dir(repo.join("sub")).unwrap();
        // The last case works from a subdirectory: the check covers the whole repository.
        let root = if name == "outside the root" {
            repo.join("sub")
        } else {
            repo.clone()
        };
        setup(&sb, &root);
        let before = fs::read_to_string(repo.join("a.txt")).unwrap();
        let feature = find(&sb, &repo, "feature");
        let r = switch_to_remote(&root, &feature, &sb.options());
        assert!(matches!(r, Err(SwitchError::Dirty)), "{name}: {r:?}");
        assert_eq!(head(&sb, &repo), "main", "{name}");
        assert_eq!(
            fs::read_to_string(repo.join("a.txt")).unwrap(),
            before,
            "{name}"
        );
    }
}

#[test]
fn a_same_named_local_branch_that_tracks_nothing_gets_the_upstream_and_keeps_its_tip() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "feature", "main"]);
    let feature = find(&sb, &root, "feature");
    assert_eq!(feature.local, None);
    let done = switch_to_remote(&root, &feature, &sb.options()).unwrap();
    assert_eq!(done.branch, "feature");
    assert_eq!(done.upstream, Upstream::Set);
    assert_eq!(head(&sb, &root), "feature");
    assert_eq!(upstream(&sb, &root, "feature"), "origin/feature");
    assert_eq!(tip(&sb, &root, "feature"), tip(&sb, &root, "main"));
}

#[test]
fn an_untracked_local_main_ahead_of_its_remote_gets_the_upstream_and_keeps_its_tip() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    write(&root, "c.txt", "c\n");
    sb.commit_all(&root, "ahead");
    sb.git(&root, &["branch", "--unset-upstream", "main"]);
    sb.git(
        &root,
        &["switch", "-q", "--track", "-c", "feature", "origin/feature"],
    );
    let ahead = tip(&sb, &root, "main");
    let main = find(&sb, &root, "main");
    assert_eq!(main.local, None);
    let done = switch_to_remote(&root, &main, &sb.options()).unwrap();
    assert_eq!(done.branch, "main");
    assert_eq!(done.upstream, Upstream::Set);
    assert_eq!(head(&sb, &root), "main");
    assert_eq!(upstream(&sb, &root, "main"), "origin/main");
    assert_eq!(tip(&sb, &root, "main"), ahead);
}

#[test]
fn a_same_named_local_branch_tracking_another_branch_is_left_as_is() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "--track", "feature", "origin/main"]);
    let feature = RemoteBranch::new("origin", "feature", None);
    let done = switch_to_remote(&root, &feature, &sb.options()).unwrap();
    assert_eq!(done.branch, "feature");
    assert_eq!(
        done.upstream,
        Upstream::Other {
            upstream: "origin/main".to_owned()
        }
    );
    assert_eq!(head(&sb, &root), "feature");
    assert_eq!(upstream(&sb, &root, "feature"), "origin/main");
    assert_eq!(tip(&sb, &root, "feature"), tip(&sb, &root, "origin/main"));
}

#[test]
fn an_option_shaped_name_never_starts_git() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let bad = RemoteBranch::new("origin", "--orphan", None);
    let r = switch_to_remote(&root, &bad, &sb.options());
    assert!(
        matches!(&r, Err(SwitchError::Failed { stderr }) if stderr.contains("branch name")),
        "{r:?}"
    );
}

/// A held lock refuses the switch before git writes anything: no branch, no branch config, the
/// tree untouched and the lock kept. Once it is gone the same switch goes through.
#[test]
fn a_held_lock_refuses_the_switch_and_leaves_nothing_behind() {
    for name in [
        "HEAD.lock",
        "index.lock",
        "refs/heads/feature.lock",
        "config.lock",
    ] {
        let sb = Sandbox::new();
        let root = cloned(&sb);
        let held = root.join(".git").join(name);
        fs::write(&held, "").unwrap();
        let feature = find(&sb, &root, "feature");
        match switch_to_remote(&root, &feature, &sb.options()) {
            Err(e @ SwitchError::Busy { .. }) => {
                let SwitchError::Busy { lock } = &e else {
                    unreachable!()
                };
                assert_eq!(
                    lock.canonicalize().unwrap(),
                    held.canonicalize().unwrap(),
                    "{name}"
                );
                let same = GitError::Busy { lock: lock.clone() };
                assert_eq!(e.to_string(), same.to_string(), "{name}");
            }
            other => panic!("{name}: expected Busy, got {other:?}"),
        }
        assert!(!has_branch(&sb, &root, "feature"), "{name}: branch left");
        assert_eq!(
            branch_keys(&sb, &root, "feature"),
            [] as [String; 0],
            "{name}"
        );
        assert_eq!(head(&sb, &root), "main", "{name}");
        assert_eq!(sb.git(&root, &["status", "--porcelain"]), "", "{name}");
        assert!(held.exists(), "{name} was deleted");
        fs::remove_file(&held).unwrap();
        let done = switch_to_remote(&root, &feature, &sb.options()).unwrap();
        assert_eq!(done.upstream, Upstream::Created, "{name}");
        assert_eq!(head(&sb, &root), "feature", "{name}");
        assert_eq!(upstream(&sb, &root, "feature"), "origin/feature", "{name}");
    }
}

#[test]
fn a_held_ref_lock_never_refuses_a_switch_to_an_existing_branch() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "--track", "feature", "origin/feature"]);
    fs::write(root.join(".git/refs/heads/feature.lock"), "").unwrap();
    let feature = find(&sb, &root, "feature");
    assert_eq!(feature.local.as_deref(), Some("feature"));
    let done = switch_to_remote(&root, &feature, &sb.options()).unwrap();
    assert_eq!(done.upstream, Upstream::Kept);
    assert_eq!(head(&sb, &root), "feature");
}

/// `git branch --track` can make the ref and then fail; the ref and its two tracking keys go,
/// while a key of the user's own in the same section stays.
#[cfg(unix)]
#[test]
fn a_branch_made_by_a_failed_track_is_rolled_back_but_user_keys_stay() {
    for description in [false, true] {
        let sb = Sandbox::new();
        let root = cloned(&sb);
        if description {
            sb.git(&root, &["config", "branch.feature.description", "mine"]);
        }
        let opts = wrapped(&sb, " branch --track ", "\"$REAL\" \"$@\"; exit 1");
        let r = switch_to_remote(&root, &find(&sb, &root, "feature"), &opts);
        assert!(matches!(r, Err(SwitchError::Failed { .. })), "{r:?}");
        assert!(!has_branch(&sb, &root, "feature"));
        let kept: Vec<String> = if description {
            vec!["branch.feature.description=mine".to_owned()]
        } else {
            Vec::new()
        };
        assert_eq!(branch_keys(&sb, &root, "feature"), kept);
        assert_eq!(head(&sb, &root), "main");
    }
}

/// A branch another process made at the same tip after the plan is not this call's to delete.
#[cfg(unix)]
#[test]
fn a_branch_made_meanwhile_by_another_process_is_never_rolled_back() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let action = "\"$REAL\" update-ref refs/heads/feature refs/remotes/origin/feature; \
                  exec \"$REAL\" \"$@\"";
    let opts = wrapped(&sb, " branch --track ", action);
    let r = switch_to_remote(&root, &find(&sb, &root, "feature"), &opts);
    assert!(matches!(r, Err(SwitchError::Failed { .. })), "{r:?}");
    assert!(has_branch(&sb, &root, "feature"));
    assert_eq!(
        tip(&sb, &root, "feature"),
        tip(&sb, &root, "origin/feature")
    );
    assert_eq!(head(&sb, &root), "main");
}

#[cfg(unix)]
#[test]
fn a_switch_that_landed_but_exited_non_zero_is_a_success() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let opts = wrapped(&sb, " switch ", "\"$REAL\" \"$@\"; exit 1");
    let done = switch_to_remote(&root, &find(&sb, &root, "feature"), &opts).unwrap();
    assert_eq!(done.upstream, Upstream::Created);
    assert_eq!(head(&sb, &root), "feature");
    assert_eq!(upstream(&sb, &root, "feature"), "origin/feature");
}

#[cfg(unix)]
#[test]
fn a_refused_switch_removes_the_branch_it_created() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let opts = wrapped(&sb, " switch ", REFUSE);
    let r = switch_to_remote(&root, &find(&sb, &root, "feature"), &opts);
    assert!(
        matches!(&r, Err(SwitchError::Failed { stderr }) if stderr.contains("refused")),
        "{r:?}"
    );
    assert!(!has_branch(&sb, &root, "feature"));
    assert_eq!(branch_keys(&sb, &root, "feature"), [] as [String; 0]);
    assert_eq!(head(&sb, &root), "main");
}

/// The rollback deletes the branch only at the tip it created it on, so a branch something
/// else moved meanwhile is kept.
#[cfg(unix)]
#[test]
fn the_rollback_never_deletes_a_branch_that_moved() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let action = format!("\"$REAL\" update-ref refs/heads/feature refs/heads/main; {REFUSE}");
    let opts = wrapped(&sb, " switch ", &action);
    let r = switch_to_remote(&root, &find(&sb, &root, "feature"), &opts);
    assert!(matches!(r, Err(SwitchError::Failed { .. })), "{r:?}");
    assert!(has_branch(&sb, &root, "feature"));
    assert_eq!(tip(&sb, &root, "feature"), tip(&sb, &root, "main"));
    assert_eq!(head(&sb, &root), "main");
}

#[cfg(unix)]
#[test]
fn a_refused_switch_removes_the_upstream_it_set() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "feature", "main"]);
    let opts = wrapped(&sb, " switch ", REFUSE);
    let r = switch_to_remote(&root, &find(&sb, &root, "feature"), &opts);
    assert!(matches!(r, Err(SwitchError::Failed { .. })), "{r:?}");
    assert!(has_branch(&sb, &root, "feature"));
    assert_eq!(tip(&sb, &root, "feature"), tip(&sb, &root, "main"));
    assert_eq!(upstream(&sb, &root, "feature"), "");
    assert_eq!(head(&sb, &root), "main");
}

/// The listed row of local branch `name`, or the remote-only row of `origin/<name>` when
/// `local` is false.
fn listed(sb: &Sandbox, root: &Path, name: &str, local: bool) -> Branch {
    branches(root, &sb.options())
        .unwrap()
        .into_iter()
        .find(|b| match (&b.local, &b.upstream) {
            (Some(l), _) if local => l == name,
            (None, Some(u)) if !local => u.name == name,
            _ => false,
        })
        .unwrap()
}

#[test]
fn switch_branch_to_a_local_only_branch_moves_head_and_writes_no_config() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let config = fs::read_to_string(root.join(".git/config")).unwrap();
    let done = switch_branch(&root, &listed(&sb, &root, "topic", true), &sb.options()).unwrap();
    assert_eq!(done.branch, "topic");
    assert_eq!(done.upstream, Upstream::Unchanged);
    assert_eq!(head(&sb, &root), "topic");
    assert_eq!(
        fs::read_to_string(root.join(".git/config")).unwrap(),
        config
    );
}

#[test]
fn switch_branch_to_a_local_branch_over_a_held_head_lock_is_busy() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    // topic holds a file main lacks, so a switch git began would change the tree.
    sb.git(&root, &["switch", "-q", "-c", "topic"]);
    write(&root, "t.txt", "t\n");
    sb.commit_all(&root, "topic");
    sb.git(&root, &["switch", "-q", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    let held = root.join(".git/HEAD.lock");
    fs::write(&held, "").unwrap();
    let r = switch_branch(&root, &topic, &sb.options());
    match r {
        Err(SwitchError::Busy { lock }) => {
            assert_eq!(lock.canonicalize().unwrap(), held.canonicalize().unwrap())
        }
        other => panic!("expected Busy, got {other:?}"),
    }
    assert_eq!(head(&sb, &root), "main");
    assert_eq!(sb.git(&root, &["status", "--porcelain"]), "");
    assert!(held.exists(), "HEAD.lock was deleted");
}

#[test]
fn switch_branch_to_a_local_branch_on_a_dirty_tree_is_refused() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    write(&root, "new.txt", "new\n");
    let r = switch_branch(&root, &topic, &sb.options());
    assert!(matches!(r, Err(SwitchError::Dirty)), "{r:?}");
    assert_eq!(head(&sb, &root), "main");
}

/// `rel` marked `add -N` and removed: a ` D` entry with nothing behind it in `HEAD`.
fn add_n_removed(sb: &Sandbox, root: &Path, rel: &str) {
    write(root, rel, "new\n");
    sb.git(root, &["add", "-N", rel]);
    fs::remove_file(root.join(rel)).unwrap();
}

#[test]
fn an_add_n_gone_from_disk_does_not_block_a_switch_and_stays_in_the_index() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    add_n_removed(&sb, &root, "n.yml");
    switch_branch(&root, &topic, &sb.options()).expect("switch");
    assert_eq!(head(&sb, &root), "topic");
    assert_eq!(sb.git(&root, &["status", "--porcelain"]), " D n.yml\n");
}

#[test]
fn a_moved_add_n_left_behind_by_a_commit_does_not_block_a_switch() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    write(&root, "n.yml", "new\n");
    sb.git(&root, &["add", "-N", "n.yml"]);
    fs::rename(root.join("n.yml"), root.join("m.yml")).unwrap();
    stage(&root, &["m.yml"], &sb.options()).expect("stage");
    commit(&root, "m", &sb.options()).expect("commit");
    switch_branch(&root, &topic, &sb.options()).expect("switch");
    assert_eq!(head(&sb, &root), "topic");
}

#[test]
fn an_add_n_on_an_unborn_branch_does_not_block_a_switch() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    sb.git(&root, &["checkout", "-q", "--orphan", "fresh"]);
    sb.git(&root, &["rm", "-rfq", "."]);
    add_n_removed(&sb, &root, "n.yml");
    switch_branch(&root, &topic, &sb.options()).expect("switch");
    assert_eq!(head(&sb, &root), "topic");
}

#[test]
fn a_deleted_tracked_empty_file_stays_dirty() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    write(&root, "empty.yml", "");
    sb.commit_all(&root, "empty");
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    fs::remove_file(root.join("empty.yml")).unwrap();
    let r = switch_branch(&root, &topic, &sb.options());
    assert!(matches!(r, Err(SwitchError::Dirty)), "{r:?}");
}

#[test]
fn an_add_n_beside_another_change_stays_dirty() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    add_n_removed(&sb, &root, "n.yml");
    write(&root, "new.txt", "new\n");
    let r = switch_branch(&root, &topic, &sb.options());
    assert!(matches!(r, Err(SwitchError::Dirty)), "{r:?}");
}

#[test]
fn a_subfolder_collection_lets_an_add_n_elsewhere_through() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    write(&root, "sub/keep.yml", "k\n");
    sb.commit_all(&root, "sub");
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    add_n_removed(&sb, &root, "other/n.yml");
    switch_branch(&root.join("sub"), &topic, &sb.options()).expect("switch");
    assert_eq!(head(&sb, &root), "topic");
}

#[test]
fn a_subfolder_collection_stays_dirty_on_a_tracked_deletion_elsewhere() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    write(&root, "sub/keep.yml", "k\n");
    write(&root, "other/empty.yml", "");
    sb.commit_all(&root, "sub");
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    fs::remove_file(root.join("other/empty.yml")).unwrap();
    let r = switch_branch(&root.join("sub"), &topic, &sb.options());
    assert!(matches!(r, Err(SwitchError::Dirty)), "{r:?}");
}

#[test]
fn a_target_holding_the_add_n_path_fails_with_gits_text() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["checkout", "-q", "-b", "topic"]);
    write(&root, "n.yml", "theirs\n");
    sb.commit_all(&root, "n");
    sb.git(&root, &["checkout", "-q", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    add_n_removed(&sb, &root, "n.yml");
    let r = switch_branch(&root, &topic, &sb.options());
    assert!(matches!(r, Err(SwitchError::Failed { .. })), "{r:?}");
    assert_eq!(head(&sb, &root), "main");
}

#[cfg(unix)]
#[test]
fn a_failed_head_lookup_stays_dirty() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    add_n_removed(&sb, &root, "n.yml");
    let r = switch_branch(&root, &topic, &wrapped(&sb, "ls-tree", REFUSE));
    assert!(matches!(r, Err(SwitchError::Dirty)), "{r:?}");
}

#[test]
fn switch_branch_to_a_branch_checked_out_in_another_worktree_fails() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    let other = sb.path().join("other-worktree");
    sb.git(
        &root,
        &["worktree", "add", "-q", other.to_str().unwrap(), "topic"],
    );
    let r = switch_branch(&root, &topic, &sb.options());
    assert!(matches!(r, Err(SwitchError::Failed { .. })), "{r:?}");
    assert_eq!(head(&sb, &root), "main");
}

#[test]
fn switch_branch_to_a_deleted_local_branch_fails_by_name() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    sb.git(&root, &["branch", "topic", "main"]);
    let topic = listed(&sb, &root, "topic", true);
    sb.git(&root, &["branch", "-D", "topic"]);
    match switch_branch(&root, &topic, &sb.options()) {
        Err(SwitchError::Failed { stderr }) => {
            assert_eq!(stderr, "no local branch named topic")
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn switch_branch_on_a_remote_only_row_creates_a_tracking_branch() {
    let sb = Sandbox::new();
    let root = cloned(&sb);
    let feature = listed(&sb, &root, "feature", false);
    let done = switch_branch(&root, &feature, &sb.options()).unwrap();
    assert_eq!(done.branch, "feature");
    assert_eq!(done.upstream, Upstream::Created);
    assert_eq!(head(&sb, &root), "feature");
    assert_eq!(upstream(&sb, &root, "feature"), "origin/feature");
}
