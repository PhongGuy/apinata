//! The environment a shell uses when the caller names none: the presets' default environment,
//! while the collection has it.

use apinata_core::{Collection, EnvironmentChoice};
use std::fs;
use tempfile::TempDir;

const ENV: &str = "name: x\nvariables: []\n";

fn open(preset: Option<&str>, envs: &[&str]) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    let mut root = String::from("opencollection: 1.0.0\ninfo:\n  name: t\n");
    if let Some(name) = preset {
        root += &format!("extensions:\n  bruno:\n    presets:\n      defaultEnvironment: {name}\n");
    }
    fs::write(dir.path().join("opencollection.yml"), root).expect("root");
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    for env in envs {
        fs::write(dir.path().join(format!("environments/{env}.yml")), ENV).expect("env");
    }
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

#[test]
fn the_preset_is_used_when_no_environment_is_named() {
    let (_d, c) = open(Some("dev"), &["dev", "prod"]);
    let choice = c.choose_environment(None);
    assert_eq!(choice, EnvironmentChoice::Preset("dev".into()));
    assert_eq!(choice.name(), Some("dev"));
    assert_eq!(choice.warning(), None);
}

#[test]
fn a_named_environment_wins_over_the_preset() {
    let (_d, c) = open(Some("dev"), &["dev", "prod"]);
    let choice = c.choose_environment(Some("prod"));
    assert_eq!(choice.name(), Some("prod"));
    assert_eq!(choice.warning(), None);
}

#[test]
fn a_named_environment_is_passed_on_even_when_missing() {
    let (_d, c) = open(Some("dev"), &["dev"]);
    assert_eq!(c.choose_environment(Some("nope")).name(), Some("nope"));
}

#[test]
fn a_preset_naming_a_missing_environment_uses_none_and_warns() {
    let (_d, c) = open(Some("gone"), &["dev"]);
    let choice = c.choose_environment(None);
    assert_eq!(choice.name(), None);
    assert_eq!(
        choice.warning().as_deref(),
        Some("default environment \"gone\" not found; using no environment")
    );
}

#[test]
fn without_a_preset_no_environment_is_used() {
    let (_d, c) = open(None, &["dev"]);
    let choice = c.choose_environment(None);
    assert_eq!(choice, EnvironmentChoice::None);
    assert_eq!(choice.warning(), None);
}
