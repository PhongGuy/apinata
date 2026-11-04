//! Orphaned secrets: what counts as one, and that removing one takes a named, current orphan.

use apinata_core::{
    Collection, EnvError, EnvWarning, GlobalEnvironments, MemoryStore, SecretError, SecretKey,
    SecretStore,
};
use std::fs;
use tempfile::TempDir;

const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: x\n  - name: live\n    secret: true\n  - name: off\n    secret: true\n    enabled: false\n";

fn tree(dev: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .unwrap();
    fs::create_dir(dir.path().join("environments")).unwrap();
    fs::write(dir.path().join("environments/dev.yml"), dev).unwrap();
    fs::write(dir.path().join("environments/prod.yml"), "name: prod\n").unwrap();
    dir
}

fn seeded(c: &Collection, env: &str, names: &[&str]) -> MemoryStore {
    let store = MemoryStore::new();
    for name in names {
        store.set(&c.secret_key(env, name), "v").unwrap();
    }
    store
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn stored(store: &dyn SecretStore, c: &Collection, env: &str, name: &str) -> bool {
    store.get(&c.secret_key(env, name)).unwrap().is_some()
}

/// Lists like its inner store but refuses to delete `locked`.
struct NoDelete {
    inner: MemoryStore,
    locked: &'static str,
}

impl SecretStore for NoDelete {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.inner.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        if key.variable == self.locked {
            return Err(SecretError::Unavailable {
                message: "locked".into(),
            });
        }
        self.inner.delete(key)
    }
    fn list(&self, collection: &std::path::Path) -> Result<Vec<SecretKey>, SecretError> {
        self.inner.list(collection)
    }
}

/// A store with the default `list`.
struct NoList;

impl SecretStore for NoList {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Ok(())
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        panic!("nothing may be deleted when the list failed")
    }
}

#[test]
fn an_orphan_is_a_stored_name_the_file_does_not_mark_secret() {
    let dir = tree(DEV);
    let c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["live", "off", "host", "gone"]);
    store.set(&c.secret_key("prod", "elsewhere"), "v").unwrap();
    assert_eq!(c.orphaned_secrets("dev", &store).unwrap(), ["gone", "host"]);
    assert_eq!(c.orphaned_secrets("prod", &store).unwrap(), ["elsewhere"]);
}

#[test]
fn a_duplicate_with_one_secret_entry_is_declared() {
    let dir =
        tree("name: dev\nvariables:\n  - name: a\n    value: x\n  - name: a\n    secret: true\n");
    let c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["a"]);
    assert!(c.orphaned_secrets("dev", &store).unwrap().is_empty());
}

#[test]
fn an_empty_file_declares_nothing_and_that_is_known() {
    for text in [
        "name: dev\n",
        "name: dev\nvariables:\n",
        "name: dev\nvariables: []\n",
    ] {
        let dir = tree(text);
        let c = Collection::open(dir.path()).unwrap();
        let store = seeded(&c, "dev", &["a"]);
        assert_eq!(c.orphaned_secrets("dev", &store).unwrap(), ["a"], "{text}");
    }
}

#[test]
fn a_malformed_variables_key_is_an_error_not_an_empty_answer() {
    for text in [
        "name: dev\nvariables: 5\n",
        "name: dev\nvariables:\n  a: b\n",
        "name: dev\nvariables:\n  - 5\n",
    ] {
        let dir = tree(text);
        let c = Collection::open(dir.path()).unwrap();
        let store = seeded(&c, "dev", &["live"]);
        let got = c.orphaned_secrets("dev", &store);
        assert!(
            matches!(got, Err(EnvError::Unreadable { .. })),
            "{text}: {got:?}"
        );
        let names = strings(&["live"]);
        let got = c.remove_orphaned_secrets("dev", &names, &store);
        assert!(matches!(got, Err(EnvError::Unreadable { .. })), "{text}");
        assert!(stored(&store, &c, "dev", "live"));
    }
}

#[test]
fn a_file_gone_after_load_is_an_error() {
    let dir = tree(DEV);
    let c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["live"]);
    fs::remove_file(dir.path().join("environments/dev.yml")).unwrap();
    assert!(matches!(
        c.orphaned_secrets("dev", &store),
        Err(EnvError::Unreadable { .. })
    ));
}

#[test]
fn an_unknown_environment_is_not_found() {
    let dir = tree(DEV);
    let c = Collection::open(dir.path()).unwrap();
    let store = MemoryStore::new();
    assert!(matches!(
        c.orphaned_secrets("nope", &store),
        Err(EnvError::NotFound { .. })
    ));
}

#[test]
fn a_store_that_cannot_list_is_an_error_and_deletes_nothing() {
    let dir = tree(DEV);
    let c = Collection::open(dir.path()).unwrap();
    assert!(matches!(
        c.orphaned_secrets("dev", &NoList),
        Err(EnvError::SecretList { .. })
    ));
    let got = c.remove_orphaned_secrets("dev", &strings(&["gone"]), &NoList);
    assert!(matches!(got, Err(EnvError::SecretList { .. })));
}

#[test]
fn removal_deletes_the_named_orphans_and_nothing_else() {
    let dir = tree(DEV);
    let c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["live", "gone", "host"]);
    let warnings = c
        .remove_orphaned_secrets("dev", &strings(&["gone"]), &store)
        .unwrap();
    assert!(warnings.is_empty());
    assert!(!stored(&store, &c, "dev", "gone"));
    assert!(stored(&store, &c, "dev", "host") && stored(&store, &c, "dev", "live"));
}

#[test]
fn a_declared_name_refuses_the_whole_removal() {
    let dir = tree(DEV);
    let c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["live", "gone"]);
    let got = c.remove_orphaned_secrets("dev", &strings(&["gone", "live"]), &store);
    match got {
        Err(EnvError::NotOrphaned { variables, .. }) => assert_eq!(variables, ["live"]),
        other => panic!("{other:?}"),
    }
    assert!(stored(&store, &c, "dev", "gone") && stored(&store, &c, "dev", "live"));
}

#[test]
fn a_mistyped_or_already_removed_name_is_refused() {
    let dir = tree(DEV);
    let c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["gone"]);
    let got = c.remove_orphaned_secrets("dev", &strings(&["gnoe"]), &store);
    assert!(matches!(got, Err(EnvError::NotOrphaned { .. })));
    assert!(stored(&store, &c, "dev", "gone"));
}

#[test]
fn a_name_declared_again_on_disk_since_load_is_refused() {
    let dir = tree("name: dev\n");
    let c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["a"]);
    assert_eq!(c.orphaned_secrets("dev", &store).unwrap(), ["a"]);
    // A checkout puts the declaration back; the loaded collection has not noticed.
    fs::write(
        dir.path().join("environments/dev.yml"),
        "name: dev\nvariables:\n  - name: a\n    secret: true\n",
    )
    .unwrap();
    let got = c.remove_orphaned_secrets("dev", &strings(&["a"]), &store);
    assert!(matches!(got, Err(EnvError::NotOrphaned { .. })));
    assert!(stored(&store, &c, "dev", "a"));
}

#[test]
fn a_failed_delete_is_a_warning_and_the_rest_still_go() {
    let dir = tree(DEV);
    let c = Collection::open(dir.path()).unwrap();
    let inner = seeded(&c, "dev", &["a", "b", "c"]);
    let store = NoDelete { inner, locked: "b" };
    let warnings = c
        .remove_orphaned_secrets("dev", &strings(&["a", "b", "c"]), &store)
        .unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(&warnings[0], EnvWarning::Secret { variable, .. } if variable == "b"));
    assert!(!stored(&store, &c, "dev", "a") && !stored(&store, &c, "dev", "c"));
    assert!(stored(&store, &c, "dev", "b"));
}

#[test]
fn a_reload_and_a_save_leave_an_orphans_value_in_place() {
    let dir = tree("name: dev\nvariables:\n  - name: a\n    secret: true\n");
    let mut c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["a"]);
    fs::write(
        dir.path().join("environments/dev.yml"),
        "name: dev\nvariables:\n  - name: a\n    value: plain\n",
    )
    .unwrap();
    c.reload();
    assert_eq!(c.orphaned_secrets("dev", &store).unwrap(), ["a"]);
    c.set_variable_text("dev", "b", "1").unwrap();
    assert!(stored(&store, &c, "dev", "a"));
}

#[test]
fn a_global_environment_works_the_same_and_the_app_keys_stay_out() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("g.yml"),
        "name: g\nvariables:\n  - name: live\n    secret: true\n",
    )
    .unwrap();
    let globals = GlobalEnvironments::open(dir.path()).unwrap();
    let store = MemoryStore::new();
    store.set(&globals.secret_key("g", "live"), "v").unwrap();
    store.set(&globals.secret_key("g", "gone"), "v").unwrap();
    store
        .set(&SecretKey::new("", "", "oauth2-token-key"), "k")
        .unwrap();
    store
        .set(&SecretKey::new("", "app", "proxy-password"), "k")
        .unwrap();
    assert_eq!(globals.orphaned_secrets("g", &store).unwrap(), ["gone"]);
    let warnings = globals
        .remove_orphaned_secrets("g", &strings(&["gone"]), &store)
        .unwrap();
    assert!(warnings.is_empty());
    assert_eq!(store.get(&globals.secret_key("g", "gone")).unwrap(), None);
    assert!(
        store
            .get(&globals.secret_key("g", "live"))
            .unwrap()
            .is_some()
    );
    let got = globals.remove_orphaned_secrets("g", &strings(&["live"]), &store);
    assert!(matches!(got, Err(EnvError::NotOrphaned { .. })));
}

const PLAIN_EMPTY: &str = "name: dev\nvariables:\n  - name: token\n    value: ''\n";

#[test]
fn making_an_empty_variable_secret_keeps_the_stored_value() {
    let dir = tree(PLAIN_EMPTY);
    let mut c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["token"]);
    let key = c.secret_key("dev", "token");
    c.set_variable_secret("dev", "token", true, &store).unwrap();
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("v"));
    assert!(c.environment("dev").unwrap().variables()[0].secret);
}

#[test]
fn making_it_secret_and_plain_again_moves_the_stored_value_into_the_file() {
    let dir = tree(PLAIN_EMPTY);
    let mut c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["token"]);
    c.set_variable_secret("dev", "token", true, &store).unwrap();
    c.set_variable_secret("dev", "token", false, &store)
        .unwrap();
    assert!(!stored(&store, &c, "dev", "token"));
    let text = fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
    assert!(text.contains("value: v"), "{text}");
}

#[test]
fn a_global_empty_variable_made_secret_keeps_the_stored_value() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("g.yml"), PLAIN_EMPTY.replace("dev", "g")).unwrap();
    let mut globals = GlobalEnvironments::open(dir.path()).unwrap();
    let store = MemoryStore::new();
    let key = globals.secret_key("g", "token");
    store.set(&key, "v").unwrap();
    globals
        .set_variable_secret("g", "token", true, &store)
        .unwrap();
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("v"));
}

#[test]
fn a_plain_value_with_text_still_replaces_the_stored_one() {
    let dir = tree("name: dev\nvariables:\n  - name: token\n    value: new\n");
    let mut c = Collection::open(dir.path()).unwrap();
    let store = seeded(&c, "dev", &["token"]);
    let key = c.secret_key("dev", "token");
    c.set_variable_secret("dev", "token", true, &store).unwrap();
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("new"));
}

#[test]
fn an_empty_variable_with_nothing_stored_still_becomes_secret() {
    let dir = tree(PLAIN_EMPTY);
    let mut c = Collection::open(dir.path()).unwrap();
    let store = MemoryStore::new();
    let key = c.secret_key("dev", "token");
    c.set_variable_secret("dev", "token", true, &store).unwrap();
    assert!(c.environment("dev").unwrap().variables()[0].secret);
    assert_eq!(store.get(&key).unwrap().as_deref(), Some(""));
}
