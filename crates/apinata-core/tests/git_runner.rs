//! The git process contract, observed through a fake `git` first on `PATH`: fixed flags and
//! environment on every call, and the time limit.
#![cfg(unix)]

use crate::git_support;

use std::fs;
use std::time::{Duration, Instant};

use apinata_core::{GitError, GitState, collection_status_with, stage};
use git_support::Sandbox;

/// Records each call's argv and environment beside itself, answers `rev-parse` with `col/`
/// and `status` with the two branch headers of an empty repository.
const RECORDER: &str = r#"#!/bin/sh
dir=${0%/*}
for a in "$@"; do printf '%s\n' "$a"; done >> "$dir/args"
echo END >> "$dir/args"
env >> "$dir/env"
case "$*" in
  *rev-parse*) printf 'col/\n' ;;
  *status*) printf '# branch.oid (initial)\000'; printf '# branch.head main\000' ;;
esac
exit 0
"#;

fn values<'a>(env: &'a str, key: &str) -> Vec<&'a str> {
    env.lines()
        .filter_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .collect()
}

#[test]
fn every_call_carries_the_fixed_flags_and_environment() {
    let sb = Sandbox::new();
    let bin = sb.fake_git("rec", RECORDER);
    let root = sb.path().join("col");
    fs::create_dir_all(&root).unwrap();
    let options = sb
        .options()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("GIT_OPTIONAL_LOCKS", "1")
        .env("LC_ALL", "de_DE.UTF-8")
        .env("GIT_DIR", "/nowhere")
        .env("APINATA_PROBE", "seen");
    match collection_status_with(&root, &options).expect("status") {
        GitState::Repo(status) => {
            assert_eq!(status.branch.as_deref(), Some("main"));
            assert_eq!(status.head, None);
            assert!(status.files.is_empty());
        }
        other => panic!("{other:?}"),
    }

    let args = fs::read_to_string(bin.join("args")).unwrap();
    let calls: Vec<Vec<String>> = args
        .split("END\n")
        .filter(|call| !call.is_empty())
        .map(|call| call.lines().map(str::to_owned).collect())
        .collect();
    assert_eq!(
        calls,
        [
            vec![
                "--no-optional-locks",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "protocol.ext.allow=never",
                "rev-parse",
                "--show-prefix"
            ],
            vec![
                "--no-optional-locks",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "protocol.ext.allow=never",
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
                "--",
                ".",
            ],
        ]
    );

    let env = fs::read_to_string(bin.join("env")).unwrap();
    assert_eq!(values(&env, "GIT_OPTIONAL_LOCKS"), ["0", "0"]);
    assert_eq!(values(&env, "GIT_NO_LAZY_FETCH"), ["1", "1"]);
    assert_eq!(values(&env, "GIT_TERMINAL_PROMPT"), ["0", "0"]);
    assert_eq!(values(&env, "LC_ALL"), ["C", "C"]);
    assert_eq!(values(&env, "APINATA_PROBE"), ["seen", "seen"]);
    assert!(values(&env, "GIT_DIR").is_empty(), "{env}");
}

#[test]
fn a_hanging_git_is_stopped_at_the_time_limit() {
    let sb = Sandbox::new();
    // `sleep` is a grandchild that keeps the pipes open after the shell is killed.
    let bin = sb.fake_git("slow", "#!/bin/sh\nsleep 5\n");
    let root = sb.path().join("col");
    fs::create_dir_all(&root).unwrap();
    let options = sb
        .options()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .timeout(Duration::from_millis(300));
    let started = Instant::now();
    assert!(matches!(
        collection_status_with(&root, &options),
        Err(GitError::Timeout)
    ));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn the_identity_free_sandbox_sets_no_identity_variables_but_keeps_the_dates() {
    let names = |sb: &Sandbox| -> Vec<String> {
        sb.env()
            .into_iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect()
    };
    let with = names(&Sandbox::new());
    let without = names(&Sandbox::without_identity());
    for key in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
    ] {
        assert!(with.iter().any(|k| k == key), "{key}");
        assert!(!without.iter().any(|k| k == key), "{key}");
    }
    assert!(without.iter().any(|k| k == "GIT_AUTHOR_DATE"));
}

#[test]
fn a_write_call_carries_literal_pathspecs_and_the_same_environment() {
    let sb = Sandbox::new();
    let bin = sb.fake_git("rec", RECORDER);
    let root = sb.path().join("col");
    fs::create_dir_all(&root).unwrap();
    // On disk, so `stage` has no gone path to look up: its calls are the dry run, then `add`.
    for name in ["a b.yml", "*"] {
        fs::write(root.join(name), "x\n").unwrap();
    }
    let options = sb
        .options()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("GIT_TERMINAL_PROMPT", "1")
        .env("GIT_INDEX_FILE", "/nowhere");
    stage(&root, &["a b.yml", "*"], &options).expect("stage");

    let args = fs::read_to_string(bin.join("args")).unwrap();
    let lines: Vec<&str> = args.lines().collect();
    let call = |extra: &[&'static str]| {
        let mut line = vec![
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.ext.allow=never",
            "--literal-pathspecs",
            "add",
        ];
        line.extend(extra);
        line.extend(["-A", "--", "a b.yml", "*", "END"]);
        line
    };
    assert_eq!(lines, [call(&["-n"]), call(&[])].concat());
    let env = fs::read_to_string(bin.join("env")).unwrap();
    assert_eq!(values(&env, "GIT_TERMINAL_PROMPT"), ["0", "0"]);
    assert_eq!(values(&env, "GIT_OPTIONAL_LOCKS"), ["0", "0"]);
    assert!(values(&env, "GIT_INDEX_FILE").is_empty(), "{env}");
}
