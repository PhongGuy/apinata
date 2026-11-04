//! `write_collection`: Bruno's directory naming, collision handling, notes tied to files, and
//! secrets stored under the final root.

use apinata_core::import::{
    ImportError, NoteKind, parse_postman, parse_postman_environment, write_collection,
};
use apinata_core::{CreateError, ItemPath, MemoryStore, SecretStore};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

fn fixture(rel: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/postman")
            .join(rel),
    )
    .expect("fixture")
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_collection_is_written_under_its_sanitised_name_with_nothing_left_behind() {
    let parent = TempDir::new().expect("tempdir");
    let imported = parse_postman(&fixture("collections/apinata-edge-cases.json")).expect("parses");
    let (collection, report) =
        write_collection(parent.path(), &imported, &[], &MemoryStore::new()).expect("writes");
    assert_eq!(
        entries(parent.path()),
        ["Edge- cases-1"],
        "no staging directory survives"
    );
    assert_eq!(collection.root(), parent.path().join("Edge- cases-1"));
    assert_eq!(collection.name(), "Edge: cases/1");
    assert!(
        collection.root().join("environments").is_dir(),
        "Bruno always creates it"
    );
    assert_eq!(
        report.items.len(),
        14,
        "2 folders and 12 requests: {:?}",
        report.items
    );
    // "a/b" and "a-b" sanitise alike: the second gets a suffix and keeps its display name.
    let renamed = ItemPath::new("Users/a-b 1.yml").expect("path");
    assert_eq!(collection.request(&renamed).expect("written").name(), "a-b");
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Renamed && n.file.as_deref() == Some("Users/a-b 1.yml")),
        "{:?}",
        report.notes
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Converted
                && n.file.as_deref() == Some("Users/Get user.yml"))
    );
    let (again, _) =
        write_collection(parent.path(), &imported, &[], &MemoryStore::new()).expect("writes again");
    assert_eq!(again.name(), "Edge: cases/1 - 1");
    assert_eq!(again.root(), parent.path().join("Edge- cases-1 - 1"));
}

#[test]
fn names_that_differ_by_case_or_cannot_be_files_are_renamed() {
    let json = format!(
        r#"{{"info": {{"name": "c", "schema": "{V21}"}}, "item": [
        {{"name": "Get", "request": {{"method": "GET", "url": "u"}}}}, {{"name": "get", "request": {{"method": "GET", "url": "u"}}}},
        {{"name": "CON", "request": {{"method": "GET", "url": "u"}}}}, {{"name": "My Environments", "item": []}},
        {{"name": "{long}", "request": {{"method": "GET", "url": "u"}}}}, {{"name": "{wide}", "request": {{"method": "GET", "url": "u"}}}}]}}"#,
        long = "a".repeat(300),
        wide = "名".repeat(100)
    );
    // `long` is over 255 UTF-16 units; `wide` is 100 units but 300 UTF-8 bytes, which the file system refuses.
    let imported = parse_postman(json.as_bytes()).expect("parses");
    let parent = TempDir::new().expect("tempdir");
    let (collection, report) =
        write_collection(parent.path(), &imported, &[], &MemoryStore::new()).expect("writes");
    assert_eq!(
        entries(collection.root()),
        [
            "CON 1.yml",
            "Folder 4",
            "Get.yml",
            "Request 5.yml",
            "Request 6.yml",
            "environments",
            "get 1.yml",
            "opencollection.yml"
        ]
    );
    let renamed: Vec<&str> = report
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::Renamed)
        .filter_map(|n| n.file.as_deref())
        .collect();
    assert_eq!(
        renamed,
        [
            "get 1.yml",
            "CON 1.yml",
            "Folder 4/folder.yml",
            "Request 5.yml",
            "Request 6.yml"
        ]
    );
    let folder = collection
        .folder(&ItemPath::new("Folder 4").expect("path"))
        .expect("folder");
    assert_eq!(folder.name(), "My Environments", "the display name is kept");
    let wide = collection
        .request(&ItemPath::new("Request 6.yml").expect("path"))
        .expect("request");
    assert_eq!(wide.name(), "名".repeat(100));
}

#[test]
fn secrets_are_stored_under_the_final_root_after_the_rename() {
    let parent = TempDir::new().expect("tempdir");
    let json = format!(r#"{{"info": {{"name": "c", "schema": "{V21}"}}, "item": []}}"#);
    let imported = parse_postman(json.as_bytes()).expect("parses");
    let env = parse_postman_environment(&fixture("environments/apinata-env-edge-cases.json"))
        .expect("parses");
    let store = MemoryStore::new();
    let (collection, report) =
        write_collection(parent.path(), &imported, &[env], &store).expect("writes");
    assert_eq!(report.environments, ["Staging- EU"]);
    let key = collection.secret_key("Staging- EU", "token");
    assert_eq!(
        key.collection,
        fs::canonicalize(parent.path().join("c")).expect("canonical")
    );
    assert_eq!(store.get(&key).expect("reads"), Some("s3cret".into()));
    assert!(collection.environment("Staging- EU").is_some());
}

/// Items fall back to `Request <seq>`, but the collection's own name has no fallback: a name
/// that cannot be a directory fails the import, which leaves nothing behind.
#[test]
fn a_collection_name_that_cannot_be_a_directory_fails_with_invalid_name() {
    for name in ["   ", "CON", &"名".repeat(100)] {
        let parent = TempDir::new().expect("tempdir");
        let json = format!(r#"{{"info": {{"name": "{name}", "schema": "{V21}"}}, "item": []}}"#);
        let imported = parse_postman(json.as_bytes()).expect("parses");
        let result = write_collection(parent.path(), &imported, &[], &MemoryStore::new());
        assert!(
            matches!(
                result,
                Err(ImportError::Create(CreateError::InvalidName { .. }))
            ),
            "{name:?}: {result:?}"
        );
        assert!(entries(parent.path()).is_empty(), "{name:?} left files");
    }
}
