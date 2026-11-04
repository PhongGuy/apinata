//! Spec success criterion 3: every vendored Bruno fixture resolves offline to an enumerated outcome.

use crate::common;

use apinata_core::{
    Body, Collection, ContextOptions, Folder, GlobalEnvironments, Part, RequestFile, ResolveError,
    VarContext, Warning, resolve,
};
use common::request;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bruno")
        .join(name)
}

fn collect<'a>(folder: &'a Folder, out: &mut Vec<&'a RequestFile>) {
    out.extend(folder.requests());
    for sub in folder.folders() {
        collect(sub, out);
    }
}

fn ok(url: &str) -> Result<String, ResolveError> {
    Ok(url.to_owned())
}

fn undefined(name: &str) -> Result<String, ResolveError> {
    Err(ResolveError::UndefinedVariable {
        name: name.to_owned(),
        hint: None,
    })
}

/// Resolve every request and compare the outcomes, keyed by path relative to the root, with `expected`.
fn expect_outcomes(
    c: &Collection,
    ctx: &VarContext,
    expected: &[(&str, Result<String, ResolveError>)],
) {
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    let mut all = Vec::new();
    collect(c.tree(), &mut all);
    let actual: BTreeMap<String, Result<String, ResolveError>> = all
        .into_iter()
        .map(|r| {
            let rel = r
                .path()
                .strip_prefix(c.root())
                .expect("inside root")
                .to_string_lossy()
                .replace('\\', "/");
            (rel, resolve(r, ctx).map(|ok| ok.url().to_owned()))
        })
        .collect();
    let expected: BTreeMap<String, Result<String, ResolveError>> = expected
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    assert_eq!(actual, expected);
}

fn dev_context(c: &Collection) -> VarContext {
    let options = ContextOptions::new()
        .env("dev")
        .env_var("base_token", "token-from-base")
        .env_var("overridden_plain", "secret-from-dev")
        .env_var("overridden_secret", "secret-from-base");
    VarContext::new(c, options).expect("context")
}

#[test]
fn bruno_tests_resolve_under_local() {
    let c = Collection::open(fixture("bruno-tests")).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new().env("Local")).expect("context");
    expect_outcomes(
        &c,
        &ctx,
        &[
            (
                "response-time/delayed-headers.yml",
                ok("http://localhost:8081/api/wait-for/headers?time=1000"),
            ),
            (
                "response-time/delayed-body.yml",
                ok("http://localhost:8081/api/wait-for/body?time=1000"),
            ),
            (
                "variable-datatypes/typed-vars.yml",
                undefined("runtime_num"),
            ),
            (
                "variable-datatypes/form-urlencoded.yml",
                ok("https://echo.usebruno.com/"),
            ),
            (
                "variable-datatypes/multipart.yml",
                ok("https://echo.usebruno.com/"),
            ),
        ],
    );
    let multipart =
        resolve(request(&c, "variable-datatypes/multipart.yml"), &ctx).expect("resolves");
    let expected = [
        ("mp_str", "form_string"),
        ("mp_num", "42"),
        ("mp_bool", "true"),
        ("mp_obj", "{\"key\":\"value\"}"),
    ]
    .map(|(name, value)| Part::text(name, value));
    assert_eq!(multipart.body(), &Body::Multipart(expected.to_vec()));
    for rel in [
        "response-time/delayed-headers.yml",
        "response-time/delayed-body.yml",
    ] {
        let r = resolve(request(&c, rel), &ctx).expect(rel);
        assert_eq!(r.warnings(), [Warning::ScriptsNotRun], "{rel}");
    }
}

#[test]
fn cli_docs_resolve_without_an_environment() {
    let c = Collection::open(fixture("cli-docs")).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    expect_outcomes(
        &c,
        &ctx,
        &[
            ("request_1.yml", ok("https://api.example.com/one")),
            ("request_2.yml", ok("https://api.example.com/two")),
            (
                "folder_1/request_1.yml",
                ok("https://api.example.com/folder-1/one"),
            ),
            (
                "folder_2/request_1.yml",
                ok("https://api.example.com/folder-2/one"),
            ),
        ],
    );
    for rel in [
        "request_1.yml",
        "request_2.yml",
        "folder_1/request_1.yml",
        "folder_2/request_1.yml",
    ] {
        let r = resolve(request(&c, rel), &ctx).expect(rel);
        assert!(r.warnings().is_empty(), "{rel}: {:?}", r.warnings());
    }
    let r = resolve(request(&c, "request_1.yml"), &ctx).expect("resolves");
    assert!(r.warnings().is_empty() && r.headers().is_empty());
    assert_eq!(r.body(), &Body::None);
}

#[test]
fn env_inheritance_resolves_under_dev_with_secrets_supplied() {
    let c = Collection::open(fixture("env-inheritance")).expect("opens");
    let ctx = dev_context(&c);
    expect_outcomes(
        &c,
        &ctx,
        &[
            ("echo.yml", undefined("disabled_in_base")),
            ("workspace-vars.yml", undefined("workspace_host")),
            ("set-env-var.yml", ok("http://localhost:8081/ping")),
            ("set-global-env-var.yml", undefined("workspace_host")),
        ],
    );
    let r = resolve(request(&c, "set-env-var.yml"), &ctx).expect("resolves");
    assert_eq!(r.warnings(), [Warning::ScriptsNotRun]);
}

#[test]
fn env_inheritance_resolves_under_dev_with_the_workspace_dev_globals() {
    let c = Collection::open(fixture("env-inheritance")).expect("opens");
    let globals = GlobalEnvironments::open(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/global-environments"),
    )
    .expect("globals");
    assert!(
        globals.load_errors().is_empty(),
        "{:?}",
        globals.load_errors()
    );
    let options = ContextOptions::new()
        .env("dev")
        .env_var("base_token", "token-from-base")
        .env_var("overridden_plain", "secret-from-dev")
        .env_var("overridden_secret", "secret-from-base")
        .global_env(Arc::new(globals), "workspace_dev")
        .global_env_var("workspace_token", "token-from-workspace-base")
        .global_env_var("workspace_secret_object", r#"{"scope":"admin"}"#)
        .global_env_var("workspace_overridden_secret", "secret-from-workspace-base")
        .global_env_var("workspace_overridden_plain", "secret-from-workspace-dev");
    let ctx = VarContext::new(&c, options).expect("context");
    expect_outcomes(
        &c,
        &ctx,
        &[
            ("echo.yml", undefined("disabled_in_base")),
            // Disabled in `workspace_base`; the reference sends it literally, we refuse.
            ("workspace-vars.yml", undefined("workspace_disabled")),
            ("set-env-var.yml", ok("http://localhost:8081/ping")),
            ("set-global-env-var.yml", ok("http://localhost:8081/ping")),
        ],
    );
}

const TYPED_BODY: &str = r#"{
  "coll_num": 100,
  "coll_bool": false,
  "coll_obj": {"scope":"collection"},
  "fold_num": 200,
  "fold_bool": true,
  "fold_obj": {"scope":"folder"},
  "req_num": 42,
  "req_bool": true,
  "req_obj": {"key":"value"},
  "req_obj_precision": {"integer":123,"negativeInteger":-99,"zero":0,"float":2.718,"negativeFloat":-1.618,"largeDouble":12345.678901234567,"smallDouble":9.876e-12,"booleanTrue":true,"booleanFalse":false},
  "req_obj_empty": {},
  "typed_env_num": 300,
  "typed_env_bool": true,
  "typed_env_obj": {"scope":"env"},
  "runtime_num": 999,
  "runtime_bool": true,
  "runtime_obj": {"nested":"yes"},
  "inferred_env_num": 700,
  "inferred_env_bool": true,
  "inferred_env_obj": {"from":"script"}
}"#;

#[test]
fn typed_vars_render_every_scope_once_script_values_are_supplied() {
    let c = Collection::open(fixture("bruno-tests")).expect("opens");
    let options = ContextOptions::new()
        .env("Local")
        .env_var("runtime_num", "999")
        .env_var("runtime_bool", "true")
        .env_var("runtime_obj", r#"{"nested":"yes"}"#)
        .env_var("inferred_env_num", "700")
        .env_var("inferred_env_bool", "true")
        .env_var("inferred_env_obj", r#"{"from":"script"}"#);
    let ctx = VarContext::new(&c, options).expect("context");
    let r = resolve(request(&c, "variable-datatypes/typed-vars.yml"), &ctx).expect("resolves");
    assert_eq!(r.method(), "POST");
    assert_eq!(r.url(), "https://echo.usebruno.com/");
    assert_eq!(r.body(), &Body::Text(TYPED_BODY.to_owned()));
    let headers: Vec<(&str, &str)> = r
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        headers,
        [
            ("x-h-str", "request_string"),
            ("x-h-num", "42"),
            ("x-h-bool", "true"),
            ("x-h-obj", r#"{"key":"value"}"#),
            ("content-type", "application/json"),
        ]
    );
    assert_eq!(r.warnings(), [Warning::ScriptsNotRun]);
}

#[test]
fn form_urlencoded_fixture_stringifies_typed_values() {
    let c = Collection::open(fixture("bruno-tests")).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new().env("Local")).expect("context");
    let r = resolve(request(&c, "variable-datatypes/form-urlencoded.yml"), &ctx).expect("resolves");
    let expected = [
        ("fu_str", "form_string"),
        ("fu_num", "42"),
        ("fu_bool", "true"),
        ("fu_obj", r#"{"key":"value"}"#),
    ];
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    assert_eq!(r.body(), &Body::FormUrlEncoded(expected));
    let headers: Vec<(&str, &str)> = r
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        headers,
        [("content-type", "application/x-www-form-urlencoded")]
    );
    let slow = resolve(request(&c, "response-time/delayed-headers.yml"), &ctx).expect("resolves");
    assert_eq!(
        (
            slow.timeout(),
            slow.follow_redirects(),
            slow.max_redirects()
        ),
        (None::<Duration>, true, 5)
    );
}
