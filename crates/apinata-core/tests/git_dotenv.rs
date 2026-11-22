//! Keeping a collection's `.env` out of git: git's own ignore rules decide, a missing rule is
//! appended to the collection's `.gitignore`, and a tracked `.env` is reported, never rewritten.

use crate::common;
use crate::git_support;

use apinata_core::{Collection, DotenvGit, ignore_dotenv};
use common::write_tree;
use git_support::{Sandbox, write};
use std::fs;
use std::path::Path;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

// A repository at `<sandbox>/repo` holding a collection at `<repo>/col` with a `.env`.
fn repo_with_collection(sb: &Sandbox) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = sb.repo("repo");
    let col = repo.join("col");
    write(&col, "opencollection.yml", ROOT);
    write(&col, ".env", "API_KEY=abc\n");
    (repo, col)
}

fn ignored_by_git(sb: &Sandbox, col: &Path) -> bool {
    !sb.git(
        col,
        &["status", "--porcelain", "--untracked-files=all", "--", "."],
    )
    .contains(".env")
}

#[test]
fn a_fresh_repository_gains_the_rule_once() {
    let sb = Sandbox::new();
    let (_repo, col) = repo_with_collection(&sb);
    assert!(!ignored_by_git(&sb, &col), "git offers .env before");
    assert_eq!(
        ignore_dotenv(&col, &sb.options()).unwrap(),
        DotenvGit::Added
    );
    assert_eq!(
        fs::read_to_string(col.join(".gitignore")).unwrap(),
        ".env\n"
    );
    assert!(ignored_by_git(&sb, &col), "git ignores .env after");
    assert_eq!(
        ignore_dotenv(&col, &sb.options()).unwrap(),
        DotenvGit::Ignored
    );
    assert_eq!(
        fs::read_to_string(col.join(".gitignore")).unwrap(),
        ".env\n"
    );
}

#[test]
fn a_rule_elsewhere_in_the_repository_is_enough() {
    let sb = Sandbox::new();
    let (repo, col) = repo_with_collection(&sb);
    write(&repo, ".gitignore", "*.env\n.env*\n");
    assert_eq!(
        ignore_dotenv(&col, &sb.options()).unwrap(),
        DotenvGit::Ignored
    );
    assert!(!col.join(".gitignore").exists(), "nothing is written");
}

#[test]
fn a_literal_line_undone_by_a_later_rule_still_gains_one() {
    let sb = Sandbox::new();
    let (_repo, col) = repo_with_collection(&sb);
    write(&col, ".gitignore", ".env\r\n!.env\r\n");
    assert_eq!(
        ignore_dotenv(&col, &sb.options()).unwrap(),
        DotenvGit::Added
    );
    assert_eq!(
        fs::read_to_string(col.join(".gitignore")).unwrap(),
        ".env\r\n!.env\r\n.env\r\n"
    );
    assert!(ignored_by_git(&sb, &col));
}

#[test]
fn a_tracked_dotenv_is_reported_and_nothing_is_written() {
    let sb = Sandbox::new();
    let (repo, col) = repo_with_collection(&sb);
    sb.commit_all(&repo, "init");
    assert_eq!(
        ignore_dotenv(&col, &sb.options()).unwrap(),
        DotenvGit::Tracked
    );
    assert!(!col.join(".gitignore").exists());
}

#[test]
fn outside_a_repository_nothing_is_written() {
    let sb = Sandbox::new();
    let col = sb.path().join("plain");
    write(&col, "opencollection.yml", ROOT);
    write(&col, ".env", "API_KEY=abc\n");
    assert_eq!(
        ignore_dotenv(&col, &sb.options()).unwrap(),
        DotenvGit::Outside
    );
    assert!(!col.join(".gitignore").exists());
}

#[test]
fn without_git_nothing_is_written() {
    let sb = Sandbox::new();
    let (_repo, col) = repo_with_collection(&sb);
    let options = sb.options().env("PATH", sb.empty_bin());
    assert_eq!(ignore_dotenv(&col, &options).unwrap(), DotenvGit::Outside);
    assert!(!col.join(".gitignore").exists());
}

#[test]
fn a_collection_write_asks_git_first_inside_a_repository() {
    let sb = Sandbox::new();
    let (repo, col) = repo_with_collection(&sb);
    write(&repo, ".gitignore", ".env*\n");
    let c = Collection::open(&col).expect("opens");
    assert!(!c.ensure_dotenv_ignored_with(&sb.options()).unwrap());
    assert!(
        !col.join(".gitignore").exists(),
        "the parent rule is enough"
    );
    fs::remove_file(repo.join(".gitignore")).unwrap();
    assert!(c.ensure_dotenv_ignored_with(&sb.options()).unwrap());
    assert_eq!(
        fs::read_to_string(col.join(".gitignore")).unwrap(),
        ".env\n"
    );
}

#[test]
fn a_collection_write_outside_a_repository_still_names_dotenv() {
    let sb = Sandbox::new();
    let dir = write_tree(&[("opencollection.yml", ROOT)]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.ensure_dotenv_ignored_with(&sb.options()).unwrap());
    assert_eq!(
        fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
        ".env\n"
    );
}
