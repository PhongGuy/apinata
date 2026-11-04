//! `export::write_atomic`: the atomic, shareable file write behind every export.

use apinata_core::export::write_atomic;
use std::path::Path;

#[test]
fn an_atomic_write_leaves_only_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("c.json");
    write_atomic(&file, b"{}\n").unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"{}\n");
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["c.json"]);
    assert!(write_atomic(&dir.path().join("missing/c.json"), b"{}").is_err());
}

// The export is meant to be shared: a new file gets the mode any other new file in that
// directory gets under the current umask, and a replaced file keeps its mode.
#[cfg(unix)]
#[test]
fn an_atomic_write_gives_ordinary_permissions() {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain");
    std::fs::write(&plain, b"").unwrap();
    let file = dir.path().join("c.json");
    write_atomic(&file, b"{}\n").unwrap();
    assert_eq!(mode(&file), mode(&plain));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    write_atomic(&file, b"[]\n").unwrap();
    assert_eq!(mode(&file), 0o640);
}
