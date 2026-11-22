//! When the secret store fails during an import, the report says so once, in plain words: one
//! note names every secret left without a value, no note carries the platform's error text, and
//! no note claims a value was kept that was not.

use crate::import_secrets::{all_text, postman};
use apinata_core::import::{
    CollectionOptions, ImportNote, ImportReport, add_environment, add_request, parse_collections,
    parse_environments, parse_request, place_items, store_request_secrets, write_collection,
};
use apinata_core::{Collection, ItemPath, MemoryStore, SecretError, SecretKey, SecretStore};
use tempfile::TempDir;

const RAW: &str = "Platform failure: zbus error: Failed to connect to address \
                   `unix:path=/nonexistent`: No such file or directory (os error 2)";

/// A store that fails every call for the variables in `refused` (all when empty) and keeps the
/// rest in memory.
struct Failing {
    unavailable: bool,
    refused: &'static [&'static str],
    kept: MemoryStore,
}

impl Failing {
    fn down() -> Failing {
        Failing {
            unavailable: true,
            refused: &[],
            kept: MemoryStore::new(),
        }
    }

    fn fails(&self, key: &SecretKey) -> Option<SecretError> {
        let hit = self.refused.is_empty() || self.refused.contains(&key.variable.as_str());
        let message = RAW.to_owned();
        hit.then(|| match self.unavailable {
            true => SecretError::Unavailable { message },
            false => SecretError::Failed { message },
        })
    }
}

impl SecretStore for Failing {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        match self.fails(key) {
            Some(e) => Err(e),
            None => self.kept.get(key),
        }
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        match self.fails(key) {
            Some(e) => Err(e),
            None => self.kept.set(key, value),
        }
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
}

const TWO_TOKENS: &str = r#"{"name":"one","request":{"method":"GET","url":"https://x.test/1",
    "auth":{"type":"bearer","bearer":[{"key":"token","value":"tok-one-s3cret"}]}}},
  {"name":"two","request":{"method":"GET","url":"https://x.test/2",
    "auth":{"type":"bearer","bearer":[{"key":"token","value":"tok-two-s3cret"}]}}}"#;

fn write(store: &dyn SecretStore, items: &str) -> (TempDir, Collection, ImportReport) {
    let parent = TempDir::new().expect("tempdir");
    let input = postman(items);
    let bundles =
        parse_collections(None, input.as_bytes(), &CollectionOptions::default()).expect("parses");
    let (c, report) = write_collection(
        parent.path(),
        &bundles[0].collection,
        &bundles[0].environments,
        store,
    )
    .expect("writes");
    (parent, c, report)
}

/// The notes that mention the keychain.
fn keychain_notes(notes: &[ImportNote]) -> Vec<&ImportNote> {
    notes
        .iter()
        .filter(|n| n.detail.contains("keychain"))
        .collect()
}

/// No note shows the platform's words or claims a value went to the store.
fn assert_plain(notes: &[ImportNote]) {
    for note in notes {
        let d = &note.detail;
        for raw in [
            "zbus",
            "os error",
            "secret store unavailable",
            "Platform failure",
        ] {
            assert!(!d.contains(raw), "raw store text in a note: {d}");
        }
        assert!(
            !d.contains("kept in the secret store"),
            "a note claims a value was kept: {d}"
        );
    }
}

#[test]
fn a_collection_import_with_no_keychain_says_so_once_and_names_every_secret() {
    let (_p, c, report) = write(&Failing::down(), TWO_TOKENS);
    assert_plain(&report.notes);
    let notes = keychain_notes(&report.notes);
    assert_eq!(notes.len(), 1, "{:#?}", report.notes);
    let d = &notes[0].detail;
    assert!(d.contains("did not answer"), "{d}");
    assert!(
        d.contains("`one_token`") && d.contains("`two_token`"),
        "{d}"
    );
    assert!(d.contains("--env-var") && d.contains(".env"), "{d}");
    assert_eq!(report.secrets_to_enter.len(), 2);
    let files = all_text(c.root());
    assert!(!files.contains("tok-one-s3cret") && !files.contains("tok-two-s3cret"));
}

#[test]
fn a_store_that_answers_but_fails_is_named_in_plain_words_too() {
    let failing = Failing {
        unavailable: false,
        ..Failing::down()
    };
    let (_p, _c, report) = write(&failing, TWO_TOKENS);
    assert_plain(&report.notes);
    let notes = keychain_notes(&report.notes);
    assert_eq!(notes.len(), 1, "{:#?}", report.notes);
    assert!(notes[0].detail.contains("could not store"), "{notes:?}");
}

#[test]
fn only_the_values_actually_stored_are_said_to_be_kept() {
    let failing = Failing {
        refused: &["two_token"],
        ..Failing::down()
    };
    let (_p, _c, report) = write(&failing, TWO_TOKENS);
    let kept: Vec<&ImportNote> = report
        .notes
        .iter()
        .filter(|n| n.detail.contains("kept in the secret store"))
        .collect();
    assert_eq!(kept.len(), 1, "{:#?}", report.notes);
    assert!(kept[0].detail.contains("`one_token`"), "{kept:?}");
    assert!(!kept[0].detail.contains("`two_token`"), "{kept:?}");
    let failed = keychain_notes(&report.notes);
    assert_eq!(failed.len(), 1, "{:#?}", report.notes);
    assert!(failed[0].detail.contains("`two_token`"));
    assert!(!failed[0].detail.contains("`one_token`"));
}

#[test]
fn many_secrets_are_named_briefly() {
    let items: Vec<String> = (1..=6)
        .map(|i| {
            format!(
                r#"{{"name":"r{i}","request":{{"method":"GET","url":"https://x.test/{i}",
                "auth":{{"type":"bearer","bearer":[{{"key":"token","value":"tok-{i}"}}]}}}}}}"#
            )
        })
        .collect();
    let (_p, _c, report) = write(&Failing::down(), &items.join(","));
    let notes = keychain_notes(&report.notes);
    assert_eq!(notes.len(), 1, "{:#?}", report.notes);
    let d = &notes[0].detail;
    assert!(d.contains("`r1_token`") && d.contains("3 more"), "{d}");
    assert!(!d.contains("`r6_token`"), "{d}");
}

#[test]
fn a_request_import_with_no_keychain_has_one_plain_note() {
    let dir = TempDir::new().expect("tempdir");
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    c.create_environment("dev", &MemoryStore::new())
        .expect("env");
    let store = Failing::down();
    let mut parsed = parse_request(
        None,
        "curl -H 'Authorization: Bearer abc123' https://x.test/z",
    )
    .expect("parses");
    add_request(&mut c, &ItemPath::root(), "z", &parsed.request).expect("adds");
    store_request_secrets(&mut c, &parsed.request, None, &store, &mut parsed.notes);
    assert_plain(&parsed.notes);
    let notes = keychain_notes(&parsed.notes);
    assert_eq!(notes.len(), 1, "{:#?}", parsed.notes);
    assert!(notes[0].detail.contains("did not answer"), "{notes:?}");
    assert!(
        notes[0]
            .detail
            .contains(&format!("`{}`", parsed.request.secrets[0].name)),
        "{notes:?}"
    );
}

const ENV: &str = r#"{"name":"E","values":[
  {"key":"a","value":"va","type":"secret","enabled":true},
  {"key":"b","value":"vb","type":"secret","enabled":true}]}"#;

#[test]
fn an_environment_import_with_no_keychain_has_one_plain_note() {
    let dir = TempDir::new().expect("tempdir");
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    let envs = parse_environments(None, ENV.as_bytes()).expect("parses");
    let (_, report) = add_environment(&mut c, &envs[0], &Failing::down()).expect("adds");
    assert_plain(&report.notes);
    let notes = keychain_notes(&report.notes);
    assert_eq!(notes.len(), 1, "{:#?}", report.notes);
    assert!(notes[0].detail.contains("`a`") && notes[0].detail.contains("`b`"));
    assert_eq!(report.secrets_to_enter.len(), 2);
}

#[test]
fn merged_reports_keep_one_keychain_note() {
    let dir = TempDir::new().expect("tempdir");
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    let envs = parse_environments(None, ENV.as_bytes()).expect("parses");
    let (_, first) = add_environment(&mut c, &envs[0], &Failing::down()).expect("adds");
    let (_, second) = add_environment(&mut c, &envs[0], &Failing::down()).expect("adds");
    let mut merged = ImportReport::default();
    merged.merge(first);
    merged.merge(second);
    let notes = keychain_notes(&merged.notes);
    assert_eq!(notes.len(), 1, "{:#?}", merged.notes);
    assert_eq!(merged.environments.len(), 2);
    assert_eq!(merged.secrets_to_enter.len(), 4);
}

#[test]
fn placing_items_with_no_keychain_has_one_plain_note() {
    let dir = TempDir::new().expect("tempdir");
    let mut c = Collection::create(&dir.path().join("c"), "c").expect("create");
    c.create_environment("dev", &MemoryStore::new())
        .expect("env");
    let input = postman(TWO_TOKENS);
    let bundles =
        parse_collections(None, input.as_bytes(), &CollectionOptions::default()).expect("parses");
    let placed =
        place_items(&mut c, &ItemPath::root(), &bundles[0], &Failing::down()).expect("places");
    assert_plain(&placed.report.notes);
    let notes = keychain_notes(&placed.report.notes);
    assert_eq!(notes.len(), 1, "{:#?}", placed.report.notes);
    assert!(notes[0].detail.contains("`one_token`") && notes[0].detail.contains("`two_token`"));
}
