//! `apinata convert` and the refusal of legacy roots by `run` and `mcp`.

use crate::common;

use common::apinata;
use predicates::prelude::*;
use predicates::str::contains;
use std::fs;
use std::path::Path;
use std::time::Duration;

const REQUEST: &str =
    "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n";

fn legacy() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in [
        (
            "bruno.json",
            r#"{"version": "1", "name": "Legacy", "type": "collection"}"#.to_owned(),
        ),
        ("a.bru", REQUEST.to_owned()),
        ("f/b.bru", REQUEST.replace("name: a", "name: b")),
        (".env", "TOKEN=x\n".to_owned()),
    ] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

fn canonical(path: &Path) -> String {
    fs::canonicalize(path).unwrap().display().to_string()
}

#[test]
fn out_converts_into_a_new_folder_and_prints_its_root() {
    let src = legacy();
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("new");
    let out = apinata()
        .arg("convert")
        .arg(src.path())
        .arg("--out")
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{}\n", canonical(&target))
    );
    assert!(target.join("opencollection.yml").is_file() && target.join("f/b.yml").is_file());
    assert!(src.path().join("a.bru").is_file());
}

#[test]
fn dry_run_lists_the_plan_and_writes_nothing() {
    let src = legacy();
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("new");
    apinata()
        .arg("convert")
        .arg(src.path())
        .arg("--out")
        .arg(&target)
        .arg("--dry-run")
        .assert()
        .code(0)
        .stdout("create a.yml\ncreate f/b.yml\ncreate opencollection.yml\ncopy .env\n");
    assert!(!target.exists());
    apinata().arg("convert").arg(src.path()).args(["--in-place", "--dry-run"])
        .assert().code(0)
        .stdout("create a.yml\ncreate f/b.yml\ncreate opencollection.yml\ndelete a.bru\ndelete bruno.json\ndelete f/b.bru\n");
    assert!(src.path().join("a.bru").is_file() && !src.path().join("a.yml").exists());
}

#[test]
fn in_place_without_a_terminal_needs_yes() {
    let src = legacy();
    apinata()
        .arg("convert")
        .arg(src.path())
        .arg("--in-place")
        .assert()
        .code(3)
        .stderr(contains("--yes"));
    assert!(src.path().join("a.bru").is_file() && !src.path().join("opencollection.yml").exists());
}

#[test]
fn in_place_with_yes_lists_the_changes_and_converts() {
    let src = legacy();
    let out = apinata()
        .arg("convert")
        .arg(src.path())
        .args(["--in-place", "--yes"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("delete a.bru\n") && stdout.contains("create opencollection.yml\n"),
        "{stdout}"
    );
    assert!(
        stdout.ends_with(&format!("{}\n", canonical(src.path()))),
        "{stdout}"
    );
    assert!(!src.path().join("a.bru").exists() && src.path().join(".env").is_file());
    assert!(src.path().join("opencollection.yml").is_file());
}

#[test]
fn exactly_one_target_is_required() {
    let src = legacy();
    apinata().arg("convert").arg(src.path()).assert().code(3);
    apinata()
        .arg("convert")
        .arg(src.path())
        .args(["--in-place", "--out", "x"])
        .assert()
        .code(3);
}

#[test]
fn failures_list_their_files_and_exit_3() {
    let src = legacy();
    fs::write(src.path().join("bad.bru"), "meta {\n  name: a\n").unwrap();
    let parent = tempfile::tempdir().unwrap();
    apinata()
        .arg("convert")
        .arg(src.path())
        .arg("--out")
        .arg(parent.path().join("n"))
        .assert()
        .code(3)
        .stderr(contains("1 of the legacy files could not be read").and(contains("bad.bru:")));
    let ok = legacy();
    apinata()
        .arg("convert")
        .arg(ok.path())
        .arg("--out")
        .arg(parent.path())
        .assert()
        .code(3)
        .stderr(contains("would overwrite existing paths (1)"));
}

#[test]
fn run_and_mcp_refuse_a_legacy_root_and_point_at_convert() {
    let src = legacy();
    let message = "is in the legacy .bru format; convert it with `apinata convert ";
    apinata()
        .arg("run")
        .arg(src.path())
        .assert()
        .code(3)
        .stderr(contains(message));
    apinata()
        .arg("run")
        .arg(src.path().join("a.bru"))
        .assert()
        .code(3)
        .stderr(contains(message));
    apinata()
        .args(["mcp", "--collection"])
        .arg(src.path())
        .timeout(Duration::from_secs(30))
        .assert()
        .code(3)
        .stderr(contains(message));
}

#[test]
fn a_dry_run_into_an_existing_folder_fails_like_the_real_run() {
    let src = legacy();
    let parent = tempfile::tempdir().unwrap();
    for extra in [&[][..], &["--dry-run"][..]] {
        apinata()
            .arg("convert")
            .arg(src.path())
            .arg("--out")
            .arg(parent.path())
            .args(extra)
            .assert()
            .code(3)
            .stdout("")
            .stderr(contains("would overwrite existing paths (1)"));
    }
}

#[test]
fn yes_with_out_is_a_usage_error() {
    let src = legacy();
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("new");
    apinata()
        .arg("convert")
        .arg(src.path())
        .arg("--out")
        .arg(&target)
        .arg("--yes")
        .assert()
        .code(3)
        .stderr(contains("cannot be used with"));
    assert!(!target.exists());
}
