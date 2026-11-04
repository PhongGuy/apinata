//! New tree fields reach the files, and an empty list never clears a draft.

use apinata_core::import::{
    ImportedCollection, ImportedItem, ImportedRequest, parse_postman, write_collection,
};
use apinata_core::{Collection, ItemPath, KeyValue, MemoryStore, VarValue, Variable};
use tempfile::TempDir;

fn var(name: &str, value: &str) -> Variable {
    Variable::new(name, VarValue::Text(value.into()))
}

fn imported() -> ImportedCollection {
    let json = br#"{"info": {"name": "c", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
        "item": [{"name": "f", "item": [{"name": "r", "request": {"method": "GET", "url": "https://x.test"}}]}]}"#;
    let mut c = parse_postman(json).expect("parses");
    assert!(c.headers.is_empty());
    c.headers = vec![KeyValue::new("X-Team", "core")];
    let ImportedItem::Folder(folder) = &mut c.items[0] else {
        panic!("folder")
    };
    assert!(folder.headers.is_empty() && folder.vars.is_empty());
    folder.headers = vec![KeyValue::new("X-Folder", "1")];
    folder.vars = vec![var("page", "2")];
    let ImportedItem::Request(item) = &mut folder.items[0] else {
        panic!("request")
    };
    item.request.vars = vec![var("id", "7")];
    item.request.tags = vec!["smoke".into(), "users".into()];
    c
}

#[test]
fn write_collection_writes_headers_vars_and_tags() {
    let parent = TempDir::new().expect("tempdir");
    let (written, _) =
        write_collection(parent.path(), &imported(), &[], &MemoryStore::new()).expect("writes");
    let c = Collection::open(written.root()).expect("opens");
    assert_eq!(c.file().headers(), vec![KeyValue::new("X-Team", "core")]);
    let folder = c
        .folder(&ItemPath::new("f").expect("path"))
        .and_then(|f| f.file())
        .expect("folder.yml");
    assert_eq!(folder.headers(), vec![KeyValue::new("X-Folder", "1")]);
    assert_eq!(folder.vars(), vec![var("page", "2")]);
    let request = c
        .request(&ItemPath::new("f/r.yml").expect("path"))
        .expect("request");
    assert_eq!(request.vars(), vec![var("id", "7")]);
    assert_eq!(request.tags(), ["smoke", "users"]);
}

#[test]
fn apply_to_keeps_a_drafts_tags_and_vars_unless_the_import_has_some() {
    let dir = TempDir::new().expect("tempdir");
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    let path = c.create_request(&ItemPath::root(), "r").expect("request");
    let file = c.request_mut(&path).expect("loaded");
    file.set_tags(vec!["mine".into()]);
    file.set_vars(vec![var("v", "1")]);
    let bare = ImportedRequest::new("GET", "https://x.test");
    assert!(bare.tags.is_empty() && bare.vars.is_empty());
    bare.apply_to(file).expect("applies");
    assert_eq!(
        (file.tags(), file.vars()),
        (vec!["mine".to_owned()], vec![var("v", "1")])
    );
    let mut tagged = ImportedRequest::new("GET", "https://x.test");
    tagged.tags = vec!["new".into()];
    tagged.vars = vec![var("w", "2")];
    tagged.apply_to(file).expect("applies");
    assert_eq!(
        (file.tags(), file.vars()),
        (vec!["new".to_owned()], vec![var("w", "2")])
    );
}
