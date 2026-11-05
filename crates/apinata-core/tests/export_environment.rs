//! `export::postman_environment`, its global and every-environment forms, and `export::file_stem`.

use crate::export_support;

use apinata_core::export::{
    EnvironmentFile, file_stem, postman_environment, postman_environment_files,
    postman_global_environment, postman_global_environment_files,
};
use apinata_core::import::parse_postman_environment;
use apinata_core::{Collection, GlobalEnvironments};
use export_support::*;
use serde_json::{Value, json};
use std::path::Path;
use uuid::Uuid;

fn fixture(rel: &str) -> Collection {
    Collection::open(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).expect("opens")
}

fn row(key: &str, value: &str, kind: &str) -> Value {
    json!({"key": key, "value": value, "type": kind, "enabled": true})
}

fn values(c: &Collection, env: &str) -> Value {
    let v: Value = serde_json::from_str(&postman_environment(c, env).expect(env).json).unwrap();
    v["values"].clone()
}

#[test]
fn the_demo_environment_exports_whole() {
    let c = fixture("../../examples/demo");
    let out = postman_environment(&c, "JSONPlaceholder").expect("exists");
    assert!(out.json.ends_with("}\n"));
    assert!(out.notes.is_empty());
    let v: Value = serde_json::from_str(&out.json).unwrap();
    assert_eq!(
        keys(&v),
        ["id", "name", "values", "_postman_variable_scope"]
    );
    let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, b"JSONPlaceholder").to_string();
    assert_eq!(
        v,
        json!({"id": id, "name": "JSONPlaceholder",
        "values": [row("baseUrl", "https://jsonplaceholder.typicode.com", "default")],
        "_postman_variable_scope": "environment"})
    );
    assert!(postman_environment(&c, "nope").is_none());
}

#[test]
fn inheritance_is_flattened_with_secrets_left_empty() {
    let c = fixture("tests/fixtures/bruno/env-inheritance");
    assert_eq!(
        values(&c, "dev"),
        json!([
            row("api_key", "plain_api_key", "default"),
            row("api_url", "https://dev.example.com", "default"),
            row("base_boolean", "true", "default"),
            row("base_number", "42", "default"),
            row("base_object", "{\"region\":\"eu\"}", "default"),
            row("base_only", "base_only_value", "default"),
            row("base_secret_object", "", "secret"),
            row("base_token", "", "secret"),
            row("dev_only", "dev_only_value", "default"),
            row("host", "http://localhost:8081", "default"),
            row("overridden_plain", "", "secret"),
            row("overridden_secret", "", "secret"),
            row("session_id", "base_session", "default"),
            row("shadowed_by_disabled", "from_base", "default"),
            row("typed_override", "overridden_in_dev", "default"),
        ])
    );
    let back = parse_postman_environment(postman_environment(&c, "dev").unwrap().json.as_bytes())
        .expect("imports");
    let secrets: Vec<&str> = back
        .variables
        .iter()
        .filter(|v| v.secret)
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(
        secrets,
        [
            "base_secret_object",
            "base_token",
            "overridden_plain",
            "overridden_secret"
        ]
    );
}

#[test]
fn a_secret_value_in_the_file_never_leaves_and_disabled_rows_are_dropped() {
    let env = "name: prod\nvariables:\n  - name: token\n    value: leaked\n    secret: true\n  - name: idle\n    value: x\n    disabled: true\n";
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("environments/prod.yml", env)]);
    assert_eq!(values(&c, "prod"), json!([row("token", "", "secret")]));
}

#[test]
fn a_broken_chain_becomes_a_note() {
    let lost = "name: lost\nextends: nope\nvariables:\n  - name: own\n    value: o\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("environments/lost.yml", lost),
    ]);
    let out = postman_environment(&c, "lost").expect("lost");
    let notes: Vec<(String, String)> = out
        .notes
        .into_iter()
        .map(|n| (n.source, n.detail))
        .collect();
    assert_eq!(
        notes,
        [(
            "environments / lost".to_owned(),
            "environment `lost` extends `nope`, which does not exist".to_owned()
        )]
    );
    assert_eq!(values(&c, "lost"), json!([row("own", "o", "default")]));
}

#[test]
fn file_stems_lose_separators_and_control_characters() {
    assert_eq!(file_stem("Users/Admin\\x"), "Users_Admin_x");
    assert_eq!(file_stem("a\u{7}b\nc"), "a_b_c");
    assert_eq!(file_stem("  Prod ü  "), "Prod ü");
    assert_eq!(file_stem(" \t"), "untitled");
}

#[test]
fn file_stems_replace_every_windows_invalid_character() {
    assert_eq!(file_stem("a:b*c?d\"e<f>g|h"), "a_b_c_d_e_f_g_h");
}

#[test]
fn file_stems_drop_trailing_dots_and_spaces() {
    assert_eq!(file_stem("prod. . "), "prod");
    assert_eq!(file_stem("..."), "untitled");
}

#[test]
fn file_stems_avoid_windows_device_names() {
    for name in [
        "CON", "nul", "Aux", "PRN", "com1", "LPT9", "CON.v2", "nul.x.y",
    ] {
        let stem = file_stem(name);
        assert_ne!(stem, name, "{name}");
        assert!(stem.starts_with(name.split('.').next().unwrap()), "{stem}");
    }
    assert_eq!(file_stem("console"), "console");
    assert_eq!(file_stem("com10"), "com10");
}

#[test]
fn file_stems_stay_within_a_file_name() {
    let stem = file_stem(&"é".repeat(400));
    assert!(stem.encode_utf16().count() <= 200, "{}", stem.len());
    assert!(!stem.is_empty());
}

#[test]
fn file_stems_handle_awkward_edges() {
    assert_eq!(file_stem("CON .x"), "CON_ .x");
    let capped = file_stem(&format!("{}.", "a".repeat(199)));
    assert_eq!(capped, "a".repeat(199));
    assert!(file_stem(&"\u{1F600}".repeat(150)).encode_utf16().count() <= 200);
}

#[test]
fn file_stems_guard_superscript_and_console_device_names() {
    for name in [
        "COM\u{b9}",
        "COM\u{b2}",
        "COM\u{b3}",
        "LPT\u{b9}",
        "LPT\u{b2}",
        "LPT\u{b3}",
        "CONIN$",
        "CONOUT$",
    ] {
        assert_eq!(file_stem(name), format!("{name}_"), "{name}");
        let lower = name.to_lowercase();
        assert_eq!(file_stem(&lower), format!("{lower}_"), "{lower}");
        assert_eq!(
            file_stem(&format!("{name}.txt")),
            format!("{name}_.txt"),
            "{name}.txt"
        );
        assert_eq!(
            file_stem(&format!("{lower}.txt")),
            format!("{lower}_.txt"),
            "{lower}.txt"
        );
    }
    // Near misses stay as they are.
    assert_eq!(file_stem("COM\u{b9}x"), "COM\u{b9}x");
    assert_eq!(file_stem("CONIN"), "CONIN");
}

fn names(files: &[EnvironmentFile]) -> Vec<&str> {
    files.iter().map(|f| f.file_name.as_str()).collect()
}

#[test]
fn every_environment_gets_a_file_whose_text_is_the_single_export() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: h\n    value: x\n",
        ),
        (
            "environments/prod.yml",
            "name: prod\nvariables:\n  - name: t\n    value: s\n    secret: true\n",
        ),
    ]);
    let files = postman_environment_files(&c);
    assert_eq!(
        names(&files),
        [
            "dev.postman_environment.json",
            "prod.postman_environment.json"
        ]
    );
    for (file, env) in files.iter().zip(["dev", "prod"]) {
        assert_eq!(file.export, postman_environment(&c, env).unwrap());
    }
    assert_eq!(values(&c, "prod"), json!([row("t", "", "secret")]));
}

// Windows forbids `:` and `*` in file names and macOS folds case, so only Linux can hold these.
#[cfg(target_os = "linux")]
#[test]
fn stems_that_collide_are_numbered_ignoring_case() {
    // Names come from file names, which differ here only in case or in a forbidden character.
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("environments/Dev.yml", "name: x\n"),
        ("environments/dev.yml", "name: x\n"),
        ("environments/d:v.yml", "name: x\n"),
        ("environments/d*v.yml", "name: x\n"),
    ]);
    let files = postman_environment_files(&c);
    let mut got = names(&files);
    got.sort();
    assert_eq!(
        got,
        [
            "Dev.postman_environment.json",
            "d_v (2).postman_environment.json",
            "d_v.postman_environment.json",
            "dev (2).postman_environment.json",
        ]
    );
}

#[test]
fn global_environments_export_like_a_collections_with_secrets_left_empty() {
    let dir = tempfile::tempdir().unwrap();
    let base = "name: base\nvariables:\n  - name: h\n    value: x\n  - name: t\n    value: leaked\n    secret: true\n";
    std::fs::write(dir.path().join("base.yml"), base).unwrap();
    std::fs::write(
        dir.path().join("dev.yml"),
        "name: dev\nextends: base\nvariables:\n  - name: own\n    value: o\n",
    )
    .unwrap();
    let globals = GlobalEnvironments::open(dir.path()).unwrap();
    let out = postman_global_environment(&globals, "dev").expect("dev");
    let v: Value = serde_json::from_str(&out.json).unwrap();
    assert_eq!(
        v["values"],
        json!([
            row("h", "x", "default"),
            row("own", "o", "default"),
            row("t", "", "secret")
        ])
    );
    assert_eq!(v["_postman_variable_scope"], "environment");
    assert!(postman_global_environment(&globals, "nope").is_none());

    // A collection holding the same files exports the same bytes.
    let (_c, c) = open(&[
        ("opencollection.yml", ROOT),
        ("environments/base.yml", base),
        (
            "environments/dev.yml",
            "name: dev\nextends: base\nvariables:\n  - name: own\n    value: o\n",
        ),
    ]);
    let files = postman_global_environment_files(&globals);
    assert_eq!(
        names(&files),
        [
            "base.postman_environment.json",
            "dev.postman_environment.json"
        ]
    );
    assert_eq!(files, postman_environment_files(&c));
}

#[test]
fn a_broken_global_chain_is_noted_as_a_global() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lost.yml"), "name: lost\nextends: nope\n").unwrap();
    let globals = GlobalEnvironments::open(dir.path()).unwrap();
    let notes = postman_global_environment(&globals, "lost").unwrap().notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].source, "global environments / lost");
}
