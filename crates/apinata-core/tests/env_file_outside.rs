//! An environment read from a file outside the collection (`run --env-file`): both file kinds,
//! precedence against overrides, and secrets that never come from the collection's store.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, EnvFileError, EnvFileKind, EnvironmentFile, MemoryStore,
    ResolveError, SecretStore, UndefinedHint, VarContext,
};
use common::{at, write_tree};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

const POSTMAN: &str = r#"{
  "id": "5f1c",
  "name": "CI",
  "values": [
    {"key": "base", "value": "https://ci.test", "type": "default", "enabled": true},
    {"key": "path", "value": "from-file", "type": "default", "enabled": true},
    {"key": "off", "value": "never", "type": "default", "enabled": false},
    {"key": "token", "value": "pm-s3cret-value", "type": "secret", "enabled": true}
  ],
  "_postman_variable_scope": "environment"
}"#;

const OPEN_COLLECTION: &str = "name: ignored\nvariables:\n  - name: base\n    value: https://oc.test\n  - name: path\n    value: from-yaml\n";

/// A collection with environment `prod`, whose secret `token` the store holds, and requests
/// that put one variable each in their URL.
fn collection() -> (TempDir, Collection) {
    let request = |name: &str, url: &str| {
        format!("info:\n  name: {name}\n  type: http\nhttp:\n  method: get\n  url: \"{url}\"\n")
    };
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        ),
        (
            "environments/prod.yml",
            "name: prod\nvariables:\n  - name: base\n    value: https://prod.test\n  - secret: true\n    name: token\n",
        ),
        ("path.yml", &request("path", "{{base}}/{{path}}")),
        ("token.yml", &request("token", "https://x.test/{{token}}")),
        ("off.yml", &request("off", "https://x.test/{{off}}")),
        ("name.yml", &request("name", "https://x.test/{{__name__}}")),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    (dir, c)
}

/// Write `body` to `name` in a directory of its own, outside any collection.
fn outside(name: &str, body: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(name);
    std::fs::write(&path, body).expect("write");
    (dir, path)
}

fn load(path: &Path) -> EnvironmentFile {
    EnvironmentFile::load(path).expect("loads")
}

fn url(c: &Collection, ctx: &VarContext, rel: &str) -> Result<String, ResolveError> {
    at(c, ctx, rel).map(|r| r.url().to_owned())
}

#[test]
fn a_postman_export_supplies_its_enabled_values() {
    let (_d, c) = collection();
    let (_f, path) = outside("ci.json", POSTMAN);
    let file = load(&path);
    assert_eq!(file.kind(), EnvFileKind::Postman);
    assert_eq!(file.name(), "CI");
    let ctx = VarContext::new(&c, ContextOptions::new().env_file(file)).expect("context");
    assert_eq!(
        url(&c, &ctx, "path.yml"),
        Ok("https://ci.test/from-file".into())
    );
    assert!(matches!(
        url(&c, &ctx, "off.yml"),
        Err(ResolveError::UndefinedVariable { .. })
    ));
    assert_eq!(ctx.environment(), Some("CI"));
    assert_eq!(url(&c, &ctx, "name.yml"), Ok("https://x.test/CI".into()));
}

#[test]
fn an_open_collection_file_is_named_by_its_stem() {
    let (_d, c) = collection();
    let (_f, path) = outside("ci-env.yml", OPEN_COLLECTION);
    let file = load(&path);
    assert_eq!(file.kind(), EnvFileKind::OpenCollection);
    assert_eq!(file.name(), "ci-env");
    let ctx = VarContext::new(&c, ContextOptions::new().env_file(file)).expect("context");
    assert_eq!(
        url(&c, &ctx, "path.yml"),
        Ok("https://oc.test/from-yaml".into())
    );
}

#[test]
fn a_postman_secret_with_a_value_is_used_and_kept_sensitive() {
    let (_d, c) = collection();
    let (_f, path) = outside("ci.json", POSTMAN);
    let file = load(&path);
    assert_eq!(file.secret_names(), vec!["token".to_owned()]);
    let ctx = VarContext::new(&c, ContextOptions::new().env_file(file)).expect("context");
    let req = at(&c, &ctx, "token.yml").expect("resolves");
    assert_eq!(req.url(), "https://x.test/pm-s3cret-value");
    assert!(
        req.sensitive_values()
            .iter()
            .any(|v| v == "pm-s3cret-value"),
        "{:?}",
        req.sensitive_values().len()
    );
}

#[test]
fn a_file_named_like_a_collection_environment_never_reads_its_stored_secrets() {
    let (_d, c) = collection();
    let store = MemoryStore::new();
    store
        .set(&c.secret_key("prod", "token"), "the-real-prod-token")
        .expect("set");
    let (_f, path) = outside(
        "prod.yml",
        "variables:\n  - name: base\n    value: https://attacker.test\n  - secret: true\n    name: token\n",
    );
    let options = ContextOptions::new()
        .env_file(load(&path))
        .secrets(Arc::new(store));
    let ctx = VarContext::new(&c, options).expect("context");
    assert_eq!(
        url(&c, &ctx, "token.yml"),
        Err(ResolveError::UndefinedVariable {
            name: "token".to_owned(),
            hint: Some(UndefinedHint::SecretWithoutValueInFile {
                environment: "prod".to_owned(),
                kind: EnvFileKind::OpenCollection,
            }),
        })
    );
}

#[test]
fn a_secret_without_a_value_in_a_file_hints_at_the_flag_or_the_file_only() {
    let (_d, c) = collection();
    let (_f, path) = outside(
        "ci.json",
        r#"{"name": "CI", "values": [{"key": "token", "value": "", "type": "secret", "enabled": true}]}"#,
    );
    let ctx = VarContext::new(&c, ContextOptions::new().env_file(load(&path))).expect("context");
    let Err(ResolveError::UndefinedVariable {
        hint: Some(hint), ..
    }) = url(&c, &ctx, "token.yml")
    else {
        panic!("token is undefined with a hint");
    };
    assert_eq!(
        hint,
        UndefinedHint::SecretWithoutValueInFile {
            environment: "CI".to_owned(),
            kind: EnvFileKind::Postman,
        }
    );
    let cli = hint.cli_text("token");
    assert!(
        cli.contains("pass --env-var token=VALUE or give it a value in the file"),
        "{cli}"
    );
    assert!(!cli.contains("editor"), "{cli}");
    assert!(!hint.mcp_text("token").contains("editor"));
}

#[test]
fn an_open_collection_file_secret_hints_at_the_flag_alone() {
    let (_d, c) = collection();
    let (_f, path) = outside("ci.yml", "variables:\n  - secret: true\n    name: token\n");
    let ctx = VarContext::new(&c, ContextOptions::new().env_file(load(&path))).expect("context");
    let Err(ResolveError::UndefinedVariable {
        hint: Some(hint), ..
    }) = url(&c, &ctx, "token.yml")
    else {
        panic!("token is undefined with a hint");
    };
    let cli = hint.cli_text("token");
    assert_eq!(
        cli,
        "token is a secret in environment file ci with no value; pass --env-var token=VALUE"
    );
    assert!(!hint.mcp_text("token").contains("in the file"));
}

#[test]
fn a_byte_order_mark_is_read_past_in_either_kind() {
    let (_d, c) = collection();
    let (_p, pm) = outside("ci.json", &format!("\u{feff}{POSTMAN}"));
    let (_y, oc) = outside("ci.yml", &format!("\u{feff}{OPEN_COLLECTION}"));
    assert_eq!(load(&pm).kind(), EnvFileKind::Postman);
    assert_eq!(load(&oc).kind(), EnvFileKind::OpenCollection);
    let ctx = VarContext::new(&c, ContextOptions::new().env_file(load(&pm))).expect("context");
    assert_eq!(
        url(&c, &ctx, "path.yml"),
        Ok("https://ci.test/from-file".into())
    );
}

#[test]
fn an_override_beats_the_file() {
    let (_d, c) = collection();
    let (_f, path) = outside("ci.json", POSTMAN);
    let options = ContextOptions::new()
        .env_file(load(&path))
        .env_var("path", "from-flag");
    let ctx = VarContext::new(&c, options).expect("context");
    assert_eq!(
        url(&c, &ctx, "path.yml"),
        Ok("https://ci.test/from-flag".into())
    );
}

#[test]
fn the_file_replaces_a_named_environment() {
    let (_d, c) = collection();
    let (_f, path) = outside("ci.json", POSTMAN);
    let options = ContextOptions::new().env("prod").env_file(load(&path));
    let ctx = VarContext::new(&c, options).expect("context");
    assert_eq!(
        url(&c, &ctx, "path.yml"),
        Ok("https://ci.test/from-file".into())
    );
}

#[test]
fn a_missing_file_is_a_read_error_naming_the_path() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("nope.json");
    let err = EnvironmentFile::load(&path).expect_err("fails");
    assert!(matches!(err, EnvFileError::Read { .. }), "{err:?}");
    let text = err.to_string();
    assert!(text.starts_with("could not read "), "{text}");
    assert!(text.contains("nope.json"), "{text}");
}

#[test]
fn text_that_is_neither_json_nor_yaml_is_a_parse_error() {
    let (_f, path) = outside("bad.json", "{ \"name\": [1, 2\n");
    let err = EnvironmentFile::load(&path).expect_err("fails");
    assert!(matches!(err, EnvFileError::Parse { .. }), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("bad.json"), "{text}");
    assert!(text.contains("is not valid JSON or YAML"), "{text}");
}

#[test]
fn a_document_of_another_shape_is_not_an_environment_file() {
    let (_f, path) = outside("other.json", "{\"hello\": 1}");
    let err = EnvironmentFile::load(&path).expect_err("fails");
    assert!(matches!(err, EnvFileError::Unrecognized { .. }), "{err:?}");
    assert!(
        err.to_string().contains(
            "is not an environment file (expected an OpenCollection environment or a Postman environment export)"
        ),
        "{err}"
    );
}

/// The shapes a Hoppscotch environment export takes, with and without its version key.
const HOPPSCOTCH: [&str; 2] = [
    r#"{"v": 2, "id": "h1", "name": "H", "variables": [{"key": "base", "initialValue": "https://h.test", "currentValue": "https://h.test", "secret": false}]}"#,
    r#"{"name": "H", "variables": [{"key": "base", "value": "https://h.test"}]}"#,
];

#[test]
fn a_hoppscotch_export_is_refused_as_unsupported() {
    for body in HOPPSCOTCH {
        let (_f, path) = outside("hopp.json", body);
        let err = EnvironmentFile::load(&path).expect_err("fails");
        assert!(matches!(err, EnvFileError::Unsupported { .. }), "{err:?}");
        let text = err.to_string();
        assert!(text.contains("hopp.json"), "{text}");
        assert!(text.contains("format is not supported"), "{text}");
    }
}

#[test]
fn postman_globals_are_not_an_environment_file() {
    let (_f, path) = outside(
        "globals.json",
        r#"{"name": "G", "values": [{"key": "a", "value": "b"}], "_postman_variable_scope": "globals"}"#,
    );
    let err = EnvironmentFile::load(&path).expect_err("fails");
    assert!(matches!(err, EnvFileError::Unrecognized { .. }), "{err:?}");
}

#[test]
fn a_postman_shape_written_as_yaml_says_it_must_be_json() {
    let (_f, path) = outside(
        "pm.yml",
        "name: CI
values:
  - key: a
    value: b
",
    );
    let err = EnvironmentFile::load(&path).expect_err("fails");
    assert!(matches!(err, EnvFileError::Invalid { .. }), "{err:?}");
    assert!(
        err.to_string()
            .contains("a Postman environment export must be JSON"),
        "{err}"
    );
}

#[test]
fn a_parse_error_never_echoes_a_value() {
    let (_f, path) = outside(
        "cut.json",
        "{\"name\": \"CI\", \"values\": [{\"key\": \"t\", \"value\": \"V4LUE-Zq81\"",
    );
    let err = EnvironmentFile::load(&path).expect_err("fails");
    assert!(!err.to_string().contains("V4LUE-Zq81"), "{err}");
}

#[test]
fn postman_notes_about_the_values_are_kept() {
    let (_f, path) = outside(
        "ci.json",
        r#"{"id": "x", "name": "CI", "values": [{"key": "my key", "value": "v"}], "_postman_exported_at": "2026"}"#,
    );
    let file = load(&path);
    assert_eq!(
        file.notes(),
        ["values[0].key: the variable `my key` was renamed to `my_key`".to_owned()]
    );
}

#[test]
fn extends_is_refused_even_without_variables() {
    let (_f, path) = outside(
        "child.yml",
        "extends: prod
",
    );
    let err = EnvironmentFile::load(&path).expect_err("fails");
    assert!(matches!(err, EnvFileError::Extends { .. }), "{err:?}");
}

#[test]
fn an_open_collection_file_with_extends_is_refused() {
    let (_f, path) = outside(
        "child.yml",
        "extends: prod\nvariables:\n  - name: a\n    value: b\n",
    );
    let err = EnvironmentFile::load(&path).expect_err("fails");
    assert!(matches!(err, EnvFileError::Extends { .. }), "{err:?}");
    assert!(
        err.to_string()
            .contains("uses extends, which an environment file outside the collection cannot"),
        "{err}"
    );
}
