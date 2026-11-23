//! What an environment inherits from its `extends` chain, marked against its own rows, so the
//! editor can show inherited variables and which own rows override them.

use apinata_core::{Collection, EnvVariable, InheritedState, InheritedVariable, VarValue};
use std::fs;
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

fn find<'a>(vars: &'a [InheritedVariable], name: &str) -> &'a InheritedVariable {
    vars.iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("{name} inherited: {vars:?}"))
}

const ROOT: &str = "name: root\nvariables:\n  - name: deep\n    value: d\n  - name: host\n    value: root-host\n  - name: off\n    value: x\n    disabled: true\n  - name: token\n    secret: true\n";
const MID: &str = "name: mid\nextends: root\nvariables:\n  - name: host\n    value: mid-host\n";
const LEAF: &str = "name: leaf\nextends: mid\nvariables:\n  - name: own\n    value: o\n";

fn chain() -> (TempDir, Collection) {
    let dir = collection(&[("root", ROOT), ("mid", MID), ("leaf", LEAF)]);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

#[test]
fn a_two_level_chain_names_the_nearest_ancestor() {
    let (_dir, c) = chain();
    let own = c.environment("leaf").expect("leaf").variables();
    let got = c.inherited_variables("leaf", &own).expect("leaf exists");
    assert_eq!(got.warning, None);
    let host = find(&got.variables, "host");
    assert_eq!(host.from, "mid");
    assert_eq!(host.value.as_deref(), Some("mid-host"));
    assert_eq!(host.state, InheritedState::Inherited);
    assert_eq!(find(&got.variables, "deep").from, "root");
    assert!(got.variables.iter().all(|v| v.name != "own"));
}

#[test]
fn disabled_parent_rows_are_left_out() {
    let (_dir, c) = chain();
    let got = c.inherited_variables("leaf", &[]).expect("leaf");
    assert!(got.variables.iter().all(|v| v.name != "off"), "{got:?}");
}

#[test]
fn an_own_plain_row_over_an_inherited_secret_loses() {
    let (_dir, c) = chain();
    let own = [EnvVariable::new("token", VarValue::Text("mine".into()))];
    let got = c.inherited_variables("leaf", &own).expect("leaf");
    let token = find(&got.variables, "token");
    assert!(token.secret);
    assert_eq!(token.state, InheritedState::WinsOverOwn);
}

#[test]
fn own_rows_mark_what_they_override_and_follow_unsaved_edits() {
    let (_dir, c) = chain();
    let mut host = EnvVariable::new("host", VarValue::Text("leaf-host".into()));
    let own = [host.clone(), EnvVariable::secret("token")];
    let got = c.inherited_variables("leaf", &own).expect("leaf");
    assert_eq!(
        find(&got.variables, "host").state,
        InheritedState::OverriddenByOwn
    );
    assert_eq!(
        find(&got.variables, "token").state,
        InheritedState::OverriddenByOwn
    );
    // A disabled own row overrides nothing.
    host.enabled = false;
    let got = c.inherited_variables("leaf", &[host]).expect("leaf");
    assert_eq!(
        find(&got.variables, "host").state,
        InheritedState::Inherited
    );
}

#[test]
fn a_secret_is_inherited_without_a_value() {
    let (_dir, c) = chain();
    let got = c.inherited_variables("leaf", &[]).expect("leaf");
    let token = find(&got.variables, "token");
    assert!(token.secret);
    assert_eq!(token.value, None);
    assert_eq!(token.from, "root");
}

#[test]
fn a_cycle_keeps_what_was_found_and_warns() {
    let dir = collection(&[
        (
            "a",
            "name: a\nextends: b\nvariables:\n  - name: from_a\n    value: a\n",
        ),
        (
            "b",
            "name: b\nextends: a\nvariables:\n  - name: from_b\n    value: b\n",
        ),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let got = c.inherited_variables("a", &[]).expect("a");
    assert_eq!(find(&got.variables, "from_b").from, "b");
    assert!(got.variables.iter().all(|v| v.name != "from_a"));
    let warning = got.warning.expect("warns");
    assert!(warning.contains("a -> b -> a"), "{warning}");
}

#[test]
fn a_missing_parent_keeps_what_was_found_and_warns() {
    let dir = collection(&[
        (
            "root",
            "name: root\nextends: gone\nvariables:\n  - name: r\n    value: r\n",
        ),
        ("leaf", "name: leaf\nextends: root\nvariables: []\n"),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let got = c.inherited_variables("leaf", &[]).expect("leaf");
    assert_eq!(find(&got.variables, "r").from, "root");
    assert!(got.warning.expect("warns").contains("gone"));
    assert!(c.inherited_variables("nope", &[]).is_none());
}
