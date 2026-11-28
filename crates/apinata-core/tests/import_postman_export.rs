//! A Postman data export, as a zip, an unzipped folder or its `archive.json`, read in one step.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use apinata_core::import::{
    ExportError, NoteKind, PostmanExport, looks_like_postman_export, parse_postman_export,
};
use zip::write::SimpleFileOptions;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";
const ARCHIVE: &str = r#"{"collection":{"c1":true,"c2":true},"environment":{"e1":true}}"#;

fn collection(name: &str) -> String {
    format!(
        r#"{{"info":{{"name":"{name}","schema":"{V21}"}},"item":[{{"name":"r","request":{{"method":"GET","url":"https://x.test/r"}}}}]}}"#
    )
}

fn environment(name: &str) -> String {
    format!(
        r#"{{"name":"{name}","values":[{{"key":"host","value":"x.test","enabled":true}}],"_postman_variable_scope":"environment"}}"#
    )
}

const GLOBALS: &str =
    r#"{"name":"Globals","values":[{"key":"g","value":"1"}],"_postman_variable_scope":"globals"}"#;

/// The entries of a small export: two collections out of name order, one environment, the
/// archive marker, and the export's top folder around them all.
fn entries() -> Vec<(String, String)> {
    vec![
        ("export/archive.json".into(), ARCHIVE.into()),
        ("export/collection/c1.json".into(), collection("Zeta")),
        ("export/collection/c2.json".into(), collection("Alpha")),
        ("export/environment/e1.json".into(), environment("dev")),
    ]
}

fn zip_of(path: &Path, entries: &[(String, String)]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, text) in entries {
        zip.start_file(name.as_str(), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn folder_of(root: &Path, entries: &[(String, String)]) {
    for (name, text) in entries {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

/// Collection names, then each collection's environments, then each skipped entry and why.
fn summary(export: &PostmanExport) -> (Vec<String>, Vec<Vec<String>>, Vec<String>) {
    let names = export
        .bundles
        .iter()
        .map(|b| b.collection.name.clone())
        .collect();
    let envs = export
        .bundles
        .iter()
        .map(|b| b.environments.iter().map(|e| e.name.clone()).collect())
        .collect();
    let skipped = export
        .skipped
        .iter()
        .map(|n| {
            assert_eq!(n.kind, NoteKind::Skipped);
            n.source.clone()
        })
        .collect();
    (names, envs, skipped)
}

#[test]
fn a_zip_export_gives_one_bundle_per_collection_each_with_every_environment() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("export.zip");
    zip_of(&zip, &entries());
    assert!(looks_like_postman_export(&zip));
    let export = parse_postman_export(&zip).expect("parsed");
    assert_eq!(
        summary(&export),
        (
            vec!["Alpha".to_owned(), "Zeta".to_owned()],
            vec![vec!["dev".to_owned()], vec!["dev".to_owned()]],
            vec![],
        )
    );
}

#[test]
fn the_folder_and_its_archive_json_read_like_the_zip() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("export.zip");
    zip_of(&zip, &entries());
    folder_of(dir.path(), &entries());
    let root = dir.path().join("export");
    let from_zip = summary(&parse_postman_export(&zip).unwrap());
    for path in [root.clone(), root.join("archive.json")] {
        assert!(looks_like_postman_export(&path), "{}", path.display());
        assert_eq!(
            summary(&parse_postman_export(&path).unwrap()),
            from_zip,
            "{}",
            path.display()
        );
    }
}

#[test]
fn unreadable_entries_and_globals_are_skipped_and_the_rest_still_import() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("export.zip");
    let mut all = entries();
    all.push(("export/collection/bad.json".into(), "{not json".into()));
    all.push(("export/globals.json".into(), GLOBALS.into()));
    all.push(("export/notes.txt".into(), "hello".into()));
    zip_of(&zip, &all);
    let export = parse_postman_export(&zip).expect("parsed");
    let (names, _, skipped) = summary(&export);
    assert_eq!(names, ["Alpha", "Zeta"]);
    assert_eq!(
        skipped,
        [
            "export/collection/bad.json",
            "export/globals.json",
            "export/notes.txt"
        ]
    );
    assert!(
        export.skipped[1].detail.contains("globals"),
        "{}",
        export.skipped[1].detail
    );
}

#[test]
fn entry_names_that_climb_out_are_only_labels() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("export.zip");
    let mut all = entries();
    all.push(("../../escape.json".into(), collection("Escape")));
    all.push(("/abs/escape.json".into(), collection("Absolute")));
    zip_of(&zip, &all);
    let export = parse_postman_export(&zip).expect("parsed");
    let (names, _, _) = summary(&export);
    assert_eq!(names, ["Absolute", "Alpha", "Escape", "Zeta"]);
    assert!(!dir.path().join("../../escape.json").exists());
}

#[test]
fn a_zip_or_folder_without_the_archive_marker_is_not_an_export() {
    let dir = tempfile::tempdir().unwrap();
    let zip = dir.path().join("plain.zip");
    let without: Vec<_> = entries().into_iter().skip(1).collect();
    zip_of(&zip, &without);
    // Any zip is worth trying, since no other import reads one; reading it tells the truth.
    assert!(looks_like_postman_export(&zip));
    assert!(matches!(
        parse_postman_export(&zip),
        Err(ExportError::NotExport { .. })
    ));
    let folder = dir.path().join("plain");
    folder_of(&folder, &without);
    assert!(!looks_like_postman_export(&folder.join("export")));
    let json = dir.path().join("c.json");
    fs::write(&json, collection("c")).unwrap();
    assert!(!looks_like_postman_export(&json));
}

#[cfg(unix)]
#[test]
fn a_symlink_in_an_export_folder_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    folder_of(dir.path(), &entries());
    let outside = dir.path().join("outside.json");
    fs::write(&outside, collection("Outside")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.path().join("export/collection/link.json")).unwrap();
    let export = parse_postman_export(&dir.path().join("export")).expect("parsed");
    let (names, _, skipped) = summary(&export);
    assert_eq!(names, ["Alpha", "Zeta"]);
    assert_eq!(skipped, ["collection/link.json"]);
}

#[test]
fn a_symlink_entry_in_a_zip_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, text) in entries() {
        zip.start_file(name.as_str(), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.add_symlink(
        "export/collection/link.json",
        "/etc/passwd",
        SimpleFileOptions::default(),
    )
    .unwrap();
    zip.finish().unwrap();
    let export = parse_postman_export(&path).expect("parsed");
    let (names, _, skipped) = summary(&export);
    assert_eq!(names, ["Alpha", "Zeta"]);
    assert_eq!(skipped, ["export/collection/link.json"]);
}

#[test]
fn a_folder_holding_the_export_folder_reads_as_an_export() {
    let dir = tempfile::tempdir().unwrap();
    folder_of(dir.path(), &entries());
    assert!(looks_like_postman_export(dir.path()));
    let (names, _, _) = summary(&parse_postman_export(dir.path()).expect("parsed"));
    assert_eq!(names, ["Alpha", "Zeta"]);
}

#[test]
fn a_skip_note_never_repeats_a_value_from_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.zip");
    let mut all = entries();
    let leaky = format!(r#"{{"info":{{"name":"x","schema":"{V21}"}},"item":"tok-s3cret"}}"#);
    all.push(("export/collection/leaky.json".into(), leaky));
    all.push((
        "export/collection/cut.json".into(),
        format!(r#"{{"info":{{"name":"x","schema":"{V21}"}},"auth":"tok-s3cret"#),
    ));
    zip_of(&path, &all);
    let export = parse_postman_export(&path).expect("parsed");
    assert_eq!(export.skipped.len(), 2, "{:?}", export.skipped);
    for note in &export.skipped {
        assert!(!note.detail.contains("s3cret"), "{}", note.detail);
    }
}

/// Run `f` on another thread, failing when it does not return within ten seconds.
#[cfg(unix)]
fn within_seconds<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .expect("the read hung")
}

#[cfg(unix)]
fn mkfifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn a_fifo_in_an_export_folder_is_skipped_without_opening_it() {
    let dir = tempfile::tempdir().unwrap();
    folder_of(dir.path(), &entries());
    mkfifo(&dir.path().join("export/collection/pipe.json"));
    let root = dir.path().join("export");
    let export = within_seconds(move || parse_postman_export(&root).expect("parsed"));
    let (names, _, skipped) = summary(&export);
    assert_eq!(names, ["Alpha", "Zeta"]);
    assert_eq!(skipped, ["collection/pipe.json"]);
}

#[cfg(unix)]
#[test]
fn a_fifo_given_as_the_path_is_refused_without_opening_it() {
    let dir = tempfile::tempdir().unwrap();
    let pipe = dir.path().join("export.zip");
    mkfifo(&pipe);
    let (looks, parsed) = within_seconds(move || {
        (
            looks_like_postman_export(&pipe),
            parse_postman_export(&pipe).is_err(),
        )
    });
    assert!(!looks);
    assert!(parsed);
}
