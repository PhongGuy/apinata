//! The app's view of a collection's git status: letters and words, conversion from core, and
//! what a refresh leaves behind.

use crate::git_support;
use crate::support;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use apinata_app::git_status::{
    GitChange, GitChanges, GitStage, QUIET_NOTE, after_refresh, commit_failure_text, commit_label,
    item_file, with_identity_hint,
};
use apinata_core::{
    CommitError, FileStatus, GitError, GitState, StageState, collection_status_with,
};
use git_support::{Sandbox, write};
use support::*;

fn one_modified() -> Option<Arc<GitChanges>> {
    let mut g = GitChanges::default();
    g.files.insert(PathBuf::from("a.yml"), GitChange::Modified);
    Some(Arc::new(g))
}

#[test]
fn each_change_has_a_letter_a_word_and_a_description() {
    let renamed = GitChange::Renamed {
        from: PathBuf::from("old.yml"),
    };
    let all = [
        (
            GitChange::Modified,
            "M",
            "modified",
            "Modified since last commit",
        ),
        (GitChange::Added, "A", "added", "Added since last commit"),
        (
            GitChange::Deleted,
            "D",
            "deleted",
            "Deleted since last commit",
        ),
        (GitChange::Untracked, "U", "untracked", "Untracked"),
        (renamed, "R", "renamed", "Renamed since last commit"),
        (GitChange::Conflicted, "C", "conflicted", "Conflicted"),
    ];
    for (change, letter, word, words) in all {
        assert_eq!(
            (change.letter(), change.word(), change.description()),
            (letter, word, words)
        );
    }
}

#[test]
fn core_statuses_map_one_to_one() {
    let from = PathBuf::from("a.yml");
    let pairs = [
        (FileStatus::Modified, GitChange::Modified),
        (FileStatus::Added, GitChange::Added),
        (FileStatus::Deleted, GitChange::Deleted),
        (FileStatus::Untracked, GitChange::Untracked),
        (FileStatus::Conflicted, GitChange::Conflicted),
        (
            FileStatus::Renamed { from: from.clone() },
            GitChange::Renamed { from },
        ),
    ];
    for (core, app) in pairs {
        assert_eq!(GitChange::from_core(&core), app);
    }
}

#[test]
fn the_quiet_note_names_both_causes_in_words() {
    assert_eq!(
        QUIET_NOTE,
        "Changes are not available: git is missing or this collection is not in a repository."
    );
}

#[test]
fn a_row_reads_its_own_file_and_a_folder_its_folder_yml() {
    assert_eq!(item_file(&ip("users/list.yml"), false), "users/list.yml");
    assert_eq!(item_file(&ip("users"), true), "users/folder.yml");
    assert_eq!(item_file(&ip("a.yml"), false), "a.yml");
}

#[test]
fn the_count_names_changed_files_and_the_cap() {
    let mut g = GitChanges::default();
    assert_eq!((g.count_label(), g.truncated_note()), (None, None));
    g.files.insert(PathBuf::from("a.yml"), GitChange::Modified);
    g.files.insert(PathBuf::from("b.yml"), GitChange::Untracked);
    assert_eq!(g.count_label().as_deref(), Some("2 changed"));
    g.truncated = true;
    assert_eq!(g.count_label().as_deref(), Some("2+ changed"));
    assert_eq!(
        g.truncated_note().as_deref(),
        Some("Showing the first 5,000 changed files")
    );
    assert_eq!(g.get("a.yml"), Some(&GitChange::Modified));
    assert_eq!(g.get("c.yml"), None);
}

#[test]
fn a_timeout_keeps_the_last_state_and_anything_else_quiet_clears_it() {
    let prev = one_modified();
    assert_eq!(after_refresh(prev.clone(), Err(GitError::Timeout)), prev);
    assert_eq!(after_refresh(prev.clone(), Ok(GitState::NotARepo)), None);
    assert_eq!(after_refresh(prev.clone(), Ok(GitState::Unavailable)), None);
    let refused = GitError::Failed {
        command: "git status".into(),
        stderr: "detected dubious ownership".into(),
    };
    assert_eq!(after_refresh(prev.clone(), Err(refused)), None);
    let gone = GitError::Io(io::Error::new(io::ErrorKind::NotFound, "collection root"));
    assert_eq!(after_refresh(prev.clone(), Err(gone)), None);
    assert_eq!(
        after_refresh(prev, Err(GitError::Malformed("x".into()))),
        None
    );
}

#[test]
fn a_repository_state_becomes_collection_relative_changes() {
    let sb = Sandbox::new();
    let repo = sb.repo("r");
    write(&repo, "col/opencollection.yml", ROOT);
    write(&repo, "col/a.yml", &request("a", "http://x/a"));
    write(&repo, "col/users/folder.yml", &folder("Users"));
    sb.commit_all(&repo, "init");
    write(&repo, "col/a.yml", &request("a", "http://x/changed"));
    write(&repo, "col/new.yml", &request("n", "http://x/n"));
    std::fs::remove_file(repo.join("col/users/folder.yml")).unwrap();
    let state = collection_status_with(&repo.join("col"), &sb.options()).unwrap();
    let changes = GitChanges::from_state(&state).expect("a repository");
    assert_eq!(
        changes.files.into_iter().collect::<Vec<_>>(),
        vec![
            (PathBuf::from("a.yml"), GitChange::Modified),
            (PathBuf::from("new.yml"), GitChange::Untracked),
            (PathBuf::from("users/folder.yml"), GitChange::Deleted),
        ]
    );
    assert!(!changes.truncated);
    assert_eq!(GitChanges::from_state(&GitState::NotARepo), None);
    assert_eq!(GitChanges::from_state(&GitState::Unavailable), None);
}

#[test]
fn core_stage_states_map_one_to_one() {
    assert_eq!(GitStage::from_core(&StageState::Staged), GitStage::Staged);
    assert_eq!(
        GitStage::from_core(&StageState::Unstaged),
        GitStage::Unstaged
    );
    assert_eq!(GitStage::from_core(&StageState::Both), GitStage::Both);
    assert_eq!(
        GitStage::from_core(&StageState::StagedGone),
        GitStage::StagedGone
    );
}

#[test]
fn a_staged_file_gone_from_disk_counts_as_staged() {
    let changes = GitChanges {
        stage: [(PathBuf::from("n.yml"), GitStage::StagedGone)].into(),
        ..GitChanges::default()
    };
    assert_eq!(changes.staged_count(), 1);
}

#[test]
fn the_commit_button_names_the_branch() {
    assert_eq!(commit_label(Some("main")), "Commit to main");
    assert_eq!(commit_label(None), "Commit");
}

#[test]
fn an_unknown_identity_gets_a_hint_and_other_output_is_untouched() {
    for text in [
        "Author identity unknown\n*** Please tell me who you are.",
        "fatal: empty ident name (for <x@y>) not allowed",
    ] {
        let out = with_identity_hint(text);
        assert!(out.starts_with(text));
        assert!(out.contains("git config user.name"));
        assert!(out.contains("git config user.email"));
    }
    assert_eq!(with_identity_hint("hook says no"), "hook says no");
}

#[test]
fn commit_failures_read_as_plain_sentences() {
    assert_eq!(
        commit_failure_text(&CommitError::EmptyMessage),
        "The commit message is empty."
    );
    assert_eq!(
        commit_failure_text(&CommitError::NothingStaged),
        "Nothing is staged."
    );
    assert_eq!(
        commit_failure_text(&CommitError::Failed {
            output: "hook says no".into()
        }),
        "hook says no"
    );
    let outside = commit_failure_text(&CommitError::StagedOutside {
        paths: vec![PathBuf::from("x/a.yml"), PathBuf::from("x/b.yml")],
        count: 5,
    });
    assert!(outside.starts_with("Files outside this collection are staged."));
    assert!(outside.contains("\nx/a.yml\nx/b.yml"));
    assert!(outside.ends_with("and 3 more"));
}
