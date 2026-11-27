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
            hint: Some(UndefinedHint::SecretWithoutValue {
                environment: "prod".to_owned(),
            }),
        })
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
