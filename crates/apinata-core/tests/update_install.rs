//! The standalone swap on Unix, in temporary directories.
#![cfg(all(feature = "update", unix))]

use crate::update_support;

use apinata_core::update::{Staged, UpdateError, download, install_standalone};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use update_support::*;
use wiremock::{MockServer, ResponseTemplate};

const NEW: &[u8] = b"new binary";

// An executable "old" binary at dir/apinata and a verified 99.0.0 staged beside it.
async fn prepared() -> (MockServer, tempfile::TempDir, PathBuf, Staged) {
    let (s, k, dir) = (
        MockServer::start().await,
        TestKey::generate(),
        tempfile::tempdir().unwrap(),
    );
    let target = dir.path().join("apinata");
    std::fs::write(&target, b"old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    serve_artifact(&s, ResponseTemplate::new(200).set_body_bytes(NEW)).await;
    let staged = download(&offer(&s, &k, NEW).await, dir.path(), |_, _| {})
        .await
        .unwrap();
    (s, dir, target, staged)
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn the_swap_replaces_the_binary_and_keeps_it_executable() {
    let (_s, dir, target, staged) = prepared().await;
    install_standalone(staged, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), NEW);
    assert_eq!(mode(&target), 0o755);
    assert_eq!(leftovers(dir.path()), ["apinata"]);
}

#[tokio::test]
async fn a_read_only_directory_keeps_the_original() {
    let (_s, dir, target, staged) = prepared().await;
    let _g = crate::common::ModeGuard::set(dir.path(), 0o555);
    if std::fs::write(dir.path().join("probe"), b"").is_ok() {
        return; // running as root, which ignores directory permissions
    }
    let got = install_standalone(staged, &target);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        got.unwrap_err(),
        UpdateError::NotWritable(dir.path().to_path_buf())
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
}

#[tokio::test]
async fn a_failed_swap_keeps_the_original_and_removes_the_staged_file() {
    let (_s, dir, _target, staged) = prepared().await;
    // A non-empty directory where the binary belongs makes the rename fail with an I/O error.
    let blocked = dir.path().join("blocked");
    std::fs::create_dir_all(blocked.join("inside")).unwrap();
    let got = install_standalone(staged, &blocked);
    assert!(matches!(got, Err(UpdateError::Io { .. })), "{got:?}");
    assert!(blocked.join("inside").is_dir());
    assert_eq!(leftovers(dir.path()), ["apinata", "blocked"]);
}

#[tokio::test]
async fn a_staged_path_equal_to_the_target_is_refused_and_the_target_kept() {
    let (_s, _dir, target, mut staged) = prepared().await;
    staged.path = target.clone();
    let got = install_standalone(staged, &target);
    assert!(matches!(got, Err(UpdateError::Io { .. })), "{got:?}");
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
}

#[tokio::test]
async fn the_swap_drops_setuid_setgid_and_sticky_bits() {
    let (_s, _dir, target, staged) = prepared().await;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o7755)).unwrap();
    install_standalone(staged, &target).unwrap();
    let full = std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777;
    assert_eq!(full, 0o755);
}
