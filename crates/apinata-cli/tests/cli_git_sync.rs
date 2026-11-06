use crate::cli_git_clone::{cli, cloned, err_text, out_text};
use crate::git_support::{Sandbox, write};

use std::path::Path;

/// Commit `file` on origin's `main` from the seed work tree and return the new tip.
pub fn advance(sb: &Sandbox, file: &str) -> String {
    let seed = sb.path().join("seed");
    sb.git(&seed, &["switch", "-q", "main"]);
    write(&seed, file, &format!("{file}\n"));
    sb.commit_all(&seed, file);
    sb.git(&seed, &["push", "-q", "origin", "main"]);
    head(sb, &seed)
}

pub fn head(sb: &Sandbox, repo: &Path) -> String {
    sb.git(repo, &["rev-parse", "HEAD"]).trim().to_owned()
}

pub fn local_commit(sb: &Sandbox, repo: &Path, file: &str) {
    write(repo, file, &format!("{file}\n"));
    sb.commit_all(repo, file);
}

/// Run `apinata <args>` in `cwd`, require exit 3, and return stderr.
pub fn fails(sb: &Sandbox, cwd: &Path, args: &[&str]) -> String {
    err_text(&cli(sb, cwd).args(args).assert().code(3))
}

#[test]
fn pull_fast_forwards_and_prints_the_range() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let before = head(&sb, &col);
    let tip = advance(&sb, "c.txt");
    let out = cli(&sb, &col).args(["git", "pull"]).assert().code(0);
    assert_eq!(
        out_text(&out),
        format!(
            "Fast-forwarded main to origin/main ({}..{})\n",
            &before[..7],
            &tip[..7]
        )
    );
    assert_eq!(head(&sb, &col), tip);
}

#[test]
fn pull_with_nothing_new_says_up_to_date_and_counts_local_commits() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let out = cli(&sb, &col).args(["git", "pull"]).assert().code(0);
    assert_eq!(out_text(&out), "main is up to date with origin/main\n");
    assert!(!err_text(&out).contains("ahead"));
    local_commit(&sb, &col, "d.txt");
    let err = err_text(&cli(&sb, &col).args(["git", "pull"]).assert().code(0));
    assert!(
        err.contains("note: main is 1 commit ahead of origin/main\n"),
        "{err}"
    );
    local_commit(&sb, &col, "e.txt");
    let err = err_text(&cli(&sb, &col).args(["git", "pull"]).assert().code(0));
    assert!(
        err.contains("note: main is 2 commits ahead of origin/main\n"),
        "{err}"
    );
}

#[test]
fn pull_json_is_the_result_and_the_collection_flag_works() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let before = head(&sb, &col);
    let tip = advance(&sb, "c.txt");
    let out = cli(&sb, sb.path())
        .args(["git", "pull", "--json", "--collection"])
        .arg(&col)
        .assert()
        .code(0);
    let v: serde_json::Value = serde_json::from_str(&out_text(&out)).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"branch": "main", "upstream": "origin/main", "from": before, "to": tip, "ahead": 0})
    );
}

#[test]
fn push_prints_the_range_or_up_to_date() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    let before = head(&sb, &col);
    local_commit(&sb, &col, "d.txt");
    let after = head(&sb, &col);
    let out = cli(&sb, &col).args(["git", "push"]).assert().code(0);
    assert_eq!(
        out_text(&out),
        format!(
            "Pushed main to origin/main ({}..{})\n",
            &before[..7],
            &after[..7]
        )
    );
    let out = cli(&sb, &col).args(["git", "push"]).assert().code(0);
    assert_eq!(out_text(&out), "origin/main is up to date\n");
}

#[test]
fn push_set_upstream_publishes_a_new_branch_text_and_json() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    sb.git(&col, &["switch", "-q", "-c", "topic"]);
    local_commit(&sb, &col, "t.txt");
    let out = cli(&sb, &col)
        .args(["git", "push", "--set-upstream", "origin"])
        .assert()
        .code(0);
    assert_eq!(
        out_text(&out),
        "Pushed topic to origin/topic (new branch); topic now tracks origin/topic\n"
    );
    sb.git(&col, &["switch", "-q", "-c", "next"]);
    local_commit(&sb, &col, "n.txt");
    let tip = head(&sb, &col);
    let out = cli(&sb, &col)
        .args(["git", "push", "--set-upstream", "origin", "--json"])
        .assert()
        .code(0);
    let v: serde_json::Value = serde_json::from_str(&out_text(&out)).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"branch": "next", "upstream": "origin/next", "from": null, "to": tip, "upstream_set": true})
    );
}

#[test]
fn refusals_and_a_missing_collection_exit_three() {
    let sb = Sandbox::new();
    let col = cloned(&sb);
    write(&col, "new.txt", "x\n");
    assert!(fails(&sb, &col, &["git", "pull"]).contains("commit or discard your changes"));
    std::fs::remove_file(col.join("new.txt")).unwrap();
    let err = fails(&sb, &col, &["git", "push", "--set-upstream", "nope"]);
    assert!(err.contains("nope is not a configured remote"), "{err}");
    sb.git(&col, &["switch", "-q", "--detach"]);
    assert!(fails(&sb, &col, &["git", "pull"]).contains("detached"));
    let plain = sb.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    fails(&sb, &plain, &["git", "pull"]);
    fails(&sb, &plain, &["git", "push"]);
}
