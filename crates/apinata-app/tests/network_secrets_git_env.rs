//! A stray git environment in the app's process does not hide a tracked `.env`.
//! The test re-runs itself in a child process that carries the stray variables, so the shared
//! test process keeps a clean environment.

use apinata_app::network_secrets::git_tracks_dotenv;
use std::process::Command;

const CHILD_DIR: &str = "APINATA_STRAY_GIT_CHILD_DIR";
const THIS_TEST: &str =
    "network_secrets_git_env::a_stray_common_or_object_dir_does_not_hide_a_tracked_dotenv";

#[test]
fn a_stray_common_or_object_dir_does_not_hide_a_tracked_dotenv() {
    // In the child the stray variables are already set by the parent; only the check runs.
    if let Some(dir) = std::env::var_os(CHILD_DIR) {
        assert!(git_tracks_dotenv(std::path::Path::new(&dir)));
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "X=1\n").unwrap();
    for args in [&["init", "-q"][..], &["add", ".env"]] {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .expect("git runs")
            .success();
        assert!(ok, "git {args:?}");
    }
    let bogus = dir.path().join("nowhere");
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", THIS_TEST, "--test-threads=1"])
        .env(CHILD_DIR, dir.path())
        .env("GIT_COMMON_DIR", &bogus)
        .env("GIT_OBJECT_DIRECTORY", &bogus)
        .output()
        .expect("the test binary re-runs");
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stdout)
    );
}
