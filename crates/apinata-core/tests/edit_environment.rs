//! Environment variables, color and `extends`: Bruno's bytes on normalised files, minimal edits
//! elsewhere. Secret entries keep no value in the file.

use crate::config_parity;

use apinata_core::{Collection, EnvVariable, VarValue};
use config_parity::check;
use std::fs;
use tempfile::TempDir;

fn text(value: &str) -> VarValue {
    VarValue::Text(value.into())
}

/// A collection whose `environments/<name>.yml` holds `body`.
fn env_collection(name: &str, body: &str) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: c\n",
    )
    .expect("root");
    fs::create_dir(dir.path().join("environments")).expect("mkdir");
    fs::write(
        dir.path().join("environments").join(format!("{name}.yml")),
        body,
    )
    .expect("env");
    dir
}

#[test]
fn variable_edits_match_bruno() {
    check("env-vars-", |id, c| {
        let env_name = match id {
            "env-vars-edit" | "env-vars-add-secret" => "Local",
            "env-vars-disable-secret" => "staging",
            "env-vars-clear" => "qa",
            other => panic!("unexpected case {other}"),
        };
        let env = c.environment_mut(env_name).expect("environment");
        let mut vars = env.variables();
        match id {
            "env-vars-edit" => vars[0].value = text("http://x.test"),
            "env-vars-add-secret" => vars.push(EnvVariable::secret("token")),
            "env-vars-disable-secret" => vars[0].enabled = false,
            _ => vars.clear(),
        }
        env.set_variables(vars);
        env.save().expect("saves");
    });
}

#[test]
fn color_and_extends_edits_match_bruno() {
    check("env-color", |_, c| {
        let env = c.environment_mut("Local").expect("environment");
        env.set_color(Some("#ff0000".into()));
        env.save().expect("saves");
    });
    check("env-extends-", |id, c| {
        let (name, parent) = match id {
            "env-extends-set" => ("Local", Some("Prod".to_owned())),
            "env-extends-clear" => ("dev", None),
            other => panic!("unexpected case {other}"),
        };
        let env = c.environment_mut(name).expect("environment");
        env.set_extends(parent);
        env.save().expect("saves");
    });
}

#[test]
fn variables_read_plain_typed_and_secret_entries() {
    let body = "name: e\nextends: \" base \"\ncolor: \"#00ff00\"\nvariables:\n  - name: plain\n    value: 1\n  - name: typed\n    value:\n      type: number\n      data: \"2\"\n  - secret: true\n    name: token\n  - secret: true\n    name: blob\n    type: object\n    disabled: true\n";
    let dir = env_collection("e", body);
    let c = Collection::open(dir.path()).expect("opens");
    let env = c.environment("e").expect("environment");
    let mut blob = EnvVariable::secret("blob");
    blob.value = VarValue::Typed {
        kind: "object".into(),
        data: String::new(),
    };
    blob.enabled = false;
    assert_eq!(
        env.variables(),
        vec![
            EnvVariable::new("plain", text("1")),
            EnvVariable::new(
                "typed",
                VarValue::Typed {
                    kind: "number".into(),
                    data: "2".into()
                }
            ),
            EnvVariable::secret("token"),
            blob,
        ]
    );
    assert_eq!(env.extends().as_deref(), Some("base"));
    assert_eq!(env.color().as_deref(), Some("#00ff00"));
    assert!(c.environment("missing").is_none());
}

#[test]
fn a_secret_never_writes_its_value_and_keeps_unknown_keys() {
    let body = "name: e\nvariables:\n  - name: token\n    x-note: keep\n    value: plain-before\n";
    let dir = env_collection("e", body);
    let mut c = Collection::open(dir.path()).expect("opens");
    let env = c.environment_mut("e").expect("environment");
    let mut vars = env.variables();
    vars[0].secret = true;
    vars[0].value = text("must-not-be-written");
    env.set_variables(vars);
    env.save().expect("saves");
    let written = fs::read_to_string(dir.path().join("environments/e.yml")).expect("read");
    assert_eq!(
        written,
        "name: e\nvariables:\n  - secret: true\n    name: token\n    x-note: keep\n"
    );
}

#[test]
fn writing_back_what_was_read_leaves_the_file_clean() {
    let body = "name: e\ncolor: red\nvariables:\n  - name: n\n    value: 1\n    description: why\n  - secret: true\n    name: s\n";
    let dir = env_collection("e", body);
    let mut c = Collection::open(dir.path()).expect("opens");
    let env = c.environment_mut("e").expect("environment");
    let (vars, color, parent) = (env.variables(), env.color(), env.extends());
    env.set_variables(vars);
    env.set_color(color);
    env.set_extends(parent);
    assert!(!env.is_dirty());
}

#[test]
fn an_empty_color_removes_the_key() {
    let dir = env_collection("e", "name: e\ncolor: red\n");
    let mut c = Collection::open(dir.path()).expect("opens");
    let env = c.environment_mut("e").expect("environment");
    env.set_color(Some(String::new()));
    env.save().expect("saves");
    assert_eq!(
        fs::read_to_string(dir.path().join("environments/e.yml")).unwrap(),
        "name: e\n"
    );
}

#[test]
fn debug_hides_variable_values() {
    let mut var = EnvVariable::new("token", text("hunter2"));
    var.enabled = false;
    let shown = format!("{var:?}");
    assert_eq!(
        shown,
        "EnvVariable { name: \"token\", value: <7 bytes>, enabled: false, secret: false }"
    );
}

#[test]
fn extends_is_trimmed_and_an_empty_name_removes_the_key() {
    let dir = env_collection("e", "name: e\n");
    let mut c = Collection::open(dir.path()).expect("opens");
    let env = c.environment_mut("e").expect("environment");
    env.set_extends(Some(" Prod ".into()));
    env.save().expect("saves");
    let path = dir.path().join("environments/e.yml");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "name: e\nextends: Prod\n"
    );

    let env = c.environment_mut("e").expect("environment");
    env.set_extends(Some("  ".into()));
    env.save().expect("saves");
    assert_eq!(fs::read_to_string(&path).unwrap(), "name: e\n");
}
