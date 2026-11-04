//! Pins every collection form the commands accept today, so naming the collection a second way
//! never changes what an existing invocation means.

use crate::common;

use std::path::Path;

use common::{ROOT, apinata, get, write_tree};

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

fn ran(args: &[&str], cwd: &Path) -> Ran {
    let out = apinata()
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("runs");
    Ran {
        code: out.status.code().expect("exit code"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A collection with one request at the root and one in `users/`.
fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("first.yml", &get("first", 1, "http://127.0.0.1:9/a", "")),
        (
            "users/get.yml",
            &get("get-user", 1, "http://127.0.0.1:9/u", ""),
        ),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: http://x.test\n",
        ),
    ])
}

fn empty() -> tempfile::TempDir {
    write_tree(&[("note.txt", "x")])
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

#[test]
fn run_takes_the_collection_as_its_positional() {
    let col = collection();
    let elsewhere = empty();
    let r = ran(
        &["run", s(col.path()), "--format", "json"],
        elsewhere.path(),
    );
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert!(r.stdout.contains("first"), "{}", r.stdout);
    assert!(r.stdout.contains("get-user"), "{}", r.stdout);
}

#[test]
fn run_takes_a_folder_and_a_request_file_as_its_positional() {
    let col = collection();
    let elsewhere = empty();
    let folder = col.path().join("users");
    let r = ran(&["run", s(&folder), "--format", "json"], elsewhere.path());
    assert!(r.stdout.contains("get-user"), "{}", r.stdout);
    assert!(!r.stdout.contains("\"first\""), "{}", r.stdout);
    let file = col.path().join("first.yml");
    let r = ran(&["run", s(&file), "--format", "json"], elsewhere.path());
    assert!(r.stdout.contains("first"), "{}", r.stdout);
    assert!(!r.stdout.contains("get-user"), "{}", r.stdout);
}

#[test]
fn run_without_a_path_is_a_usage_error() {
    let col = collection();
    let r = ran(&["run"], col.path());
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(r.stderr.contains("PATH"), "{}", r.stderr);
}

#[test]
fn run_of_a_missing_path_exits_3() {
    let dir = empty();
    let r = ran(&["run", "nope"], dir.path());
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("cannot open"), "{}", r.stderr);
}

#[test]
fn run_outside_a_collection_names_the_directory() {
    let dir = empty();
    let r = ran(&["run", s(dir.path())], dir.path());
    assert_eq!(r.code, 3);
    assert!(
        r.stderr.contains("no opencollection.yml in")
            && r.stderr.contains("or any parent directory"),
        "{}",
        r.stderr
    );
}

#[test]
fn check_takes_a_positional_and_defaults_to_the_working_directory() {
    let col = collection();
    let elsewhere = empty();
    let by_arg = ran(&["check", s(col.path()), "--json"], elsewhere.path());
    let by_cwd = ran(&["check", "--json"], col.path());
    assert_eq!(by_arg.code, 0, "{}", by_arg.stderr);
    assert_eq!(by_arg.code, by_cwd.code);
    assert_eq!(by_arg.stdout, by_cwd.stdout);
}

#[test]
fn check_outside_a_collection_exits_3() {
    let dir = empty();
    let r = ran(&["check"], dir.path());
    assert_eq!(r.code, 3);
    assert!(
        r.stderr.contains("no opencollection.yml in"),
        "{}",
        r.stderr
    );
}

#[test]
fn mock_requires_its_positional_and_rejects_a_missing_path() {
    let dir = empty();
    assert_eq!(ran(&["mock"], dir.path()).code, 3);
    let r = ran(&["mock", "nope"], dir.path());
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("cannot open"), "{}", r.stderr);
}

#[test]
fn git_status_takes_a_positional_and_defaults_to_the_working_directory() {
    let col = collection();
    let elsewhere = empty();
    let by_arg = ran(
        &["git", "status", s(col.path()), "--json"],
        elsewhere.path(),
    );
    let by_cwd = ran(&["git", "status", "--json"], col.path());
    assert_eq!(by_arg.code, by_cwd.code, "{}", by_arg.stderr);
    assert_eq!(by_arg.stdout, by_cwd.stdout);
}

#[test]
fn git_status_outside_a_collection_exits_3() {
    let dir = empty();
    let r = ran(&["git", "status"], dir.path());
    assert_eq!(r.code, 3);
    assert!(
        r.stderr.contains("no opencollection.yml in"),
        "{}",
        r.stderr
    );
}

#[test]
fn git_branches_and_switch_take_the_flag() {
    let col = collection();
    let elsewhere = empty();
    let c = s(col.path());
    let b = ran(&["git", "branches", "--collection", c], elsewhere.path());
    let b_cwd = ran(&["git", "branches"], col.path());
    assert_eq!(b.code, b_cwd.code, "{}", b.stderr);
    assert_eq!(b.stdout, b_cwd.stdout);
    let w = ran(
        &["git", "switch", "main", "--collection", c],
        elsewhere.path(),
    );
    let w_cwd = ran(&["git", "switch", "main"], col.path());
    assert_eq!(w.code, w_cwd.code, "{}", w.stderr);
}

#[test]
fn git_branches_outside_a_collection_exits_3() {
    let dir = empty();
    let r = ran(&["git", "branches"], dir.path());
    assert_eq!(r.code, 3);
    assert!(
        r.stderr.contains("no opencollection.yml in"),
        "{}",
        r.stderr
    );
}

#[test]
fn ls_lists_the_working_directory_collection_a_folder_and_the_flag_form() {
    let col = collection();
    let elsewhere = empty();
    let c = s(col.path());
    let root = ran(&["ls", "--json"], col.path());
    assert_eq!(root.code, 0, "{}", root.stderr);
    assert!(root.stdout.contains("first.yml"), "{}", root.stdout);
    let folder = ran(&["ls", "users"], col.path());
    assert_eq!(folder.code, 0, "{}", folder.stderr);
    assert!(folder.stdout.contains("get-user"), "{}", folder.stdout);
    assert!(!folder.stdout.contains("first"), "{}", folder.stdout);
    let flag = ran(&["ls", "--collection", c, "--json"], elsewhere.path());
    assert_eq!(flag.stdout, root.stdout);
    let both = ran(&["ls", "users", "--collection", c], elsewhere.path());
    assert_eq!(both.stdout, folder.stdout);
}

#[test]
fn ls_outside_a_collection_exits_3() {
    let dir = empty();
    let r = ran(&["ls"], dir.path());
    assert_eq!(r.code, 3);
    assert!(
        r.stderr.contains("no opencollection.yml in"),
        "{}",
        r.stderr
    );
}

#[test]
fn search_takes_the_query_and_the_flag() {
    let col = collection();
    let elsewhere = empty();
    let cwd = ran(&["search", "get-user", "--json"], col.path());
    assert_eq!(cwd.code, 0, "{}", cwd.stderr);
    assert!(cwd.stdout.contains("get-user"), "{}", cwd.stdout);
    let flag = ran(
        &[
            "search",
            "get-user",
            "--collection",
            s(col.path()),
            "--json",
        ],
        elsewhere.path(),
    );
    assert_eq!(flag.stdout, cwd.stdout);
}

#[test]
fn search_outside_a_collection_exits_3() {
    let dir = empty();
    let r = ran(&["search", "x"], dir.path());
    assert_eq!(r.code, 3);
    assert!(
        r.stderr.contains("no opencollection.yml in"),
        "{}",
        r.stderr
    );
}

#[test]
fn replace_takes_pattern_replacement_and_the_flag() {
    let col = collection();
    let elsewhere = empty();
    let cwd = ran(
        &["replace", "get-user", "x", "--dry-run", "--json"],
        col.path(),
    );
    assert_eq!(cwd.code, 0, "{}", cwd.stderr);
    let flag = ran(
        &[
            "replace",
            "get-user",
            "x",
            "--dry-run",
            "--json",
            "--collection",
            s(col.path()),
        ],
        elsewhere.path(),
    );
    assert_eq!(flag.stdout, cwd.stdout);
}

#[test]
fn replace_outside_a_collection_exits_3() {
    let dir = empty();
    let r = ran(&["replace", "a", "b", "--dry-run"], dir.path());
    assert_eq!(r.code, 3);
    assert!(
        r.stderr.contains("no opencollection.yml in"),
        "{}",
        r.stderr
    );
}

#[test]
fn mcp_outside_a_collection_exits_3() {
    let dir = empty();
    let r = ran(&["mcp"], dir.path());
    assert_eq!(r.code, 3);
    assert!(
        r.stderr.contains("no opencollection.yml in"),
        "{}",
        r.stderr
    );
}

#[test]
fn export_takes_its_collection_as_a_required_positional() {
    let col = collection();
    let elsewhere = empty();
    let r = ran(
        &["export", s(col.path()), "--format", "postman", "--out", "-"],
        elsewhere.path(),
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("get-user"), "{}", r.stdout);
    let none = ran(&["export", "--format", "postman", "--out", "-"], col.path());
    assert_eq!(none.code, 3);
}

#[test]
fn env_and_trust_take_only_the_flag() {
    let col = collection();
    let elsewhere = empty();
    let c = s(col.path());
    let env_cwd = ran(&["env", "ls"], col.path());
    assert_eq!(env_cwd.code, 0, "{}", env_cwd.stderr);
    assert!(env_cwd.stdout.contains("dev"), "{}", env_cwd.stdout);
    let env_flag = ran(&["env", "ls", "--collection", c], elsewhere.path());
    assert_eq!(env_flag.stdout, env_cwd.stdout);
    let trust_file = elsewhere.path().join("trust.json");
    let tf = s(&trust_file);
    let t = ran(
        &["trust", "list", "--collection", c, "--trust-file", tf],
        elsewhere.path(),
    );
    assert_eq!(t.code, 0, "{}", t.stderr);
}

#[test]
fn env_and_trust_outside_a_collection_exit_3() {
    let dir = empty();
    let tf = dir.path().join("t.json");
    let e = ran(&["env", "ls"], dir.path());
    assert_eq!(e.code, 3);
    assert!(
        e.stderr.contains("no opencollection.yml in"),
        "{}",
        e.stderr
    );
    let t = ran(&["trust", "list", "--trust-file", s(&tf)], dir.path());
    assert_eq!(t.code, 3);
    assert!(
        t.stderr.contains("no opencollection.yml in"),
        "{}",
        t.stderr
    );
}
