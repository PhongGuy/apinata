//! Scripts carried by an imported tree reach the files at all three levels; blank ones write
//! nothing.

use apinata_core::import::{
    ImportedCollection, ImportedItem, ImportedScripts, parse_postman, write_collection,
};
use apinata_core::{MemoryStore, ScriptPhase};
use std::fs;
use tempfile::TempDir;

fn scripts(pre: &str, post: &str) -> ImportedScripts {
    let mut s = ImportedScripts::default();
    s.pre_request = pre.into();
    s.post_response = post.into();
    s
}

fn tree(
    root: ImportedScripts,
    folder: ImportedScripts,
    request: ImportedScripts,
) -> ImportedCollection {
    let json = br#"{"info": {"name": "c", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
        "item": [{"name": "f", "item": [{"name": "r", "request": {"method": "GET", "url": "https://x.test"}}]}]}"#;
    let mut c = parse_postman(json).expect("parses");
    c.scripts = root;
    let ImportedItem::Folder(f) = &mut c.items[0] else {
        panic!("folder")
    };
    f.scripts = folder;
    let ImportedItem::Request(r) = &mut f.items[0] else {
        panic!("request")
    };
    r.request.scripts = request;
    c
}

fn written(c: &ImportedCollection) -> (apinata_core::Collection, TempDir) {
    let parent = TempDir::new().expect("tempdir");
    let (collection, _) =
        write_collection(parent.path(), c, &[], &MemoryStore::new()).expect("writes");
    (collection, parent)
}

#[test]
fn scripts_are_written_at_request_folder_and_root_level() {
    let c = tree(
        scripts("bru.setVar('a', 1);", "bru.setVar('b', 2);"),
        scripts("bru.setVar('f', 1);", ""),
        scripts("bru.setVar('r', 1);", "test('ok', () => {});"),
    );
    let (collection, _dir) = written(&c);
    let root = collection.root();

    let request = fs::read_to_string(root.join("f/r.yml")).expect("request file");
    assert!(request.contains("runtime:\n  scripts:"), "{request}");
    assert!(request.contains("type: before-request"), "{request}");
    assert!(request.contains("bru.setVar('r', 1);"), "{request}");
    assert!(request.contains("type: after-response"), "{request}");
    assert!(request.contains("test('ok', () => {});"), "{request}");

    let folder = fs::read_to_string(root.join("f/folder.yml")).expect("folder file");
    assert!(
        folder.contains("request:\n  auth: inherit\n  scripts:"),
        "{folder}"
    );
    assert!(folder.contains("bru.setVar('f', 1);"), "{folder}");
    assert!(!folder.contains("after-response"), "{folder}");

    let top = fs::read_to_string(root.join("opencollection.yml")).expect("root file");
    assert!(top.contains("request:\n  scripts:"), "{top}");
    assert!(top.contains("bru.setVar('a', 1);"), "{top}");
    assert!(top.contains("bru.setVar('b', 2);"), "{top}");

    assert_eq!(
        collection.file().script(ScriptPhase::PostResponse),
        "bru.setVar('b', 2);"
    );
}

#[test]
fn blank_scripts_write_nothing() {
    let c = tree(scripts("  \n", ""), scripts("", "\n\n"), scripts(" ", "\t"));
    let (collection, _dir) = written(&c);
    let root = collection.root();
    for file in ["f/r.yml", "f/folder.yml", "opencollection.yml"] {
        let text = fs::read_to_string(root.join(file)).expect("file");
        assert!(!text.contains("scripts"), "{file}: {text}");
    }
}
