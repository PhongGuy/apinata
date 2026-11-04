//! Global environments: loading a profile directory and the create, rename, delete and secret
//! operations shared with collection environments.

use apinata_core::{Collection, GlobalEnvironments, MemoryStore, SecretKey, SecretStore};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// A temp profile whose `global-environments/` holds `envs` (stem, body).
fn globals_with(envs: &[(&str, &str)]) -> (TempDir, PathBuf, GlobalEnvironments) {
    let profile = TempDir::new().expect("tempdir");
    let dir = profile.path().join("global-environments");
    fs::create_dir(&dir).expect("mkdir");
    for (name, body) in envs {
        fs::write(dir.join(format!("{name}.yml")), body).expect("env");
    }
    let globals = GlobalEnvironments::open(&dir).expect("opens");
    (profile, dir, globals)
}

#[test]
fn a_missing_directory_is_an_empty_set_and_is_not_created() {
    let profile = TempDir::new().expect("tempdir");
    let dir = profile.path().join("global-environments");
    let globals = GlobalEnvironments::open(&dir).expect("opens");
    assert!(globals.environments().is_empty());
    assert!(globals.load_errors().is_empty());
    assert_eq!(globals.dir(), dir.as_path());
    assert!(!dir.exists());
}

#[test]
fn yml_files_load_sorted_and_broken_ones_are_load_errors() {
    let (_p, dir, globals) = globals_with(&[
        ("b", "name: b\nvariables:\n  - name: x\n    value: \"1\"\n"),
        ("a", "name: a\n"),
        ("bad", "name: [unclosed\n"),
    ]);
    fs::write(dir.join("notes.txt"), "not an environment").expect("txt");
    let names: Vec<&str> = globals.environments().iter().map(|e| e.name()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(globals.load_errors().len(), 1);
    assert!(globals.load_errors()[0].path().ends_with("bad.yml"));
    assert_eq!(
        globals.environment("b").expect("b").variables()[0].name,
        "x"
    );
}

#[test]
fn secret_keys_use_the_canonical_directory() {
    let (_p, dir, globals) = globals_with(&[("e", "name: e\n")]);
    let canonical = fs::canonicalize(&dir).expect("canonical");
    assert_eq!(
        globals.secret_key("e", "v"),
        SecretKey::new(canonical, "e", "v")
    );
}

#[test]
fn create_makes_the_directory_and_writes_what_a_collection_writes() {
    let profile = TempDir::new().expect("tempdir");
    let dir = profile.path().join("global-environments");
    let mut globals = GlobalEnvironments::open(&dir).expect("opens");
    let store = MemoryStore::new();
    let (name, warnings) = globals.create_environment("qa", &store).expect("creates");
    assert_eq!(name, "qa");
    assert!(warnings.is_empty());
    let (again, _) = globals.create_environment("qa", &store).expect("creates");
    assert_eq!(again, "qa copy");

    let col = TempDir::new().expect("tempdir");
    fs::write(
        col.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .expect("root");
    let mut c = Collection::open(col.path()).expect("opens");
    c.create_environment("qa", &store).expect("creates");
    assert_eq!(
        fs::read(dir.join("qa.yml")).expect("global"),
        fs::read(col.path().join("environments/qa.yml")).expect("collection"),
    );
}

#[test]
fn rename_moves_secrets_and_repoints_extends_and_delete_removes_them() {
    let (_p, dir, mut globals) = globals_with(&[
        (
            "base",
            "name: base\nvariables:\n  - secret: true\n    name: tok\n",
        ),
        ("dev", "name: dev\nextends: base\n"),
    ]);
    let store = MemoryStore::new();
    store
        .set(&globals.secret_key("base", "tok"), "s3cret")
        .expect("set");
    let warnings = globals
        .rename_environment("base", "root", &store)
        .expect("renames");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!dir.join("base.yml").exists());
    assert!(dir.join("root.yml").exists());
    assert_eq!(
        globals
            .environment("dev")
            .expect("dev")
            .extends()
            .as_deref(),
        Some("root")
    );
    assert_eq!(
        store
            .get(&globals.secret_key("root", "tok"))
            .expect("get")
            .as_deref(),
        Some("s3cret")
    );
    assert_eq!(
        store.get(&globals.secret_key("base", "tok")).expect("get"),
        None
    );

    // A parent goes only once nothing extends it.
    globals
        .delete_environment("dev", &store)
        .expect("deletes the child");
    globals.delete_environment("root", &store).expect("deletes");
    assert!(!dir.join("root.yml").exists());
    assert!(globals.environment("root").is_none());
    assert_eq!(
        store.get(&globals.secret_key("root", "tok")).expect("get"),
        None
    );
}

#[test]
fn set_variable_secret_moves_the_value_into_the_store() {
    let (_p, dir, mut globals) =
        globals_with(&[("e", "name: e\nvariables:\n  - name: tok\n    value: abc\n")]);
    let store = MemoryStore::new();
    globals
        .set_variable_secret("e", "tok", true, &store)
        .expect("flags");
    assert_eq!(
        store
            .get(&globals.secret_key("e", "tok"))
            .expect("get")
            .as_deref(),
        Some("abc")
    );
    let text = fs::read_to_string(dir.join("e.yml")).expect("read");
    assert!(
        text.contains("secret: true") && !text.contains("abc"),
        "{text}"
    );
}

#[test]
fn delete_refuses_a_global_environment_others_extend() {
    let (_p, dir, mut globals) = globals_with(&[
        ("base", "name: base\n"),
        ("dev", "name: dev\nextends: base\n"),
        ("qa", "name: qa\nextends: base\n"),
    ]);
    assert_eq!(globals.extended_by("base"), ["dev", "qa"]);
    let err = globals
        .delete_environment("base", &MemoryStore::new())
        .expect_err("refused");
    assert!(
        matches!(&err, apinata_core::EnvError::ExtendedBy { name, by, global: true }
            if name == "base" && by == &["dev", "qa"]),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("global environment `base` is extended by `dev`, `qa`")
    );
    assert!(dir.join("base.yml").exists());
    assert!(globals.environment("base").is_some());
}
