//! A failed write in an environment operation leaves memory, disk and the secret store
//! agreeing with each other.

use apinata_core::{Collection, EnvError, MemoryStore, SaveError, SecretStore};
use std::fs;
use tempfile::TempDir;

const DEV: &str = "name: dev\nvariables:\n  - name: token\n    value: abc\n";
const DEV_SECRET: &str = "name: dev\nvariables:\n  - secret: true\n    name: token\n";
const THEIRS: &str = "name: dev\nvariables:\n  - name: token\n    value: theirs\n";

fn collection(env: &str) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .expect("root");
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    fs::write(dir.path().join("environments/dev.yml"), env).expect("env");
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn env_path(dir: &TempDir, name: &str) -> std::path::PathBuf {
    dir.path().join("environments").join(format!("{name}.yml"))
}

#[test]
fn a_failed_save_after_storing_a_secret_undoes_the_flag_and_the_stored_value() {
    let (dir, mut c) = collection(DEV);
    let store = MemoryStore::new();
    // Someone else edits the file, so the save that drops the value conflicts.
    fs::write(env_path(&dir, "dev"), THEIRS).expect("write");
    let err = c.set_variable_secret("dev", "token", true, &store);
    assert!(
        matches!(err, Err(EnvError::Save(SaveError::Conflict { .. }))),
        "{err:?}"
    );
    let dev = c.environment("dev").expect("dev");
    assert!(!dev.variables()[0].secret, "the draft is not secret");
    assert!(!dev.is_dirty(), "the draft is as before the call");
    let key = c.secret_key("dev", "token");
    assert_eq!(
        store.get(&key).expect("get"),
        None,
        "the stored copy is gone"
    );
    assert_eq!(
        fs::read_to_string(env_path(&dir, "dev")).expect("read"),
        THEIRS
    );
}

#[test]
fn a_failed_save_making_an_empty_variable_secret_keeps_the_stored_value() {
    let (dir, mut c) = collection("name: dev\nvariables:\n  - name: token\n    value: ''\n");
    let store = MemoryStore::new();
    let key = c.secret_key("dev", "token");
    store.set(&key, "kept").expect("set");
    fs::write(env_path(&dir, "dev"), THEIRS).expect("write");
    let err = c.set_variable_secret("dev", "token", true, &store);
    assert!(
        matches!(err, Err(EnvError::Save(SaveError::Conflict { .. }))),
        "{err:?}"
    );
    assert_eq!(store.get(&key).expect("get").as_deref(), Some("kept"));
    assert!(!c.environment("dev").expect("dev").variables()[0].secret);
}

#[test]
fn a_failed_save_after_reading_a_secret_keeps_it_secret_and_stored() {
    let (dir, mut c) = collection(DEV_SECRET);
    let store = MemoryStore::new();
    let key = c.secret_key("dev", "token");
    store.set(&key, "abc").expect("set");
    fs::write(env_path(&dir, "dev"), THEIRS).expect("write");
    let err = c.set_variable_secret("dev", "token", false, &store);
    assert!(matches!(err, Err(EnvError::Save(_))), "{err:?}");
    let dev = c.environment("dev").expect("dev");
    assert!(dev.variables()[0].secret);
    assert!(!dev.is_dirty());
    assert_eq!(store.get(&key).expect("get").as_deref(), Some("abc"));
}

#[test]
fn a_failed_file_rename_restores_the_old_name_in_the_file() {
    let (dir, mut c) = collection(DEV);
    let store = MemoryStore::new();
    // A valid name whose file name exceeds the file system's 255-byte limit.
    let long = "x".repeat(255);
    let err = c.rename_environment("dev", &long, &store);
    assert!(
        matches!(err, Err(EnvError::Save(SaveError::Io { .. }))),
        "{err:?}"
    );
    assert_eq!(
        fs::read_to_string(env_path(&dir, "dev")).expect("read"),
        DEV
    );
    let dev = c.environment("dev").expect("still dev");
    assert_eq!(dev.path(), env_path(&dir, "dev"));
    assert!(!dev.is_dirty());
}

#[test]
fn a_conflicting_save_before_an_environment_rename_keeps_the_old_name_in_the_draft() {
    let (dir, mut c) = collection(DEV);
    let store = MemoryStore::new();
    fs::write(env_path(&dir, "dev"), THEIRS).expect("their edit");
    let err = c.rename_environment("dev", "prod", &store);
    assert!(
        matches!(err, Err(EnvError::Save(SaveError::Conflict { .. }))),
        "{err:?}"
    );
    assert_eq!(
        fs::read_to_string(env_path(&dir, "dev")).expect("read"),
        THEIRS
    );
    let dev = c.environment("dev").expect("still dev");
    assert!(!dev.is_dirty(), "the draft does not carry the new name");
}

#[test]
fn a_failed_save_keeps_a_secret_value_stored_before_the_call() {
    let (dir, mut c) = collection(DEV);
    let store = MemoryStore::new();
    let key = c.secret_key("dev", "token");
    store.set(&key, "earlier").expect("set");
    fs::write(env_path(&dir, "dev"), THEIRS).expect("write");
    let err = c.set_variable_secret("dev", "token", true, &store);
    assert!(
        matches!(err, Err(EnvError::Save(SaveError::Conflict { .. }))),
        "{err:?}"
    );
    assert_eq!(
        store.get(&key).expect("get").as_deref(),
        Some("earlier"),
        "the earlier stored value is restored"
    );
}
