//! Setting and removing one environment variable, and storing a secret's value.

use apinata_core::{Collection, EnvError, MemoryStore, SecretStore};
use std::fs;
use tempfile::TempDir;

const ENV: &str =
    "name: dev\nvariables:\n  - name: host\n    value: a\n  - secret: true\n    name: token\n";

fn open() -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .expect("root");
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    fs::write(dir.path().join("environments/dev.yml"), ENV).expect("env");
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn file(dir: &TempDir) -> String {
    fs::read_to_string(dir.path().join("environments/dev.yml")).expect("read")
}

#[test]
fn set_creates_a_variable_and_a_repeat_leaves_the_file_identical() {
    let (dir, mut c) = open();
    assert!(c.set_variable_text("dev", "port", "80").expect("set"));
    let first = file(&dir);
    assert!(first.contains("name: port"), "{first}");
    assert!(!c.set_variable_text("dev", "port", "80").expect("again"));
    assert_eq!(file(&dir), first);
}

#[test]
fn set_changes_an_existing_plain_value_and_refuses_a_secret() {
    let (dir, mut c) = open();
    c.set_variable_text("dev", "host", "b").expect("set");
    assert!(file(&dir).contains("value: b"));
    let err = c
        .set_variable_text("dev", "token", "x")
        .expect_err("secret");
    assert!(matches!(err, EnvError::SecretVariable { .. }), "{err}");
    let err = c
        .set_variable_text("nope", "k", "x")
        .expect_err("unknown env");
    assert!(matches!(err, EnvError::NotFound { .. }), "{err}");
}

#[test]
fn remove_drops_a_plain_variable_and_refuses_a_secret_without_the_flag() {
    let (dir, mut c) = open();
    let store = MemoryStore::new();
    c.remove_variable("dev", "host", false, &store)
        .expect("remove");
    assert!(!file(&dir).contains("host"));
    let err = c
        .remove_variable("dev", "token", false, &store)
        .expect_err("secret");
    assert!(matches!(err, EnvError::SecretVariable { .. }), "{err}");
    let err = c
        .remove_variable("dev", "gone", false, &store)
        .expect_err("missing");
    assert!(matches!(err, EnvError::VariableNotFound { .. }), "{err}");
}

#[test]
fn removing_a_secret_deletes_its_stored_value() {
    let (dir, mut c) = open();
    let store = MemoryStore::new();
    let key = c.secret_key("dev", "token");
    store.set(&key, "s3").expect("seed");
    c.remove_variable("dev", "token", true, &store)
        .expect("remove");
    assert!(!file(&dir).contains("token"));
    assert_eq!(store.get(&key).expect("get"), None);
}

#[test]
fn a_secret_value_goes_to_the_store_and_never_to_the_file() {
    let (dir, mut c) = open();
    let store = MemoryStore::new();
    c.set_secret_value("dev", "apikey", "hunter2", &store)
        .expect("new");
    c.set_secret_value("dev", "token", "tok", &store)
        .expect("declared");
    c.set_secret_value("dev", "host", "moved", &store)
        .expect("plain becomes secret");
    let text = file(&dir);
    assert!(
        !text.contains("hunter2") && !text.contains("tok\n") && !text.contains("moved"),
        "{text}"
    );
    assert!(text.contains("name: apikey"), "{text}");
    let get = |v: &str| store.get(&c.secret_key("dev", v)).expect("get");
    assert_eq!(get("apikey").as_deref(), Some("hunter2"));
    assert_eq!(get("token").as_deref(), Some("tok"));
    assert_eq!(get("host").as_deref(), Some("moved"));
    c.set_secret_value("dev", "apikey", "hunter2", &store)
        .expect("repeat");
    assert_eq!(file(&dir), text);
}

#[test]
fn a_failing_store_writes_nothing() {
    use apinata_core::{SecretError, SecretKey};
    struct Down;
    impl SecretStore for Down {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Ok(None)
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Unavailable {
                message: "down".into(),
            })
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            Ok(())
        }
    }
    let (dir, mut c) = open();
    let err = c
        .set_secret_value("dev", "apikey", "v", &Down)
        .expect_err("fails");
    assert!(matches!(err, EnvError::SecretStore { .. }), "{err}");
    assert_eq!(file(&dir), ENV);
}
