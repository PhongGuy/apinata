//! Staging an app update beside its install, the AppImage swap and the NSIS hand-off.
#![cfg(all(feature = "update", unix))]

use crate::update_support;

use apinata_core::update::{
    InstallKind, Reason, Relaunch, UpdateError, app_staging_dir, download, install_app, stage_app,
};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use update_support::*;
use wiremock::MockServer;

const NEW: &[u8] = b"new app";

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

async fn artifact_requests(s: &MockServer) -> usize {
    let all = s.received_requests().await.unwrap();
    all.iter().filter(|r| r.url.path() == "/artifact").count()
}

fn appimage_in(dir: &Path) -> PathBuf {
    let target = dir.join("Apinata.AppImage");
    std::fs::write(&target, b"old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    target
}

#[test]
fn staging_happens_beside_the_install() {
    let image = InstallKind::AppImage("/opt/apps/Apinata.AppImage".into());
    assert_eq!(app_staging_dir(&image), Ok(PathBuf::from("/opt/apps")));
    let bundle = InstallKind::MacApp("/Applications/Apinata.app".into());
    assert_eq!(app_staging_dir(&bundle), Ok(PathBuf::from("/Applications")));
    assert_eq!(
        app_staging_dir(&InstallKind::WindowsNsis),
        Ok(std::env::temp_dir())
    );
    assert_eq!(
        app_staging_dir(&InstallKind::NotOwned(Reason::SystemPackage)),
        Err(UpdateError::NotOwned(Reason::SystemPackage))
    );
    assert_eq!(
        app_staging_dir(&InstallKind::Standalone("/usr/local/bin/apinata".into())),
        Err(UpdateError::NotOwned(Reason::Unrecognised))
    );
}

#[tokio::test]
async fn a_format_that_does_not_match_the_install_is_refused_before_download() {
    let (s, dir) = (MockServer::start().await, tempfile::tempdir().unwrap());
    let target = appimage_in(dir.path());
    let a = offer_as(&s, "binary", NEW).await;
    let got = stage_app(&InstallKind::AppImage(target), &a, |_, _| {}).await;
    assert!(
        matches!(got, Err(UpdateError::ManifestInvalid(_))),
        "{got:?}"
    );
    assert_eq!(artifact_requests(&s).await, 0);
    assert_eq!(leftovers(dir.path()), ["Apinata.AppImage"]);
}

#[tokio::test]
async fn an_appimage_is_staged_beside_and_swapped_in_place() {
    let (s, dir) = (MockServer::start().await, tempfile::tempdir().unwrap());
    let target = appimage_in(dir.path());
    let kind = InstallKind::AppImage(target.clone());
    let a = offer_as(&s, "appimage", NEW).await;
    let staged = stage_app(&kind, &a, |_, _| {}).await.unwrap();
    assert_eq!(staged.path.parent(), Some(dir.path()));
    assert_eq!(
        install_app(&kind, staged),
        Ok(Relaunch::Restart(target.clone()))
    );
    assert_eq!(std::fs::read(&target).unwrap(), NEW);
    assert_eq!(mode(&target), 0o755);
    assert_eq!(leftovers(dir.path()), ["Apinata.AppImage"]);
}

#[tokio::test]
async fn an_unwritable_install_directory_is_refused_before_download() {
    let (s, dir) = (MockServer::start().await, tempfile::tempdir().unwrap());
    let target = appimage_in(dir.path());
    let _g = crate::common::ModeGuard::set(dir.path(), 0o555);
    if std::fs::write(dir.path().join("probe"), b"").is_ok() {
        return; // running as root: permissions do not apply
    }
    let a = offer_as(&s, "appimage", NEW).await;
    let got = stage_app(&InstallKind::AppImage(target), &a, |_, _| {}).await;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        got.unwrap_err(),
        UpdateError::NotWritable(dir.path().to_path_buf())
    );
    assert_eq!(artifact_requests(&s).await, 0);
}

#[tokio::test]
async fn the_nsis_installer_is_handed_off_passive_with_relaunch() {
    let (s, dir) = (MockServer::start().await, tempfile::tempdir().unwrap());
    let staged = download(&offer_as(&s, "nsis", NEW).await, dir.path(), |_, _| {})
        .await
        .unwrap();
    let program = dir.path().join("apinata-update-99.0.0-setup.exe");
    let args = vec![OsString::from("/P"), OsString::from("/R")];
    assert_eq!(
        install_app(&InstallKind::WindowsNsis, staged),
        Ok(Relaunch::Installer {
            program: program.clone(),
            args
        })
    );
    assert_eq!(std::fs::read(&program).unwrap(), NEW);
    assert_eq!(leftovers(dir.path()), ["apinata-update-99.0.0-setup.exe"]);
}

#[tokio::test]
async fn kinds_the_app_does_not_install_are_refused_and_cleaned_up() {
    let (s, dir) = (MockServer::start().await, tempfile::tempdir().unwrap());
    let a = offer_as(&s, "binary", NEW).await;
    let cases = [
        (
            InstallKind::NotOwned(Reason::SystemPackage),
            Reason::SystemPackage,
        ),
        (
            InstallKind::Standalone(dir.path().join("apinata")),
            Reason::Unrecognised,
        ),
    ];
    for (kind, reason) in cases {
        let staged = download(&a, dir.path(), |_, _| {}).await.unwrap();
        assert_eq!(
            install_app(&kind, staged),
            Err(UpdateError::NotOwned(reason))
        );
        assert!(leftovers(dir.path()).is_empty(), "{kind:?}");
    }
}

#[tokio::test]
async fn a_staged_link_is_refused_for_nsis_and_only_the_link_is_removed() {
    let (s, dir) = (MockServer::start().await, tempfile::tempdir().unwrap());
    let staged = download(&offer_as(&s, "nsis", NEW).await, dir.path(), |_, _| {})
        .await
        .unwrap();
    let target = dir.path().join("elsewhere");
    std::fs::write(&target, b"keep").unwrap();
    std::fs::remove_file(&staged.path).unwrap();
    std::os::unix::fs::symlink(&target, &staged.path).unwrap();
    let got = install_app(&InstallKind::WindowsNsis, staged);
    assert!(matches!(got, Err(UpdateError::Io { .. })), "{got:?}");
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    assert_eq!(leftovers(dir.path()), ["elsewhere"]);
}
