//! `ResolvedRequest::sensitive_values`: what resolution substituted from secrets, sensitive
//! overrides, `.env` and the process environment.

use crate::common;

use apinata_core::{Collection, ContextOptions, MemoryStore, SecretStore, VarContext};
use common::{at, write_tree};
use std::sync::Arc;
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// `dev` holds secrets `token`, `templated` and `unused`, and plain `auth` and `prefix`.
const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: templated\n    secret: true\n  - name: unused\n    secret: true\n  - name: auth\n    value: \"Bearer {{token}}\"\n  - name: prefix\n    value: abc\n";

/// A request whose URL, header and body are the given templates.
fn request(url: &str, header: &str, body: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{url}\"\n  headers:\n    - name: X-A\n      value: \"{header}\"\n  body:\n    type: text\n    data: \"{body}\"\n"
    )
}

/// The collection with `r.yml` and any extra files, plus a store holding `dev`'s secrets.
fn open(req: &str, extra: &[(&str, &str)]) -> (TempDir, Collection, Arc<MemoryStore>) {
    let mut files = vec![
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", req),
    ];
    files.extend_from_slice(extra);
    let dir = write_tree(&files);
    let c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    for (var, value) in [
        ("token", "s3cr3t-Token"),
        ("templated", "{{prefix}}-xyz"),
        ("unused", "never-used"),
    ] {
        store.set(&c.secret_key("dev", var), value).unwrap();
    }
    (dir, c, Arc::new(store))
}

/// The sorted sensitive values of `r.yml` under `options`.
fn sensitive(c: &Collection, options: ContextOptions) -> Vec<String> {
    let ctx = VarContext::new(c, options).expect("context");
    let mut values = at(c, &ctx, "r.yml")
        .expect("resolves")
        .sensitive_values()
        .to_vec();
    values.sort();
    values
}

fn dev(store: &Arc<MemoryStore>) -> ContextOptions {
    ContextOptions::new().env("dev").secrets(store.clone())
}

#[test]
fn a_secret_counts_once_wherever_it_is_substituted_and_unused_ones_do_not() {
    let req = request(
        "https://x.test/{{token}}?q={{token}}",
        "{{token}}",
        "{{token}}",
    );
    let (_d, c, store) = open(&req, &[]);
    assert_eq!(sensitive(&c, dev(&store)), ["s3cr3t-Token"]);
}

#[test]
fn nested_references_count_the_secret_value_not_the_plain_wrapper() {
    let req = request("https://x.test/", "{{auth}}", "{{templated}}");
    let (_d, c, store) = open(&req, &[]);
    // `auth` is plain and wraps `token`; `templated` is a secret whose value references `prefix`.
    assert_eq!(sensitive(&c, dev(&store)), ["abc-xyz", "s3cr3t-Token"]);
}

#[test]
fn basic_auth_credentials_from_secrets_count() {
    let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n  auth:\n    type: basic\n    username: me\n    password: \"{{token}}\"\n";
    let (_d, c, store) = open(req, &[]);
    assert_eq!(sensitive(&c, dev(&store)), ["s3cr3t-Token"]);
}

#[test]
fn dot_env_and_process_env_values_count_whatever_their_path() {
    let req = request(
        "https://x.test/{{process.env.DOT}}",
        "{{process.env.PROC}}",
        "{{process.env['a.b']}}",
    );
    let (_d, c, _) = open(&req, &[(".env", "DOT=from-dotenv\na.b=dotted\n")]);
    let options = ContextOptions::new().process_env([("PROC", "from-process")]);
    assert_eq!(
        sensitive(&c, options),
        ["dotted", "from-dotenv", "from-process"]
    );
}

#[test]
fn sensitive_overrides_count_and_plain_overrides_do_not() {
    let req = request("https://x.test/{{key}}", "{{plain}}", "{{token}}");
    let (_d, c, store) = open(&req, &[]);
    let options = dev(&store)
        .sensitive_env_var("key", "human-key")
        .env_var("plain", "agent-value")
        .env_var("token", "agent-token");
    // A plain override replaces the secret, so its value is no longer sensitive.
    assert_eq!(sensitive(&c, options), ["human-key"]);

    let later_wins = dev(&store)
        .env_var("key", "first")
        .sensitive_env_var("key", "second")
        .env_var("plain", "p")
        .sensitive_env_var("token", "human-token");
    assert_eq!(sensitive(&c, later_wins), ["human-token", "second"]);

    let plain_last = dev(&store)
        .sensitive_env_var("key", "first")
        .env_var("key", "second")
        .env_var("plain", "p");
    assert_eq!(sensitive(&c, plain_last), ["s3cr3t-Token"]);
}

#[test]
fn a_folder_or_request_variable_shadowing_a_secret_is_not_sensitive() {
    let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test/{{token}}\"\nruntime:\n  variables:\n    - name: token\n      value: request-own\n";
    let (_d, c, store) = open(req, &[]);
    assert!(sensitive(&c, dev(&store)).is_empty());

    let nested = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test/{{token}}\"\n";
    let folder = "info:\n  name: f\n  type: folder\nrequest:\n  variables:\n    - name: token\n      value: folder-own\n";
    let (_d, c, store) = open(nested, &[("f/folder.yml", folder), ("f/r.yml", nested)]);
    let ctx = VarContext::new(&c, dev(&store)).expect("context");
    assert!(
        at(&c, &ctx, "f/r.yml")
            .unwrap()
            .sensitive_values()
            .is_empty()
    );
}

#[test]
fn plain_variables_and_empty_values_are_never_sensitive() {
    let req = request("https://x.test/{{prefix}}", "{{process.env.EMPTY}}", "x");
    let (_d, c, store) = open(&req, &[]);
    let options = dev(&store).process_env([("EMPTY", "")]);
    assert!(sensitive(&c, options).is_empty());
}

#[test]
fn debug_output_counts_sensitive_values_without_showing_them() {
    let req = request("https://x.test/", "{{token}}", "x");
    let (_d, c, store) = open(&req, &[]);
    let ctx = VarContext::new(&c, dev(&store)).unwrap();
    let debug = format!("{:?}", at(&c, &ctx, "r.yml").unwrap());
    assert!(!debug.contains("s3cr3t"), "{debug}");
    assert!(debug.contains("sensitive_values: 1"), "{debug}");
}

/// The Display and Debug text of the error resolving `r.yml` as written in `req`.
fn invalid_url_error(req: &str) -> String {
    let (_d, c, store) = open(req, &[]);
    let ctx = VarContext::new(&c, dev(&store)).unwrap();
    let err = at(&c, &ctx, "r.yml").expect_err("the URL does not parse");
    assert!(
        matches!(err, apinata_core::ResolveError::InvalidUrl { .. }),
        "{err:?}"
    );
    format!("{err} {err:?}")
}

#[test]
fn a_secret_in_the_port_of_an_invalid_url_stays_out_of_the_error() {
    let text = invalid_url_error(&request("https://h:{{token}}/", "x", "x"));
    assert!(!text.contains("s3cr3t"), "{text}");
    assert!(text.contains("{{token}}"), "{text}");
}

#[test]
fn a_secret_in_an_unparsable_host_stays_out_of_the_error() {
    let text = invalid_url_error(&request("https://{{token}}:x/", "x", "x"));
    assert!(!text.contains("s3cr3t"), "{text}");
}

#[test]
fn a_secret_in_the_path_of_an_invalid_url_stays_out_of_the_error() {
    let text = invalid_url_error(&request("https://h:bad/{{token}}", "x", "x"));
    assert!(!text.contains("s3cr3t"), "{text}");
}

#[test]
fn a_secret_nested_in_a_plain_variable_stays_out_of_the_error() {
    // `auth` is plain and wraps `token`; the space makes the host invalid.
    let text = invalid_url_error(&request("https://{{auth}}/", "x", "x"));
    assert!(!text.contains("s3cr3t"), "{text}");
}

#[test]
fn a_secret_path_param_of_an_invalid_url_stays_out_of_the_error() {
    let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://h:bad/:id\"\n  params:\n    - name: id\n      value: \"{{token}}\"\n      type: path\n";
    let text = invalid_url_error(req);
    assert!(!text.contains("s3cr3t"), "{text}");
}

#[test]
fn an_invalid_url_without_secrets_shows_its_interpolated_text() {
    let text = invalid_url_error(&request("https://h:{{prefix}}/", "x", "x"));
    assert!(text.contains("https://h:abc/"), "{text}");
}
