//! Pins that a flag may sit anywhere among a command's positionals: before, between and after.
//! `replace`, `search` and `ls` take optional leading collection words, which must not stop
//! clap from accepting a flag between two of them.

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

fn collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("first.yml", &get("first", 1, "http://127.0.0.1:9/a", "")),
        (
            "users/get.yml",
            &get("get-user", 1, "http://127.0.0.1:9/u", ""),
        ),
    ])
}

fn empty() -> tempfile::TempDir {
    write_tree(&[("note.txt", "x")])
}

/// Every argument order that keeps `words` in order and puts each of `flags` (a flag with its
/// value, if any) in any gap, before, between or after the words.
fn orders<'a>(command: &'a str, words: &[&'a str], flags: &[Vec<&'a str>]) -> Vec<Vec<&'a str>> {
    let gaps = words.len() + 1;
    let total = gaps.pow(u32::try_from(flags.len()).expect("few flags"));
    (0..total)
        .map(|mut pick| {
            let mut slots: Vec<Vec<&str>> = vec![Vec::new(); gaps];
            for f in flags {
                slots[pick % gaps].extend(f.iter().copied());
                pick /= gaps;
            }
            let mut args = vec![command];
            for (i, slot) in slots.iter().enumerate() {
                args.extend(slot.iter().copied());
                if let Some(w) = words.get(i) {
                    args.push(w);
                }
            }
            args
        })
        .collect()
}

/// Runs every order from `cwd` and asserts each one exits 0 with `want` on stdout.
fn all_match(orders: Vec<Vec<&str>>, cwd: &Path, want: &str) {
    for args in orders {
        let r = ran(&args, cwd);
        assert_eq!(r.code, 0, "{args:?}: {}", r.stderr);
        assert_eq!(r.stdout, want, "{args:?}");
    }
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

fn replace_flags() -> Vec<Vec<&'static str>> {
    vec![
        vec!["--dry-run"],
        vec!["--json"],
        vec!["--regex"],
        vec!["--case"],
    ]
}

fn replace_baseline(col: &Path) -> String {
    let base = ran(
        &[
            "replace",
            "get-user",
            "x",
            "--dry-run",
            "--json",
            "--regex",
            "--case",
        ],
        col,
    );
    assert_eq!(base.code, 0, "{}", base.stderr);
    assert!(base.stdout.contains("get-user"), "{}", base.stdout);
    base.stdout
}

#[test]
fn replace_takes_flags_around_pattern_and_replacement() {
    let col = collection();
    let want = replace_baseline(col.path());
    all_match(
        orders("replace", &["get-user", "x"], &replace_flags()),
        col.path(),
        &want,
    );
}

#[test]
fn replace_takes_the_collection_word_with_flags_between() {
    let col = collection();
    let elsewhere = empty();
    let c = s(col.path());
    let want = replace_baseline(col.path());
    all_match(
        orders("replace", &[c, "get-user", "x"], &replace_flags()),
        elsewhere.path(),
        &want,
    );
}

#[test]
fn replace_takes_the_collection_flag_in_every_position() {
    let col = collection();
    let elsewhere = empty();
    let c = s(col.path());
    let want = replace_baseline(col.path());
    let dry = || {
        vec![
            vec!["--dry-run"],
            vec!["--json"],
            vec!["--regex"],
            vec!["--case"],
            vec!["--collection", c],
        ]
    };
    for words in [vec!["get-user", "x"], vec![c, "get-user", "x"]] {
        all_match(orders("replace", &words, &dry()), elsewhere.path(), &want);
    }
}

#[test]
fn replace_names_a_conflicting_collection_instead_of_a_count_error() {
    let col = collection();
    let other = collection();
    let elsewhere = empty();
    let r = ran(
        &[
            "replace",
            s(col.path()),
            "--collection",
            s(other.path()),
            "get-user",
            "x",
            "--dry-run",
        ],
        elsewhere.path(),
    );
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(r.stderr.contains("named twice"), "{}", r.stderr);
}

#[test]
fn replace_takes_double_dash_before_the_words() {
    let col = collection();
    let want = replace_baseline(col.path());
    let r = ran(
        &[
            "replace",
            "--dry-run",
            "--json",
            "--regex",
            "--case",
            "--",
            "get-user",
            "x",
        ],
        col.path(),
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, want);
}

#[test]
fn search_takes_flags_around_the_collection_and_query() {
    let col = collection();
    let elsewhere = empty();
    let c = s(col.path());
    let want = ran(&["search", "get-user", "--json"], col.path());
    assert_eq!(want.code, 0, "{}", want.stderr);
    assert!(want.stdout.contains("get-user"), "{}", want.stdout);
    all_match(
        orders("search", &["get-user"], &[vec!["--json"]]),
        col.path(),
        &want.stdout,
    );
    all_match(
        orders("search", &[c, "get-user"], &[vec!["--json"]]),
        elsewhere.path(),
        &want.stdout,
    );
    let with_flag = [vec!["--json"], vec!["--collection", c]];
    all_match(
        orders("search", &["get-user"], &with_flag),
        elsewhere.path(),
        &want.stdout,
    );
    all_match(
        orders("search", &[c, "get-user"], &with_flag),
        elsewhere.path(),
        &want.stdout,
    );
    let dash = ran(&["search", "--json", "--", "get-user"], col.path());
    assert_eq!(dash.stdout, want.stdout, "{}", dash.stderr);
}

#[test]
fn ls_takes_flags_around_the_collection_and_folder() {
    let col = collection();
    let elsewhere = empty();
    let c = s(col.path());
    let want = ran(&["ls", "users", "--json", "--tags"], col.path());
    assert_eq!(want.code, 0, "{}", want.stderr);
    assert!(want.stdout.contains("get-user"), "{}", want.stdout);
    let flags = [vec!["--json"], vec!["--tags"]];
    all_match(orders("ls", &["users"], &flags), col.path(), &want.stdout);
    all_match(
        orders("ls", &[c, "users"], &flags),
        elsewhere.path(),
        &want.stdout,
    );
    let with_flag = [vec!["--json"], vec!["--tags"], vec!["--collection", c]];
    for words in [vec!["users"], vec![c, "users"]] {
        all_match(
            orders("ls", &words, &with_flag),
            elsewhere.path(),
            &want.stdout,
        );
    }
}
