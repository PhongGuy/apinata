//! `apinata env orphans` and `forget`: flags, output and exit codes around an empty store.

use crate::common;

use common::{ROOT, apinata, write_tree};
use std::path::Path;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\n"),
    ])
}

/// The command's exit code, stdout and stderr.
fn run(dir: &Path, profile: &Path, args: &[&str]) -> (i32, String, String) {
    let out = apinata()
        .env("APINATA_TEST_NO_KEYCHAIN", "1")
        .env("APINATA_PROFILE_DIR", profile)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn orphans_as_json_is_an_empty_list() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let (code, out, _) = run(
        dir.path(),
        profile.path(),
        &["env", "orphans", "dev", "--json"],
    );
    assert_eq!(code, 0);
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc, serde_json::json!({"orphans": []}));
}

#[test]
fn forget_needs_a_name_or_all_but_not_both() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let (code, _, _) = run(dir.path(), profile.path(), &["env", "forget", "dev"]);
    assert_ne!(code, 0);
    let (code, _, _) = run(
        dir.path(),
        profile.path(),
        &["env", "forget", "dev", "a", "--all"],
    );
    assert_ne!(code, 0);
}

#[test]
fn forget_all_with_no_orphans_says_so_without_asking() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let (code, out, _) = run(
        dir.path(),
        profile.path(),
        &["env", "forget", "dev", "--all"],
    );
    assert_eq!(code, 0);
    assert!(out.contains("no orphaned secrets"), "{out}");
}

#[test]
fn forget_of_a_name_that_is_no_orphan_fails() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let (code, _, err) = run(
        dir.path(),
        profile.path(),
        &["env", "forget", "dev", "a", "--yes"],
    );
    assert_ne!(code, 0);
    assert!(err.contains("not orphaned"), "{err}");
}

#[test]
fn an_unknown_environment_fails() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let (code, _, _) = run(dir.path(), profile.path(), &["env", "orphans", "nope"]);
    assert_ne!(code, 0);
}

#[test]
fn global_reads_the_profile_and_needs_no_collection() {
    let (dir, profile) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let envs = profile.path().join("global-environments");
    std::fs::create_dir(&envs).unwrap();
    std::fs::write(envs.join("g.yml"), "name: g\n").unwrap();
    let args = ["env", "orphans", "g", "--global", "--json"];
    let (code, out, err) = run(dir.path(), profile.path(), &args);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("orphans"), "{out}");
}

#[test]
fn orphans_takes_a_name_or_all_but_not_both() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let (code, _, _) = run(dir.path(), profile.path(), &["env", "orphans"]);
    assert_ne!(code, 0);
    let args = ["env", "orphans", "dev", "--all"];
    let (code, _, _) = run(dir.path(), profile.path(), &args);
    assert_ne!(code, 0);
}

#[test]
fn orphans_all_as_json_has_both_lists() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let args = ["env", "orphans", "--all", "--json"];
    let (code, out, err) = run(dir.path(), profile.path(), &args);
    assert_eq!(code, 0, "{err}");
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc, serde_json::json!({"orphans": [], "unreadable": []}));
}

#[test]
fn orphans_all_works_on_the_global_environments() {
    let (dir, profile) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let args = ["env", "orphans", "--all", "--global", "--json"];
    let (code, out, err) = run(dir.path(), profile.path(), &args);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("unreadable"), "{out}");
}

#[test]
fn orphans_all_with_no_file_is_refused_as_saying_nothing_more() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let args = ["env", "orphans", "--all", "--no-file"];
    let (code, _, err) = run(dir.path(), profile.path(), &args);
    assert_ne!(code, 0);
    assert!(err.contains("--no-file"), "{err}");
}

#[test]
fn a_fileless_name_stays_unknown_without_the_flag() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let args = ["env", "orphans", "nope"];
    let (code, _, err) = run(dir.path(), profile.path(), &args);
    assert_ne!(code, 0);
    assert!(err.contains("nope") && !err.contains("--no-file"), "{err}");
}

#[test]
fn no_file_takes_the_raw_name_from_the_store() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let args = ["env", "orphans", "nope", "--no-file", "--json"];
    let (code, out, err) = run(dir.path(), profile.path(), &args);
    assert_eq!(code, 0, "{err}");
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc, serde_json::json!({"orphans": []}));
    let args = ["env", "forget", "nope", "--all", "--no-file"];
    let (code, _, err) = run(dir.path(), profile.path(), &args);
    assert_eq!(code, 0, "{err}");
    let args = ["env", "forget", "nope", "a", "--no-file", "--yes"];
    let (code, _, err) = run(dir.path(), profile.path(), &args);
    assert_ne!(code, 0);
    assert!(err.contains("not orphaned"), "{err}");
}

#[test]
fn no_file_on_an_environment_with_a_file_says_to_drop_it() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    for args in [
        &["env", "orphans", "dev", "--no-file"][..],
        &["env", "forget", "dev", "--all", "--no-file"][..],
    ] {
        let (code, _, err) = run(dir.path(), profile.path(), args);
        assert_ne!(code, 0);
        assert!(
            err.contains("has a file") && err.contains("--no-file"),
            "{err}"
        );
    }
}

/// An absolute path under `dir` where nothing exists, as text.
fn gone(dir: &Path) -> String {
    dir.join("old-collection").to_str().unwrap().to_owned()
}

#[test]
fn old_root_lists_without_a_collection_and_implies_no_file() {
    let (dir, profile) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let old = gone(dir.path());
    for args in [
        &["env", "orphans", "--all", "--old-root", &old, "--json"][..],
        &["env", "orphans", "--all", "--old-root", &old, "--no-file"][..],
        &["env", "orphans", "dev", "--old-root", &old][..],
        &["env", "orphans", "dev", "--old-root", &old, "--no-file"][..],
        &["env", "forget", "dev", "--all", "--old-root", &old][..],
    ] {
        let (code, _, err) = run(dir.path(), profile.path(), args);
        assert_eq!(code, 0, "{args:?}: {err}");
    }
}

#[test]
fn old_root_refuses_global_and_collection() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let old = gone(dir.path());
    let here = dir.path().to_str().unwrap();
    for args in [
        &["env", "orphans", "--all", "--old-root", &old, "--global"][..],
        &[
            "env",
            "forget",
            "dev",
            "--all",
            "--old-root",
            &old,
            "--global",
        ][..],
    ] {
        let (code, _, _) = run(dir.path(), profile.path(), args);
        assert_ne!(code, 0, "{args:?}");
    }
    for args in [
        &[
            "env",
            "orphans",
            "--all",
            "--old-root",
            &old,
            "--collection",
            here,
        ][..],
        &[
            "env",
            "forget",
            "dev",
            "--all",
            "--old-root",
            &old,
            "--collection",
            here,
        ][..],
    ] {
        let (code, _, err) = run(dir.path(), profile.path(), args);
        assert_ne!(code, 0, "{args:?}");
        assert!(err.contains("--collection"), "{err}");
    }
}

#[test]
fn old_root_refuses_a_relative_path_and_one_that_exists() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let args = ["env", "orphans", "--all", "--old-root", "some/where"];
    let (code, _, err) = run(dir.path(), profile.path(), &args);
    assert_ne!(code, 0);
    assert!(err.contains("absolute"), "{err}");
    let here = dir.path().to_str().unwrap();
    let args = ["env", "forget", "dev", "--all", "--old-root", here];
    let (code, _, err) = run(dir.path(), profile.path(), &args);
    assert_ne!(code, 0);
    assert!(err.contains("exists"), "{err}");
}

#[test]
fn neither_action_takes_no_keychain() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    for args in [
        &["env", "orphans", "--all", "--no-keychain"][..],
        &[
            "env",
            "forget",
            "dev",
            "--all",
            "--no-file",
            "--no-keychain",
        ][..],
    ] {
        let (code, _, err) = run(dir.path(), profile.path(), args);
        assert_ne!(code, 0, "{args:?}");
        assert!(err.contains("unexpected argument"), "{err}");
    }
}

#[test]
fn the_help_says_how_to_give_an_old_root() {
    let (dir, profile) = (tree(), tempfile::tempdir().unwrap());
    let (code, out, _) = run(dir.path(), profile.path(), &["env", "forget", "--help"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("--old-root") && out.contains("resolved"),
        "{out}"
    );
}
