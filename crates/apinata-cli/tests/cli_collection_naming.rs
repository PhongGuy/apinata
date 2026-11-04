//! Every collection-taking command accepts the collection as a positional and as `--collection`,
//! refuses two different ones, and says which forms it takes when none is found.

use crate::common;

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;

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
        .timeout(std::time::Duration::from_secs(30))
        .output()
        .expect("runs");
    Ran {
        code: out.status.code().expect("exit code"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

const EXAMPLES: &str = "examples:\n  - name: ok\n    response:\n      status: 200\n";

/// A collection whose requests carry `tag` in their names, so two of them differ.
fn collection(tag: &str) -> tempfile::TempDir {
    let first = get(&format!("first-{tag}"), 1, "http://127.0.0.1:9/a", "") + EXAMPLES;
    let user = get(&format!("user-{tag}"), 1, "http://127.0.0.1:9/u", "");
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("first.yml", &first),
        ("users/get.yml", &user),
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

/// The conflict error names both values and says to give one.
fn assert_conflict(r: &Ran, a: &Path, b: &Path) {
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(r.stderr.contains(s(a)), "{}", r.stderr);
    assert!(r.stderr.contains(s(b)), "{}", r.stderr);
    assert!(r.stderr.contains("give it once"), "{}", r.stderr);
}

fn assert_hint(r: &Ran, form: &str) {
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(
        r.stderr.contains("no opencollection.yml in"),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains(form), "wanted `{form}` in: {}", r.stderr);
}

// run

#[test]
fn run_accepts_the_flag_like_the_positional() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let flag = ran(
        &["run", "--collection", c, "--format", "json"],
        elsewhere.path(),
    );
    let pos = ran(&["run", c, "--format", "json"], elsewhere.path());
    assert_eq!(flag.code, pos.code, "{}", flag.stderr);
    assert_eq!(flag.code, 2, "{}", flag.stderr);
    assert!(flag.stdout.contains("first-a"), "{}", flag.stdout);
    assert!(flag.stdout.contains("user-a"), "{}", flag.stdout);
}

#[test]
fn run_refuses_two_different_collections_and_accepts_the_same_twice() {
    let a = collection("a");
    let b = collection("b");
    let r = ran(&["run", s(a.path()), "--collection", s(b.path())], a.path());
    assert_conflict(&r, a.path(), b.path());
    let same = ran(
        &[
            "run",
            s(a.path()),
            "--collection",
            s(a.path()),
            "--format",
            "json",
        ],
        b.path(),
    );
    assert_eq!(same.code, 2, "{}", same.stderr);
}

#[test]
fn run_with_neither_form_names_both() {
    let dir = empty();
    let r = ran(&["run"], dir.path());
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("apinata run <PATH>"), "{}", r.stderr);
    assert!(r.stderr.contains("--collection <DIR>"), "{}", r.stderr);
}

#[test]
fn run_outside_a_collection_hints_both_forms() {
    let dir = empty();
    let r = ran(&["run", "--collection", s(dir.path())], dir.path());
    assert_hint(&r, "--collection <DIR>");
    let r = ran(&["run", s(dir.path())], dir.path());
    assert_hint(&r, "apinata run <PATH>");
}

// check

#[test]
fn check_accepts_the_flag_like_the_positional() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let flag = ran(&["check", "--collection", c, "--json"], elsewhere.path());
    let pos = ran(&["check", c, "--json"], elsewhere.path());
    assert_eq!(flag.code, 0, "{}", flag.stderr);
    assert_eq!(flag.stdout, pos.stdout);
}

#[test]
fn check_refuses_two_different_collections() {
    let a = collection("a");
    let b = collection("b");
    let r = ran(
        &["check", s(a.path()), "--collection", s(b.path())],
        a.path(),
    );
    assert_conflict(&r, a.path(), b.path());
}

#[test]
fn check_outside_a_collection_hints_both_forms() {
    let dir = empty();
    assert_hint(&ran(&["check"], dir.path()), "--collection <DIR>");
}

// mock

/// Start `apinata mock <args>` and return its first line, then stop it through its own handle.
fn mock_first_line(args: &[&str], cwd: &Path) -> String {
    let mut child = common::apinata_std()
        .arg("mock")
        .args(args)
        .args(["--port", "0"])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let mut out = BufReader::new(child.stdout.take().expect("stdout"));
    let mut line = String::new();
    out.read_line(&mut line).expect("first line");
    child.kill().expect("kill");
    child.wait().expect("wait");
    line
}

#[test]
fn mock_accepts_the_flag_like_the_positional() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let flag = mock_first_line(&["--collection", c], elsewhere.path());
    assert!(flag.starts_with("Listening on http://127.0.0.1:"), "{flag}");
}

#[test]
fn mock_refuses_two_different_collections() {
    let a = collection("a");
    let b = collection("b");
    let r = ran(
        &["mock", s(a.path()), "--collection", s(b.path())],
        a.path(),
    );
    assert_conflict(&r, a.path(), b.path());
}

#[test]
fn mock_with_neither_form_names_both() {
    let dir = empty();
    let r = ran(&["mock"], dir.path());
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("apinata mock <PATH>"), "{}", r.stderr);
    assert!(r.stderr.contains("--collection <DIR>"), "{}", r.stderr);
}

#[test]
fn mock_outside_a_collection_hints_both_forms() {
    let dir = empty();
    assert_hint(
        &ran(&["mock", "--collection", s(dir.path())], dir.path()),
        "--collection <DIR>",
    );
}

// git

#[test]
fn git_status_accepts_the_flag_like_the_positional() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let flag = ran(
        &["git", "status", "--collection", c, "--json"],
        elsewhere.path(),
    );
    let pos = ran(&["git", "status", c, "--json"], elsewhere.path());
    assert_eq!(flag.code, pos.code, "{}", flag.stderr);
    assert_eq!(flag.stdout, pos.stdout);
    assert!(!flag.stdout.is_empty() || flag.code == 0);
}

#[test]
fn git_status_and_branches_refuse_two_different_collections() {
    let a = collection("a");
    let b = collection("b");
    for sub in ["status", "branches"] {
        let r = ran(
            &["git", sub, s(a.path()), "--collection", s(b.path())],
            a.path(),
        );
        assert_conflict(&r, a.path(), b.path());
    }
}

#[test]
fn git_branches_accepts_the_positional_like_the_flag() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let pos = ran(&["git", "branches", c], elsewhere.path());
    let flag = ran(&["git", "branches", "--collection", c], elsewhere.path());
    assert_eq!(pos.code, flag.code, "{}", pos.stderr);
    assert_eq!(pos.stdout, flag.stdout);
    assert_eq!(pos.stderr, flag.stderr);
}

#[test]
fn git_commands_outside_a_collection_hint_their_forms() {
    let dir = empty();
    assert_hint(&ran(&["git", "status"], dir.path()), "--collection <DIR>");
    assert_hint(&ran(&["git", "branches"], dir.path()), "--collection <DIR>");
    assert_hint(
        &ran(&["git", "switch", "x"], dir.path()),
        "--collection <DIR>",
    );
}

// ls

#[test]
fn ls_accepts_a_leading_collection_and_the_reproduction_works() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let lead = ran(&["ls", c], elsewhere.path());
    let flag = ran(&["ls", "--collection", c], elsewhere.path());
    assert_eq!(lead.code, 0, "{}", lead.stderr);
    assert_eq!(lead.stdout, flag.stdout);
    let both = ran(&["ls", c, "users"], elsewhere.path());
    let flag_folder = ran(&["ls", "users", "--collection", c], elsewhere.path());
    assert_eq!(both.code, 0, "{}", both.stderr);
    assert_eq!(both.stdout, flag_folder.stdout);
    assert!(both.stdout.contains("user-a"), "{}", both.stdout);
    assert!(!both.stdout.contains("first-a"), "{}", both.stdout);
}

#[test]
fn ls_leading_directory_stays_a_folder_when_the_cwd_collection_has_it() {
    // cwd holds a collection with a folder `col`, and `col` is itself collection-like.
    let cwd = write_tree(&[
        ("opencollection.yml", ROOT),
        ("col/opencollection.yml", ROOT),
        (
            "col/inner.yml",
            &get("inner", 1, "http://127.0.0.1:9/i", ""),
        ),
        ("top.yml", &get("top", 1, "http://127.0.0.1:9/t", "")),
    ]);
    let as_folder = ran(&["ls", "col"], cwd.path());
    assert_eq!(as_folder.code, 0, "{}", as_folder.stderr);
    assert!(as_folder.stdout.contains("inner"), "{}", as_folder.stdout);
    assert!(!as_folder.stdout.contains("top"), "{}", as_folder.stdout);
}

#[test]
fn ls_leading_directory_is_the_collection_when_the_cwd_collection_lacks_it() {
    let cwd = write_tree(&[
        ("opencollection.yml", ROOT),
        ("top.yml", &get("top", 1, "http://127.0.0.1:9/t", "")),
        ("col/opencollection.yml", ROOT),
        (
            "col/inner.yml",
            &get("inner", 1, "http://127.0.0.1:9/i", ""),
        ),
    ]);
    let other = collection("b");
    let r = ran(&["ls", s(other.path())], cwd.path());
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.contains("user-b") || r.stdout.contains("users/"),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("top"), "{}", r.stdout);
}

#[test]
fn ls_with_the_flag_keeps_a_leading_collection_like_directory_as_a_folder() {
    let col = collection("a");
    let other = collection("b");
    // today's meaning: the positional is a folder; it does not exist, so this fails.
    let r = ran(
        &["ls", s(other.path()), "--collection", s(col.path())],
        col.path(),
    );
    assert_eq!(r.code, 3, "{}", r.stdout);
}

#[test]
fn ls_two_positionals_and_the_flag_must_agree() {
    let a = collection("a");
    let b = collection("b");
    let r = ran(
        &["ls", s(a.path()), "users", "--collection", s(b.path())],
        a.path(),
    );
    assert_conflict(&r, a.path(), b.path());
    let same = ran(
        &["ls", s(a.path()), "users", "--collection", s(a.path())],
        b.path(),
    );
    assert_eq!(same.code, 0, "{}", same.stderr);
}

#[test]
fn ls_outside_a_collection_hints_both_forms() {
    let dir = empty();
    assert_hint(&ran(&["ls"], dir.path()), "apinata ls [COLLECTION] [PATH]");
}

// search and replace

#[test]
fn search_accepts_a_leading_collection_and_the_reproduction_works() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let lead = ran(&["search", c, "user-a", "--json"], elsewhere.path());
    let flag = ran(
        &["search", "user-a", "--collection", c, "--json"],
        elsewhere.path(),
    );
    assert_eq!(lead.code, 0, "{}", lead.stderr);
    assert!(lead.stdout.contains("user-a"), "{}", lead.stdout);
    assert_eq!(lead.stdout, flag.stdout);
}

#[test]
fn search_refuses_two_different_collections() {
    let a = collection("a");
    let b = collection("b");
    let r = ran(
        &["search", s(a.path()), "x", "--collection", s(b.path())],
        a.path(),
    );
    assert_conflict(&r, a.path(), b.path());
}

#[test]
fn search_outside_a_collection_hints_both_forms() {
    let dir = empty();
    assert_hint(
        &ran(&["search", "x"], dir.path()),
        "apinata search [COLLECTION] <QUERY>",
    );
}

#[test]
fn replace_accepts_a_leading_collection() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let lead = ran(
        &["replace", c, "user-a", "z", "--dry-run", "--json"],
        elsewhere.path(),
    );
    let flag = ran(
        &[
            "replace",
            "user-a",
            "z",
            "--dry-run",
            "--json",
            "--collection",
            c,
        ],
        elsewhere.path(),
    );
    assert_eq!(lead.code, 0, "{}", lead.stderr);
    assert!(lead.stdout.contains("user-a"), "{}", lead.stdout);
    assert_eq!(lead.stdout, flag.stdout);
}

#[test]
fn replace_refuses_two_different_collections() {
    let a = collection("a");
    let b = collection("b");
    let r = ran(
        &[
            "replace",
            s(a.path()),
            "x",
            "y",
            "--dry-run",
            "--collection",
            s(b.path()),
        ],
        a.path(),
    );
    assert_conflict(&r, a.path(), b.path());
}

#[test]
fn replace_outside_a_collection_hints_both_forms() {
    let dir = empty();
    assert_hint(
        &ran(&["replace", "a", "b", "--dry-run"], dir.path()),
        "apinata replace [COLLECTION] <PATTERN> <REPLACEMENT>",
    );
}

// mcp

#[test]
fn mcp_accepts_the_positional_like_the_flag() {
    let elsewhere = empty();
    let missing = s(&elsewhere.path().join("nope")).to_owned();
    let pos = ran(&["mcp", &missing], elsewhere.path());
    let flag = ran(&["mcp", "--collection", &missing], elsewhere.path());
    assert_eq!(pos.code, 3, "{}", pos.stderr);
    assert!(pos.stderr.contains("cannot open"), "{}", pos.stderr);
    assert_eq!(pos.stderr, flag.stderr);
    let col = collection("a");
    let c = s(col.path());
    let pos = ran(&["mcp", c], elsewhere.path());
    let flag = ran(&["mcp", "--collection", c], elsewhere.path());
    assert_eq!(pos.code, flag.code, "{}", pos.stderr);
    assert_eq!(pos.stdout, flag.stdout);
}

#[test]
fn mcp_refuses_two_different_collections() {
    let a = collection("a");
    let b = collection("b");
    let r = ran(&["mcp", s(a.path()), "--collection", s(b.path())], a.path());
    assert_conflict(&r, a.path(), b.path());
}

#[test]
fn mcp_outside_a_collection_hints_both_forms() {
    let dir = empty();
    assert_hint(&ran(&["mcp"], dir.path()), "apinata mcp [COLLECTION]");
}

// export

#[test]
fn export_accepts_the_flag_like_the_positional() {
    let col = collection("a");
    let elsewhere = empty();
    let c = s(col.path());
    let flag = ran(
        &[
            "export",
            "--collection",
            c,
            "--format",
            "postman",
            "--out",
            "-",
        ],
        elsewhere.path(),
    );
    let pos = ran(
        &["export", c, "--format", "postman", "--out", "-"],
        elsewhere.path(),
    );
    assert_eq!(flag.code, 0, "{}", flag.stderr);
    assert_eq!(flag.stdout, pos.stdout);
}

#[test]
fn export_refuses_two_different_collections_and_needs_one() {
    let a = collection("a");
    let b = collection("b");
    let r = ran(
        &[
            "export",
            s(a.path()),
            "--collection",
            s(b.path()),
            "--format",
            "postman",
            "--out",
            "-",
        ],
        a.path(),
    );
    assert_conflict(&r, a.path(), b.path());
    let none = ran(&["export", "--format", "postman", "--out", "-"], a.path());
    assert_eq!(none.code, 3);
    assert!(
        none.stderr.contains("apinata export <COLLECTION>"),
        "{}",
        none.stderr
    );
}

#[test]
fn export_outside_a_collection_hints_both_forms() {
    let dir = empty();
    let r = ran(
        &["export", s(dir.path()), "--format", "postman", "--out", "-"],
        dir.path(),
    );
    assert_hint(&r, "apinata export <COLLECTION>");
}

// env and trust keep only the flag, and say so

#[test]
fn env_and_trust_hint_the_flag() {
    let dir = empty();
    let tf = dir.path().join("t.json");
    assert_hint(
        &ran(&["env", "ls"], dir.path()),
        "pass the collection with --collection <DIR>",
    );
    assert_hint(
        &ran(&["trust", "list", "--trust-file", s(&tf)], dir.path()),
        "pass the collection with --collection <DIR>",
    );
}

// completions

fn completions(shell: &str) -> String {
    let out = ran(&["completions", shell], Path::new("."));
    assert_eq!(out.code, 0);
    out.stdout
}

/// The text from the first `start` to the next `end` after it.
fn between<'a>(script: &'a str, start: &str, end: &str) -> &'a str {
    let from = script.find(start).unwrap_or_else(|| panic!("no `{start}`"));
    let rest = &script[from..];
    &rest[..rest.find(end).unwrap_or(rest.len())]
}

#[test]
fn every_shell_lists_the_collection_flag_on_each_command() {
    // (fish matcher, bash case label, zsh case label) per command.
    let cases: [(&str, &str, &str); 6] = [
        ("run", "apinata__subcmd__run)", "(run)"),
        ("check", "apinata__subcmd__check)", "(check)"),
        ("mock", "apinata__subcmd__mock)", "(mock)"),
        ("mcp", "apinata__subcmd__mcp)", "(mcp)"),
        ("export", "apinata__subcmd__export)", "(export)"),
        (
            "git; and __fish_seen_subcommand_from status",
            "apinata__subcmd__git__subcmd__status)",
            "(status)",
        ),
    ];
    let (fish, bash, zsh) = (completions("fish"), completions("bash"), completions("zsh"));
    for (f, b, z) in cases {
        let needle = format!("__fish_apinata_using_subcommand {f}\"");
        assert!(
            fish.lines()
                .any(|l| l.contains(&needle) && l.contains("-l collection")),
            "fish lacks --collection for {f}"
        );
        assert!(
            between(&bash, b, ";;").contains("--collection"),
            "bash lacks --collection for {b}"
        );
        assert!(
            between(&zsh, z, ";;").contains("--collection"),
            "zsh lacks --collection for {z}"
        );
    }
}
