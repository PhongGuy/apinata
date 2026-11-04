//! The `.app` update: tarball checks, extraction beside the bundle and the swap.
#![cfg(all(feature = "update", unix))]

use crate::update_support;

use apinata_core::update::{InstallKind, Relaunch, Staged, UpdateError, download, install_app};
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use update_support::*;
use wiremock::MockServer;

#[derive(Clone, Copy)]
enum E<'a> {
    Dir(&'a str),
    DirMode(&'a str, u32),
    File(&'a str, &'a [u8], u32),
    Link(&'a str, &'a str),
    Hard(&'a str, &'a str),
}

// Names are written raw, so absolute and `..` paths reach the archive as an attacker's would.
fn tarball(entries: &[E]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for e in entries {
        let (name, kind, link, data, mode): (&str, tar::EntryType, &str, &[u8], u32) = match *e {
            E::Dir(n) => (n, tar::EntryType::Directory, "", &[][..], 0o755),
            E::DirMode(n, m) => (n, tar::EntryType::Directory, "", &[][..], m),
            E::File(n, d, m) => (n, tar::EntryType::Regular, "", d, m),
            E::Link(n, t) => (n, tar::EntryType::Symlink, t, &[][..], 0o777),
            E::Hard(n, t) => (n, tar::EntryType::Link, t, &[][..], 0o644),
        };
        let mut h = tar::Header::new_gnu();
        h.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        h.as_old_mut().linkname[..link.len()].copy_from_slice(link.as_bytes());
        h.set_entry_type(kind);
        h.set_size(data.len() as u64);
        h.set_mode(mode);
        h.set_cksum();
        b.append(&h, data).unwrap();
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&b.into_inner().unwrap()).unwrap();
    gz.finish().unwrap()
}

// A bundle with a framework-style `Versions/Current -> A` link, as real bundles carry.
fn good(top: &str) -> Vec<u8> {
    let p = |rest: &str| format!("{top}/{rest}");
    let (root, contents, macos) = (p(""), p("Contents/"), p("Contents/MacOS/"));
    let (exe, plist) = (p("Contents/MacOS/apinata-app"), p("Contents/Info.plist"));
    let (versions, a, current) = (
        p("Contents/Versions/"),
        p("Contents/Versions/A/"),
        p("Contents/Versions/Current"),
    );
    tarball(&[
        E::Dir(&root),
        E::Dir(&contents),
        E::Dir(&macos),
        E::File(&exe, b"new exe", 0o755),
        E::File(&plist, b"new plist", 0o644),
        E::Dir(&versions),
        E::Dir(&a),
        E::Link(&current, "A"),
    ])
}

fn bundle_in(dir: &Path) -> PathBuf {
    let bundle = dir.join("Apinata.app");
    std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
    std::fs::write(bundle.join("Contents/MacOS/apinata-app"), b"old exe").unwrap();
    std::fs::write(bundle.join("Contents/Info.plist"), b"old plist").unwrap();
    bundle
}

fn old_exe(bundle: &Path) -> Vec<u8> {
    std::fs::read(bundle.join("Contents/MacOS/apinata-app")).unwrap()
}

async fn staged(dir: &Path, body: &[u8]) -> (MockServer, Staged) {
    let s = MockServer::start().await;
    let a = offer_as(&s, "app", body).await;
    let staged = download(&a, dir, |_, _| {}).await.unwrap();
    (s, staged)
}

#[tokio::test]
async fn a_bundle_is_extracted_beside_and_swapped_in() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_in(dir.path());
    let (_s, st) = staged(dir.path(), &good("Apinata.app")).await;
    let got = install_app(&InstallKind::MacApp(bundle.clone()), st);
    assert_eq!(got, Ok(Relaunch::Restart(bundle.clone())));
    assert_eq!(old_exe(&bundle), b"new exe");
    let exe = bundle.join("Contents/MacOS/apinata-app");
    assert_eq!(
        std::fs::metadata(&exe).unwrap().permissions().mode() & 0o7777,
        0o755
    );
    let link = std::fs::read_link(bundle.join("Contents/Versions/Current")).unwrap();
    assert_eq!(link, PathBuf::from("A"));
    assert_eq!(leftovers(dir.path()), ["Apinata.app"]);
}

#[tokio::test]
async fn the_installed_bundle_keeps_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_in(dir.path());
    let (_s, st) = staged(dir.path(), &good("Other.app")).await;
    install_app(&InstallKind::MacApp(bundle.clone()), st).unwrap();
    let plist = std::fs::read(bundle.join("Contents/Info.plist")).unwrap();
    assert_eq!(plist, b"new plist");
    assert_eq!(leftovers(dir.path()), ["Apinata.app"]);
}

#[tokio::test]
async fn a_damaged_archive_is_refused_and_leaves_the_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_in(dir.path());
    for body in [b"not a tarball".to_vec(), tarball(&[])] {
        let (_s, st) = staged(dir.path(), &body).await;
        let got = install_app(&InstallKind::MacApp(bundle.clone()), st);
        assert!(matches!(got, Err(UpdateError::Io { .. })), "{got:?}");
        assert_eq!(old_exe(&bundle), b"old exe");
        assert_eq!(leftovers(dir.path()), ["Apinata.app"]);
    }
}

#[tokio::test]
async fn an_unwritable_parent_refuses_the_install_and_keeps_the_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_in(dir.path());
    let (_s, st) = staged(dir.path(), &good("Apinata.app")).await;
    let _g = crate::common::ModeGuard::set(dir.path(), 0o555);
    if std::fs::write(dir.path().join("probe"), b"").is_ok() {
        return; // running as root
    }
    let got = install_app(&InstallKind::MacApp(bundle.clone()), st);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(got, Err(UpdateError::NotWritable(dir.path().to_path_buf())));
    assert_eq!(old_exe(&bundle), b"old exe");
}

#[tokio::test]
async fn hostile_or_malformed_archives_are_refused_and_leave_the_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_in(dir.path());
    let outside = dir.path().join("evil");
    let abs = outside.to_str().unwrap().to_owned();
    let top = E::Dir("Apinata.app/");
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("absolute path", tarball(&[E::File(&abs, b"x", 0o644)])),
        (
            "parent dir",
            tarball(&[top, E::File("Apinata.app/../evil", b"x", 0o644)]),
        ),
        (
            "link up and out",
            tarball(&[top, E::Link("Apinata.app/Contents", "../../evil")]),
        ),
        (
            "link chain",
            tarball(&[
                top,
                E::Dir("Apinata.app/x/"),
                E::Link("Apinata.app/x/l1", ".."),
                E::Link("Apinata.app/x/l3", "l1/../.."),
            ]),
        ),
        (
            "absolute link",
            tarball(&[top, E::Link("Apinata.app/Contents", "/etc")]),
        ),
        (
            "hard link out",
            tarball(&[top, E::Hard("Apinata.app/x", "Apinata.app/../evil")]),
        ),
        (
            "absolute hard link",
            tarball(&[top, E::Hard("Apinata.app/x", "/etc/passwd")]),
        ),
        ("two bundles", tarball(&[top, E::Dir("Second.app/")])),
        (
            "bundle is a file",
            tarball(&[E::File("Apinata.app", b"x", 0o755)]),
        ),
        (
            "not a bundle",
            tarball(&[E::File("Apinata/x", b"x", 0o644)]),
        ),
    ];
    for (name, body) in cases {
        let (_s, st) = staged(dir.path(), &body).await;
        let got = install_app(&InstallKind::MacApp(bundle.clone()), st);
        assert!(
            matches!(got, Err(UpdateError::Io { .. })),
            "{name}: {got:?}"
        );
        assert!(!outside.exists(), "{name}: wrote outside");
        assert_eq!(old_exe(&bundle), b"old exe", "{name}");
        assert_eq!(leftovers(dir.path()), ["Apinata.app"], "{name}");
    }
}

#[tokio::test]
async fn setuid_bits_are_dropped_on_extraction() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_in(dir.path());
    let body = tarball(&[
        E::Dir("Apinata.app/"),
        E::File("Apinata.app/tool", b"t", 0o4755),
    ]);
    let (_s, st) = staged(dir.path(), &body).await;
    install_app(&InstallKind::MacApp(bundle.clone()), st).unwrap();
    let mode = std::fs::metadata(bundle.join("tool"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7777, 0o755);
}

#[tokio::test]
async fn group_and_other_write_bits_are_dropped_on_extraction() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_in(dir.path());
    let body = tarball(&[
        E::Dir("Apinata.app/"),
        E::File("Apinata.app/tool", b"t", 0o777),
    ]);
    let (_s, st) = staged(dir.path(), &body).await;
    install_app(&InstallKind::MacApp(bundle.clone()), st).unwrap();
    let mode = std::fs::metadata(bundle.join("tool"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7777, 0o755);
}

// A crash between the two renames of an earlier swap leaves the original only at the aside
// path; a later install must not delete it while nothing is in place at the bundle path.
#[tokio::test]
async fn the_aside_copy_is_kept_when_no_bundle_is_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("Apinata.app");
    let (_s, st) = staged(dir.path(), &good("Apinata.app")).await;
    let aside = dir.path().join(format!(".apinata-old-{}.app", st.version));
    std::fs::create_dir(&aside).unwrap();
    std::fs::write(aside.join("marker"), b"original").unwrap();
    let got = install_app(&InstallKind::MacApp(bundle.clone()), st);
    assert!(matches!(got, Err(UpdateError::Io { .. })), "{got:?}");
    assert_eq!(std::fs::read(aside.join("marker")).unwrap(), b"original");
    assert!(!bundle.exists());
}

// A bundle may carry a directory without owner write; cleanup must still remove it, or the
// leftover work or aside directory blocks every later install of that version.
#[tokio::test]
async fn read_only_directories_in_the_update_are_still_cleaned_up() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle_in(dir.path());
    let ro = |tail: &[E<'static>]| {
        let mut entries = vec![
            E::Dir("Apinata.app/"),
            E::File("Apinata.app/ro/f", b"f", 0o644),
            E::DirMode("Apinata.app/ro/", 0o555),
        ];
        entries.extend_from_slice(tail);
        tarball(&entries)
    };
    // A refused archive leaves its read-only directory in the work dir.
    let (_s, st) = staged(dir.path(), &ro(&[E::Dir("Second.app/")])).await;
    let got = install_app(&InstallKind::MacApp(bundle.clone()), st);
    assert!(matches!(got, Err(UpdateError::Io { .. })), "{got:?}");
    assert_eq!(leftovers(dir.path()), ["Apinata.app"]);
    // Installing the same version twice moves the read-only bundle aside the second time.
    for _ in 0..2 {
        let (_s, st) = staged(dir.path(), &ro(&[])).await;
        install_app(&InstallKind::MacApp(bundle.clone()), st).unwrap();
        assert_eq!(leftovers(dir.path()), ["Apinata.app"]);
    }
    let mode = std::fs::metadata(bundle.join("ro"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o555);
}
