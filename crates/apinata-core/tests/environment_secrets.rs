//! Environment rename and the secret flag: Bruno's bytes, secret values moved through the
//! store, and store failures reported as warnings while the file change stands.

use crate::config_parity;

use apinata_core::{
    Collection, EnvError, EnvWarning, MemoryStore, SecretError, SecretKey, SecretStore,
};
use config_parity::check;
use std::fs;
use tempfile::TempDir;

/// A store where every call fails, as with no keychain service running.
struct Unavailable;

impl SecretStore for Unavailable {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Err(SecretError::Unavailable {
            message: "no service".into(),
        })
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "no service".into(),
        })
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "no service".into(),
        })
    }
}

fn unavailable(environment: &str, variable: &str) -> EnvWarning {
    EnvWarning::Secret {
        environment: environment.into(),
        variable: variable.into(),
        error: SecretError::Unavailable {
            message: "no service".into(),
        },
    }
}

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

fn read_env(dir: &TempDir, name: &str) -> String {
    fs::read_to_string(dir.path().join("environments").join(format!("{name}.yml"))).expect("read")
}

#[test]
fn rename_matches_bruno_and_moves_the_secrets() {
    check("env-op-rename", |_, c| {
        let store = MemoryStore::new();
        store
            .set(&c.secret_key("base", "base_token"), "tok")
            .unwrap();
        let warnings = c
            .rename_environment("base", "root", &store)
            .expect("renames");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!c.root().join("environments/base.yml").exists());
        assert!(c.environment("base").is_none());
        assert_eq!(
            c.environment("dev").unwrap().extends().as_deref(),
            Some("root")
        );
        assert_eq!(
            store.get(&c.secret_key("root", "base_token")),
            Ok(Some("tok".into()))
        );
        assert_eq!(store.get(&c.secret_key("base", "base_token")), Ok(None));
    });
}

#[test]
fn rename_refuses_taken_invalid_and_unknown_names() {
    let dir = collection_with(&[
        ("a", "name: a\n"),
        ("b", "name: b\n"),
        ("c", "name: [broken\n"),
    ]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    assert!(matches!(
        c.rename_environment("a", "b", &store),
        Err(EnvError::Exists { .. })
    ));
    assert!(matches!(
        c.rename_environment("a", "c", &store),
        Err(EnvError::Exists { .. })
    ));
    assert!(matches!(
        c.rename_environment("a", "x/y", &store),
        Err(EnvError::InvalidName { .. })
    ));
    assert!(matches!(
        c.rename_environment("zz", "q", &store),
        Err(EnvError::NotFound { .. })
    ));
    assert_eq!(
        c.rename_environment("a", "a", &store).expect("no change"),
        []
    );
    assert_eq!(read_env(&dir, "a"), "name: a\n");
}

#[test]
fn a_failed_secret_move_is_a_warning_and_the_rename_stands() {
    let dir = collection_with(&[(
        "dev",
        "name: dev\nvariables:\n  - secret: true\n    name: token\n",
    )]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let warnings = c
        .rename_environment("dev", "stage", &Unavailable)
        .expect("renames");
    assert_eq!(warnings, [unavailable("stage", "token")]);
    assert_eq!(
        read_env(&dir, "stage"),
        "name: stage\nvariables:\n  - secret: true\n    name: token\n"
    );
}

#[test]
fn the_secret_flag_matches_bruno() {
    check("env-secret-", |id, c| {
        let (env, var, secret, stored) = match id {
            "env-secret-on" => ("dev", "api_url", true, "https://dev.example.com"),
            "env-secret-on-typed" => ("base", "base_number", true, "42"),
            "env-secret-off" => ("staging", "api_key", false, "from-store"),
            "env-secret-off-typed" => ("base", "base_secret_object", false, "{\"a\":1}"),
            other => panic!("unexpected case {other}"),
        };
        let store = MemoryStore::new();
        let key = c.secret_key(env, var);
        if !secret {
            store.set(&key, stored).unwrap();
        }
        let warnings = c
            .set_variable_secret(env, var, secret, &store)
            .expect("changes");
        assert!(warnings.is_empty(), "{id}: {warnings:?}");
        let expected = if secret {
            Some(stored.to_owned())
        } else {
            None
        };
        assert_eq!(
            store.get(&key),
            Ok(expected),
            "{id}: store after the change"
        );
    });
}

#[test]
fn a_secret_without_a_stored_value_becomes_an_empty_plain_value() {
    let dir = collection_with(&[("e", "name: e\nvariables:\n  - secret: true\n    name: t\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let warnings = c
        .set_variable_secret("e", "t", false, &MemoryStore::new())
        .expect("changes");
    assert!(warnings.is_empty());
    assert_eq!(
        read_env(&dir, "e"),
        "name: e\nvariables:\n  - name: t\n    value: \"\"\n"
    );
}

#[test]
fn an_unreadable_secret_refuses_the_change_and_keeps_the_file() {
    let body = "name: e\nvariables:\n  - secret: true\n    name: t\n";
    let dir = collection_with(&[("e", body)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .set_variable_secret("e", "t", false, &Unavailable)
        .expect_err("refused");
    assert!(
        matches!(&err, EnvError::SecretStore { environment, variable, .. } if environment == "e" && variable == "t"),
        "{err:?}"
    );
    assert_eq!(read_env(&dir, "e"), body);
    assert!(c.environment("e").unwrap().variables()[0].secret);
}

#[test]
fn a_failed_store_write_refuses_the_change_and_keeps_the_value_in_the_file() {
    let body = "name: e\nvariables:\n  - name: t\n    value: v\n";
    let dir = collection_with(&[("e", body)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .set_variable_secret("e", "t", true, &Unavailable)
        .expect_err("refused");
    assert!(
        matches!(&err, EnvError::SecretStore { environment, variable, .. } if environment == "e" && variable == "t"),
        "{err:?}"
    );
    assert_eq!(read_env(&dir, "e"), body);
    assert!(!c.environment("e").unwrap().variables()[0].secret);
}

#[test]
fn unchanged_or_unknown_variables_touch_nothing() {
    let dir = collection_with(&[("e", "name: e\nvariables:\n  - name: t\n    value: v\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    assert_eq!(
        c.set_variable_secret("e", "t", false, &Unavailable)
            .expect("no change"),
        []
    );
    let err = c
        .set_variable_secret("e", "nope", true, &MemoryStore::new())
        .expect_err("refused");
    assert!(matches!(err, EnvError::VariableNotFound { .. }), "{err:?}");
    let err = c
        .set_variable_secret("x", "t", true, &MemoryStore::new())
        .expect_err("refused");
    assert!(matches!(err, EnvError::NotFound { .. }), "{err:?}");
    assert_eq!(
        read_env(&dir, "e"),
        "name: e\nvariables:\n  - name: t\n    value: v\n"
    );
}
