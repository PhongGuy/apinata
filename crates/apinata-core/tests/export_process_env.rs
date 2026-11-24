//! `{{process.env.NAME}}` in Postman exports: written as `{{NAME}}` with one note per name,
//! never rewritten inside script code.

use crate::export_support;

use apinata_core::export::{ExportNoteKind, PostmanOptions, postman_environment, postman_with};
use export_support::*;
use serde_json::{Value, json};

const SCRIPT: &str = "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        const k = '{{process.env.API_KEY}}';\n";

fn changed(notes: &[apinata_core::export::ExportNote]) -> Vec<(String, String)> {
    notes
        .iter()
        .filter(|n| n.kind == ExportNoteKind::Changed && n.detail.contains("process.env"))
        .map(|n| (n.source.clone(), n.detail.clone()))
        .collect()
}

fn note(source: &str, name: &str) -> (String, String) {
    (
        source.to_owned(),
        format!(
            "{{{{process.env.{name}}}}} was written as {{{{{name}}}}}, since Postman cannot read the process environment; define {name} in a Postman environment"
        ),
    )
}

#[test]
fn process_env_references_become_plain_variables_with_one_note_per_name() {
    let a = http(
        "a",
        "GET",
        "https://x.test/{{process.env.STAGE}}/a",
        "  auth:\n    type: bearer\n    token: \"{{process.env.API_KEY}}\"\n",
    ) + SCRIPT;
    let b = http(
        "b",
        "GET",
        "https://x.test/b",
        "  headers:\n    - name: X-Key\n      value: \"{{process.env.API_KEY}}\"\n  auth: inherit\n",
    );
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)]);
    let out = postman_with(&c, &PostmanOptions::new().keep_scripts(true));
    let v: Value = serde_json::from_str(&out.json).unwrap();
    let a = &v["item"][0];
    assert_eq!(
        a["request"]["auth"]["bearer"],
        json!([{"key": "token", "value": "{{API_KEY}}", "type": "string"}])
    );
    assert_eq!(
        a["request"]["url"]["raw"],
        json!("https://x.test/{{STAGE}}/a")
    );
    assert_eq!(v["item"][1]["request"]["header"][0]["value"], "{{API_KEY}}");
    // Script code is exported as written.
    assert_eq!(
        a["event"][0]["script"]["exec"],
        json!(["const k = '{{process.env.API_KEY}}';"])
    );
    assert_eq!(
        changed(&out.notes),
        [note("a", "API_KEY"), note("a", "STAGE")]
    );
}

#[test]
fn an_environment_export_rewrites_process_env_values_too() {
    let env = "name: prod\nvariables:\n  - name: token\n    value: \"Bearer {{process.env.TOKEN}}\"\n  - name: pw\n    value: hidden\n    secret: true\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("environments/prod.yml", env)]);
    let out = postman_environment(&c, "prod").expect("exists");
    let v: Value = serde_json::from_str(&out.json).unwrap();
    let value = |key: &str| {
        let rows = v["values"].as_array().unwrap();
        rows.iter().find(|r| r["key"] == key).unwrap()["value"].clone()
    };
    assert_eq!(value("token"), "Bearer {{TOKEN}}");
    assert_eq!(value("pw"), "", "a secret still leaves no value");
    assert!(!out.json.contains("hidden"));
    assert_eq!(changed(&out.notes), [note("environments / prod", "TOKEN")]);
}

#[test]
fn other_templates_are_left_alone() {
    let a = http(
        "a",
        "GET",
        "https://x.test/{{process.env.}}/{{host}}/{{process.env['a b']}}",
        "",
    );
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let out = postman_with(&c, &PostmanOptions::new());
    let v: Value = serde_json::from_str(&out.json).unwrap();
    assert_eq!(
        v["item"][0]["request"]["url"]["raw"],
        "https://x.test/{{process.env.}}/{{host}}/{{process.env['a b']}}"
    );
    assert!(changed(&out.notes).is_empty());
}
