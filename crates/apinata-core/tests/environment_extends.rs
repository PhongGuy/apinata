//! Setting and clearing which environment another extends, through the validating setter that
//! the CLI and MCP share: a cycle, a missing parent and extending itself are refused.

use apinata_core::{Collection, EnvError, GlobalEnvironments};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// A collection with one environment file per `(name, body)`.
fn collection(envs: &[(&str, &str)]) -> TempDir {
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

fn env_file(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join("environments").join(format!("{name}.yml"))).expect("read")
}

const BASE: &str = "name: base\nvariables:\n  - name: host\n    value: h\n";
const DEV: &str = "name: dev\nvariables:\n  - name: own\n    value: o\n";

#[test]
fn setting_a_parent_writes_what_the_app_writes() {
    let ours = collection(&[("base", BASE), ("dev", DEV)]);
    let app = collection(&[("base", BASE), ("dev", DEV)]);
    let mut c = Collection::open(ours.path()).expect("opens");
    c.set_environment_extends("dev", Some("base"))
        .expect("sets");
    let mut a = Collection::open(app.path()).expect("opens");
    let env = a.environment_mut("dev").expect("dev");
    env.set_extends(Some("base".into()));
    env.save().expect("saves");
    assert_eq!(env_file(ours.path(), "dev"), env_file(app.path(), "dev"));
    assert!(env_file(ours.path(), "dev").contains("extends: base"));
}

#[test]
fn clearing_removes_the_key() {
    let dir = collection(&[("base", BASE), ("dev", "name: dev\nextends: base\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    c.set_environment_extends("dev", None).expect("clears");
    assert_eq!(env_file(dir.path(), "dev"), "name: dev\n");
}

#[test]
fn clearing_works_even_when_the_parent_is_missing() {
    let dir = collection(&[("dev", "name: dev\nextends: gone\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    c.set_environment_extends("dev", None).expect("clears");
    assert_eq!(env_file(dir.path(), "dev"), "name: dev\n");
}

#[test]
fn extending_itself_is_refused() {
    let dir = collection(&[("dev", DEV)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .set_environment_extends("dev", Some("dev"))
        .expect_err("refused");
    assert!(matches!(err, EnvError::ExtendsItself { .. }), "{err:?}");
    assert_eq!(err.to_string(), "environment `dev` cannot extend itself");
    assert_eq!(env_file(dir.path(), "dev"), DEV);
}

#[test]
fn a_missing_parent_is_refused_in_the_run_time_wording() {
    let dir = collection(&[("dev", DEV)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .set_environment_extends("dev", Some("gone"))
        .expect_err("refused");
    assert!(matches!(err, EnvError::ExtendsMissing { .. }), "{err:?}");
    assert_eq!(
        err.to_string(),
        "environment `dev` extends `gone`, which does not exist"
    );
    assert_eq!(env_file(dir.path(), "dev"), DEV);
}

#[test]
fn a_cycle_is_refused_in_the_run_time_wording() {
    let a = "name: a\nextends: b\n";
    let b = "name: b\nextends: c\n";
    let dir = collection(&[("a", a), ("b", b), ("c", "name: c\n")]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .set_environment_extends("c", Some("a"))
        .expect_err("refused");
    assert!(matches!(err, EnvError::ExtendsCycle { .. }), "{err:?}");
    assert_eq!(
        err.to_string(),
        "environment inheritance cycle: c -> a -> b -> c"
    );
    assert_eq!(env_file(dir.path(), "c"), "name: c\n");
}

#[test]
fn an_unknown_environment_is_not_found() {
    let dir = collection(&[("base", BASE)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    let err = c
        .set_environment_extends("nope", Some("base"))
        .expect_err("refused");
    assert!(matches!(err, EnvError::NotFound { .. }), "{err:?}");
}

#[test]
fn the_parent_name_is_trimmed() {
    let dir = collection(&[("base", BASE), ("dev", DEV)]);
    let mut c = Collection::open(dir.path()).expect("opens");
    c.set_environment_extends("dev", Some(" base "))
        .expect("sets");
    assert!(env_file(dir.path(), "dev").contains("extends: base\n"));
}

#[test]
fn global_environments_refuse_a_cycle_too() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("a.yml"), "name: a\nextends: b\n").expect("a");
    fs::write(dir.path().join("b.yml"), "name: b\n").expect("b");
    let mut g = GlobalEnvironments::open(dir.path()).expect("opens");
    let err = g
        .set_environment_extends("b", Some("a"))
        .expect_err("refused");
    assert_eq!(
        err.to_string(),
        "environment inheritance cycle: b -> a -> b"
    );
    g.set_environment_extends("b", None).expect("clears");
    g.set_environment_extends("a", None).expect("clears");
    g.set_environment_extends("b", Some("a")).expect("sets");
    let b = fs::read_to_string(dir.path().join("b.yml")).expect("read");
    assert_eq!(b, "name: b\nextends: a\n");
}
