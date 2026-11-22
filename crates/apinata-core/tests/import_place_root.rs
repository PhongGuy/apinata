//! `place_items` carrying the imported collection's own settings and environments into the
//! target: the root's settings onto the new folder, its environments into the collection.

use apinata_core::import::{
    CollectionOptions, ImportBundle, ImportReport, NoteKind, ensure_folder, parse_collections,
    place_items,
};
use apinata_core::{AuthSpec, Collection, MemoryStore, ScriptPhase, SecretStore};
use std::path::Path;
use tempfile::TempDir;

fn fresh() -> (TempDir, Collection) {
    let dir = TempDir::new().unwrap();
    let c = Collection::create(&dir.path().join("c"), "c").unwrap();
    (dir, c)
}

fn fixture(rel: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(rel),
    )
    .expect("fixture")
}

fn parsed(bytes: &[u8]) -> ImportBundle {
    parse_collections(None, bytes, &CollectionOptions::default())
        .expect("parses")
        .remove(0)
}

fn place(
    c: &mut Collection,
    name: &str,
    b: &ImportBundle,
    store: &dyn SecretStore,
) -> ImportReport {
    let folder = ensure_folder(c, &[name.to_owned()]).unwrap();
    place_items(c, &folder, b, store).unwrap().report
}

#[test]
fn the_postman_root_settings_land_on_the_new_folder() {
    let (_dir, mut c) = fresh();
    let root_before = std::fs::read_to_string(c.root().join("opencollection.yml")).unwrap();
    let b = parsed(&fixture("import-folder/postman-root.json"));
    let report = place(&mut c, "pmf", &b, &MemoryStore::new());
    let folder = c
        .folder_file_mut(&apinata_core::ItemPath::new("pmf").unwrap())
        .unwrap();
    assert!(
        matches!(folder.auth(), AuthSpec::Bearer { .. }),
        "{:?}",
        folder.auth()
    );
    let vars: Vec<String> = folder.vars().into_iter().map(|v| v.name).collect();
    assert_eq!(vars, ["base", "tok"]);
    assert!(
        folder
            .script(ScriptPhase::PreRequest)
            .contains("console.log(1)")
    );
    assert!(folder.script(ScriptPhase::PostResponse).contains("test("));
    assert_eq!(folder.docs(), "Root docs");
    assert_eq!(
        std::fs::read_to_string(c.root().join("opencollection.yml")).unwrap(),
        root_before
    );
    // `base` is defined by the folder now, so nothing asks for it.
    assert!(
        report.notes.iter().all(|n| !n.detail.contains("`base`")),
        "{:?}",
        report.notes
    );
}

#[test]
fn no_note_names_the_target_root_file() {
    let (_dir, mut c) = fresh();
    let b = parsed(&fixture("import-folder/postman-root.json"));
    let report = place(&mut c, "pmf", &b, &MemoryStore::new());
    assert!(
        report
            .notes
            .iter()
            .all(|n| n.file.as_deref() != Some("opencollection.yml")),
        "{:?}",
        report.notes
    );
    // The script's conversion note is tied to the folder file that holds the script.
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.file.as_deref() == Some("pmf/folder.yml")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_literal_root_credential_becomes_a_secret_variable_of_the_target() {
    let (_dir, mut c) = fresh();
    let input = serde_json::json!({
        "info": {
            "name": "P",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "s3cret-root"}]},
        "item": [{"name": "R", "request": {"method": "GET", "url": "http://127.0.0.1:1/x"}}]
    })
    .to_string();
    let store = MemoryStore::new();
    let report = place(&mut c, "pmf", &parsed(input.as_bytes()), &store);
    let text = std::fs::read_to_string(c.root().join("pmf/folder.yml")).unwrap();
    assert!(!text.contains("s3cret-root"), "{text}");
    let env = &c.environments()[0];
    let secret = env
        .variables()
        .into_iter()
        .find(|v| v.secret)
        .expect("a secret variable");
    assert!(text.contains(&format!("{{{{{}}}}}", secret.name)), "{text}");
    let key = c.secret_key(env.name(), &secret.name);
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("s3cret-root"));
    assert!(report.secrets_to_enter.is_empty(), "{report:?}");
}

#[test]
fn insomnia_environments_are_added_to_the_target() {
    let (_dir, mut c) = fresh();
    let b = parsed(&fixture("insomnia/bruno/insomnia-v4-with-envs.json"));
    assert!(!b.environments.is_empty());
    let report = place(&mut c, "ins", &b, &MemoryStore::new());
    let names: Vec<String> = b.environments.iter().map(|e| e.name.clone()).collect();
    assert_eq!(report.environments, names);
    for name in &names {
        assert!(
            c.root().join(format!("environments/{name}.yml")).is_file(),
            "{name}"
        );
    }
}

#[test]
fn an_environment_name_already_taken_is_added_under_a_new_name_with_a_note() {
    let (_dir, mut c) = fresh();
    let b = parsed(&fixture("insomnia/bruno/insomnia-v4-with-envs.json"));
    let first = b.environments[0].name.clone();
    c.create_environment(&first, &MemoryStore::new()).unwrap();
    let report = place(&mut c, "ins", &b, &MemoryStore::new());
    let renamed = format!("{first} copy");
    assert_eq!(report.environments[0], renamed);
    let file = format!("environments/{renamed}.yml");
    assert!(
        report.notes.iter().any(|n| n.kind == NoteKind::Renamed
            && n.file.as_deref() == Some(file.as_str())
            && n.detail.contains(&format!("`{first}`"))),
        "{:?}",
        report.notes
    );
}
