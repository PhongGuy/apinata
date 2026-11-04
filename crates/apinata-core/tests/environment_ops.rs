//! Environment create and delete: Bruno's names and bytes, the in-memory collection kept in
//! step, and secret removal whose failures are warnings.

use crate::config_parity;

use apinata_core::{
    Collection, EnvError, EnvWarning, MemoryStore, SecretError, SecretKey, SecretStore,
};
use config_parity::check;
use std::fs;
use tempfile::TempDir;

/// A store whose deletes fail, as a locked keychain's would.
struct LockedForDelete(MemoryStore);

impl SecretStore for LockedForDelete {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.0.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.0.set(key, value)
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
}

/// A collection with the given environment files.
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

fn names(c: &Collection) -> Vec<&str> {
    c.environments().iter().map(|e| e.name()).collect()
}

#[test]
fn create_matches_bruno() {
    check("env-op-create", |id, c| {
        let (asked, used) = match id {
            "env-op-create" => ("dev", "dev copy"),
            "env-op-create-sanitized" => (" a/b?. ", "a-b-"),
            "env-op-create-fresh" => ("local", "local"),
            other => panic!("unexpected case {other}"),
        };
        let (name, warnings) = c
            .create_environment(asked, &MemoryStore::new())
            .expect("creates");
        assert_eq!(name, used);
        assert!(warnings.is_empty());
        assert!(
            c.environment(used).is_some(),
            "{id}: the new environment is listed"
        );
    });
}

#[test]
fn create_counts_suffixes_and_files_that_failed_to_load() {
    let dir = collection_with(&[("dev", "name: dev\n"), ("dev copy", "name: [broken\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    assert_eq!(names(&c), ["dev"]);
    let store = MemoryStore::new();
    let (first, _) = c.create_environment("dev", &store).expect("creates");
    let (second, _) = c.create_environment("dev", &store).expect("creates");
    assert_eq!(
        (first.as_str(), second.as_str()),
        ("dev copy 2", "dev copy 3")
    );
    assert_eq!(names(&c), ["dev", "dev copy 2", "dev copy 3"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("environments/dev copy 3.yml")).unwrap(),
        "name: dev copy 3\n"
    );
}

#[test]
fn create_refuses_a_name_that_sanitises_to_nothing_usable() {
    let dir = collection_with(&[]);
    let mut c = Collection::open(dir.path()).expect("opens");
    for bad in ["", " ... ", "con"] {
        let err = c
            .create_environment(bad, &MemoryStore::new())
            .expect_err("refused");
        assert!(
            matches!(err, EnvError::InvalidName { .. }),
            "{bad:?}: {err:?}"
        );
    }
    assert!(c.environments().is_empty());
}

#[test]
fn delete_removes_the_file_and_its_secrets() {
    let dir = collection_with(&[
        (
            "dev",
            "name: dev\nvariables:\n  - secret: true\n    name: token\n  - name: plain\n    value: p\n",
        ),
        (
            "prod",
            "name: prod\nvariables:\n  - secret: true\n    name: token\n",
        ),
    ]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    store
        .set(&c.secret_key("dev", "token"), "dev-secret")
        .unwrap();
    store
        .set(&c.secret_key("prod", "token"), "prod-secret")
        .unwrap();

    let warnings = c.delete_environment("dev", &store).expect("deletes");
    assert!(warnings.is_empty());
    assert!(!dir.path().join("environments/dev.yml").exists());
    assert_eq!(names(&c), ["prod"]);
    assert_eq!(store.get(&c.secret_key("dev", "token")), Ok(None));
    assert_eq!(
        store.get(&c.secret_key("prod", "token")),
        Ok(Some("prod-secret".into()))
    );
}

#[test]
fn a_failed_secret_delete_is_a_warning_and_the_file_stays_deleted() {
    let dir = collection_with(&[(
        "dev",
        "name: dev\nvariables:\n  - secret: true\n    name: token\n",
    )]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let warnings = c
        .delete_environment("dev", &LockedForDelete(MemoryStore::new()))
        .expect("the file operation succeeds");
    assert_eq!(
        warnings,
        [EnvWarning::Secret {
            environment: "dev".into(),
            variable: "token".into(),
            error: SecretError::Unavailable {
                message: "locked".into()
            },
        }]
    );
    assert!(!dir.path().join("environments/dev.yml").exists());
}

#[test]
fn delete_of_an_unknown_environment_is_not_found() {
    let dir = collection_with(&[]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .delete_environment("nope", &MemoryStore::new())
        .expect_err("refused");
    assert!(
        matches!(err, EnvError::NotFound { ref name, .. } if name == "nope"),
        "{err:?}"
    );
}

/// A hand-written file: name-first secret, Bruno-order secret, extra keys, and a plain entry.
const HAND: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n  - secret: true\n    name: key\n  - name: host\n    value: a\n    description: the host\n    extra: 1\n  - name: pw\n    description: pass\n    value: x\n    disabled: true\n";

fn dev_file(dir: &TempDir) -> String {
    fs::read_to_string(dir.path().join("environments/dev.yml")).expect("reads")
}

#[test]
fn secret_edits_of_one_variable_keep_the_other_entries_byte_identical() {
    let dir = collection_with(&[("dev", HAND)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    let others = &HAND[..HAND.find("  - name: pw").unwrap()];
    c.set_secret_value("dev", "pw", "s3", &store).expect("sets");
    assert!(dev_file(&dir).contains(others), "{}", dev_file(&dir));
    c.remove_variable("dev", "pw", true, &store)
        .expect("removes");
    assert!(dev_file(&dir).contains(others), "{}", dev_file(&dir));
}

#[test]
fn delete_refuses_an_environment_others_extend_naming_them_and_keeps_everything() {
    let dev = "name: dev\nvariables:\n  - secret: true\n    name: token\n";
    let dir = collection_with(&[
        ("dev", dev),
        ("b-child", "name: b-child\nextends: \" dev \"\n"),
        ("a-child", "name: a-child\nextends: dev\n"),
        ("prod", "name: prod\n"),
    ]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    store.set(&c.secret_key("dev", "token"), "s3cret").unwrap();
    let err = c.delete_environment("dev", &store).expect_err("refused");
    assert!(
        matches!(&err, EnvError::ExtendedBy { name, by, global: false }
            if name == "dev" && by == &["a-child", "b-child"]),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "environment `dev` is extended by `a-child`, `b-child`; change `extends` in each first"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap(),
        dev
    );
    assert!(c.environment("dev").is_some());
    assert_eq!(
        store.get(&c.secret_key("dev", "token")),
        Ok(Some("s3cret".into()))
    );
}

#[test]
fn delete_of_an_environment_extending_itself_goes_ahead() {
    let dir = collection_with(&[("dev", "name: dev\nextends: dev\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    c.delete_environment("dev", &MemoryStore::new())
        .expect("deletes");
    assert!(!dir.path().join("environments/dev.yml").exists());
}

#[test]
fn delete_of_an_unknown_name_is_not_found_even_when_something_extends_it() {
    let dir = collection_with(&[("child", "name: child\nextends: ghost\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .delete_environment("ghost", &MemoryStore::new())
        .expect_err("refused");
    assert!(matches!(err, EnvError::NotFound { .. }), "{err:?}");
}

#[test]
fn extended_by_lists_the_other_extenders_in_name_order() {
    let dir = collection_with(&[
        ("dev", "name: dev\nextends: dev\n"),
        ("z", "name: z\nextends: dev\n"),
        ("m", "name: m\nextends: dev\n"),
        ("prod", "name: prod\nextends: other\n"),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert_eq!(c.extended_by("dev"), ["m", "z"]);
    assert!(c.extended_by("prod").is_empty());
    assert!(c.extended_by("nope").is_empty());
}
