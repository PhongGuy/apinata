//! Duplicating an environment in a collection and in the global set: the copy's name, bytes,
//! colour, `extends` and secret values.

use apinata_core::{
    Collection, EnvWarning, GlobalEnvironments, MemoryStore, SecretError, SecretKey, SecretStore,
};
use std::fs;
use tempfile::TempDir;

/// Written in the writer's own layout, so the copy must equal it apart from the name.
const DEV: &str = "name: dev\nextends: base\ncolor: \"#ff0000\"\nvariables:\n  - name: host\n    value: example.test\n  - secret: true\n    name: tok\n  - secret: true\n    name: unset\n  - name: off\n    value: x\n    enabled: false\n";

fn collection_with(envs: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .expect("root");
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    for (name, body) in envs {
        fs::write(
            dir.path().join("environments").join(format!("{name}.yml")),
            body,
        )
        .expect("env");
    }
    dir
}

#[test]
fn a_collection_environment_is_copied_with_bytes_colour_extends_and_secrets() {
    let dir = collection_with(&[("base", "name: base\n"), ("dev", DEV)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    store
        .set(&c.secret_key("dev", "tok"), "s3cret")
        .expect("set");

    let (name, warnings) = c.duplicate_environment("dev", &store).expect("copies");
    assert_eq!(name, "dev copy");
    assert!(warnings.is_empty(), "{warnings:?}");

    let copy = fs::read_to_string(dir.path().join("environments/dev copy.yml")).expect("file");
    assert_eq!(copy, DEV.replacen("name: dev\n", "name: dev copy\n", 1));
    let env = c.environment("dev copy").expect("listed");
    assert_eq!(env.color().as_deref(), Some("#ff0000"));
    assert_eq!(env.extends().as_deref(), Some("base"));
    assert_eq!(
        store
            .get(&c.secret_key("dev copy", "tok"))
            .expect("get")
            .as_deref(),
        Some("s3cret")
    );
    assert_eq!(
        store.get(&c.secret_key("dev copy", "unset")).expect("get"),
        None
    );
    // The original keeps its value.
    assert_eq!(
        store
            .get(&c.secret_key("dev", "tok"))
            .expect("get")
            .as_deref(),
        Some("s3cret")
    );
}

#[test]
fn copy_names_stay_unique() {
    let dir = collection_with(&[("dev", DEV), ("dev copy", "name: dev copy\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    let (first, _) = c.duplicate_environment("dev", &store).expect("copies");
    let (second, _) = c.duplicate_environment("dev", &store).expect("copies");
    assert_eq!(
        (first.as_str(), second.as_str()),
        ("dev copy 2", "dev copy 3")
    );
}

#[test]
fn an_unknown_environment_is_not_found() {
    let dir = collection_with(&[]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let error = c
        .duplicate_environment("nope", &MemoryStore::new())
        .expect_err("fails");
    assert!(matches!(error, apinata_core::EnvError::NotFound { .. }));
}

/// A store that cannot write.
struct ReadOnly(MemoryStore);

impl SecretStore for ReadOnly {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.0.get(key)
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.0.delete(key)
    }
}

#[test]
fn a_failed_secret_copy_is_a_warning_and_the_copy_stands() {
    let dir = collection_with(&[("dev", DEV)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let inner = MemoryStore::new();
    inner
        .set(&c.secret_key("dev", "tok"), "s3cret")
        .expect("set");
    let (name, warnings) = c
        .duplicate_environment("dev", &ReadOnly(inner))
        .expect("copies");
    assert_eq!(name, "dev copy");
    assert!(dir.path().join("environments/dev copy.yml").exists());
    assert!(matches!(
        warnings.as_slice(),
        [EnvWarning::Secret { environment, variable, .. }]
            if environment == "dev copy" && variable == "tok"
    ));
}

#[test]
fn a_global_environment_is_copied_the_same_way() {
    let profile = TempDir::new().expect("tempdir");
    let dir = profile.path().join("global-environments");
    fs::create_dir(&dir).expect("mkdir");
    fs::write(dir.join("dev.yml"), DEV).expect("env");
    let mut globals = GlobalEnvironments::open(&dir).expect("opens");
    let store = MemoryStore::new();
    store
        .set(&globals.secret_key("dev", "tok"), "s3cret")
        .expect("set");

    let (name, warnings) = globals
        .duplicate_environment("dev", &store)
        .expect("copies");
    assert_eq!(name, "dev copy");
    assert!(warnings.is_empty());
    assert_eq!(
        fs::read_to_string(dir.join("dev copy.yml")).expect("file"),
        DEV.replacen("name: dev\n", "name: dev copy\n", 1)
    );
    assert_eq!(
        store
            .get(&globals.secret_key("dev copy", "tok"))
            .expect("get")
            .as_deref(),
        Some("s3cret")
    );
    assert_eq!(
        store
            .get(&globals.secret_key("dev copy", "unset"))
            .expect("get"),
        None
    );
    let (second, _) = globals
        .duplicate_environment("dev", &store)
        .expect("copies");
    assert_eq!(second, "dev copy 2");
}
