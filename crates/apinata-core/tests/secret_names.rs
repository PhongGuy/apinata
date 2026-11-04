//! `named_secrets` and `replace_secrets`: the secret values a request used, named for redaction.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, MemoryStore, SecretStore, VarContext, known_secrets, named_secrets,
    replace_secrets,
};
use common::{request as file, write_tree};
use std::sync::Arc;
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// `dev` holds secrets `token`, `templated`, `pin` and `quoted`, and plain `auth` and `prefix`.
const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: templated\n    secret: true\n  - name: pin\n    secret: true\n  - name: quoted\n    secret: true\n  - name: auth\n    value: \"Bearer {{token}}\"\n  - name: prefix\n    value: abc\n";

fn request(header: &str, body: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"https://x.test/\"\n  headers:\n    - name: X-A\n      value: \"{header}\"\n  body:\n    type: text\n    data: \"{body}\"\n"
    )
}

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
        ("pin", "123"),
        ("quoted", "qu\"ote\\d"),
    ] {
        store.set(&c.secret_key("dev", var), value).unwrap();
    }
    (dir, c, Arc::new(store))
}

/// (label, placeholder, value) of each named secret of `r.yml`, in the returned order.
fn named(c: &Collection, options: ContextOptions) -> Vec<(String, String, String)> {
    let ctx = VarContext::new(c, options).expect("context");
    named_secrets(file(c, "r.yml"), &ctx)
        .expect("resolves")
        .iter()
        .map(|s| {
            (
                s.label().to_owned(),
                s.placeholder().to_owned(),
                s.value().to_owned(),
            )
        })
        .collect()
}

fn dev(store: &Arc<MemoryStore>) -> ContextOptions {
    ContextOptions::new().env("dev").secrets(store.clone())
}

fn t(a: &str, b: &str, c: &str) -> (String, String, String) {
    (a.to_owned(), b.to_owned(), c.to_owned())
}

#[test]
fn a_secret_variable_is_named_by_its_placeholder() {
    let (_d, c, store) = open(&request("{{token}}", "x"), &[]);
    assert_eq!(
        named(&c, dev(&store)),
        [t("token", "{{token}}", "s3cr3t-Token")]
    );
}

#[test]
fn a_secret_inside_a_plain_wrapper_keeps_the_secret_name() {
    let (_d, c, store) = open(&request("{{auth}}", "{{templated}}"), &[]);
    // Longest first: "s3cr3t-Token" (12) before "abc-xyz" (7).
    assert_eq!(
        named(&c, dev(&store)),
        [
            t("token", "{{token}}", "s3cr3t-Token"),
            t("templated", "{{templated}}", "abc-xyz"),
        ]
    );
}

#[test]
fn a_dotenv_value_is_named_as_process_env() {
    let (_d, c, _) = open(
        &request("{{process.env.API_KEY}}", "x"),
        &[(".env", "API_KEY=k3y-from-dotenv\n")],
    );
    assert_eq!(
        named(&c, ContextOptions::new()),
        [t(
            "process.env.API_KEY",
            "{{process.env.API_KEY}}",
            "k3y-from-dotenv"
        )]
    );
}

#[test]
fn literal_auth_credentials_count_without_a_name() {
    let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n  auth:\n    type: basic\n    username: me\n    password: hunter22\n";
    let (_d, c, _) = open(req, &[]);
    assert_eq!(
        named(&c, ContextOptions::new()),
        [
            t("auth credentials", "[redacted]", "Basic bWU6aHVudGVyMjI="),
            t("auth credentials", "[redacted]", "hunter22"),
        ]
    );
}

#[test]
fn an_auth_credential_from_a_secret_is_named_and_the_header_is_not() {
    let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n  auth:\n    type: basic\n    username: me\n    password: \"{{token}}\"\n";
    let (_d, c, store) = open(req, &[]);
    assert_eq!(
        named(&c, dev(&store)),
        [
            t("token", "{{token}}", "s3cr3t-Token"),
            t(
                "auth credentials",
                "[redacted]",
                "Basic bWU6czNjcjN0LVRva2Vu"
            ),
        ]
    );
}

#[test]
fn short_values_and_plain_requests_list_nothing() {
    let (_d, c, store) = open(&request("{{pin}}", "plain"), &[]);
    assert!(named(&c, dev(&store)).is_empty());
}

#[test]
fn replace_prefers_names_then_the_rest() {
    let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n  auth:\n    type: basic\n    username: me\n    password: \"{{token}}\"\n";
    let (_d, c, store) = open(req, &[]);
    let ctx = VarContext::new(&c, dev(&store)).expect("context");
    let secrets = named_secrets(file(&c, "r.yml"), &ctx).expect("resolves");
    let echoed = "{\"authorization\":\"Basic bWU6czNjcjN0LVRva2Vu\",\"x\":\"s3cr3t-Token-2\"}";
    assert_eq!(
        replace_secrets(echoed, &secrets),
        "{\"authorization\":\"[redacted]\",\"x\":\"{{token}}-2\"}"
    );
}

#[test]
fn replace_finds_the_json_escaped_form() {
    let (_d, c, store) = open(&request("{{quoted}}", "x"), &[]);
    let ctx = VarContext::new(&c, dev(&store)).expect("context");
    let secrets = named_secrets(file(&c, "r.yml"), &ctx).expect("resolves");
    assert_eq!(
        replace_secrets("{\"h\":\"qu\\\"ote\\\\d\"} raw qu\"ote\\d", &secrets),
        "{\"h\":\"{{quoted}}\"} raw {{quoted}}"
    );
}

#[test]
fn a_bearer_header_and_its_token_are_both_listed() {
    let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n  auth:\n    type: bearer\n    token: tok-literal-1\n";
    let (_d, c, _) = open(req, &[]);
    assert_eq!(
        named(&c, ContextOptions::new()),
        [
            t("auth credentials", "[redacted]", "Bearer tok-literal-1"),
            t("auth credentials", "[redacted]", "tok-literal-1"),
        ]
    );
}

#[test]
fn an_api_key_value_is_listed_in_header_or_query() {
    let header = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/\n  auth:\n    type: apikey\n    key: X-Api-Key\n    value: k-literal-99\n    placement: header\n";
    let (_d, c, _) = open(header, &[]);
    assert_eq!(
        named(&c, ContextOptions::new()),
        [t("auth credentials", "[redacted]", "k-literal-99")]
    );
    let query = header.replace("placement: header", "placement: query");
    let (_d, c, _) = open(&query, &[]);
    assert_eq!(
        named(&c, ContextOptions::new()),
        [t("auth credentials", "[redacted]", "k-literal-99")]
    );
}

#[test]
fn debug_never_shows_a_value() {
    let (_d, c, store) = open(&request("{{token}}", "x"), &[]);
    let ctx = VarContext::new(&c, dev(&store)).expect("context");
    let secrets = named_secrets(file(&c, "r.yml"), &ctx).expect("resolves");
    let shown = format!("{secrets:?}");
    assert!(
        shown.contains("token") && !shown.contains("s3cr3t"),
        "{shown}"
    );
}

#[test]
fn a_literal_proxy_password_is_a_plain_secret_value() {
    let root = format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: \"http\"\n      hostname: \"127.0.0.1\"\n      port: \"3128\"\n      auth:\n        username: \"u\"\n        password: \"pr0xy-pass\"\n      bypassProxy: \"\"\n"
    );
    let (_d, c, _) = open(&request("x", "x"), &[("opencollection.yml", &root)]);
    assert_eq!(
        named(&c, ContextOptions::new()),
        [t("secret value", "[redacted]", "pr0xy-pass")]
    );
}

// Known secrets add every environment secret the request never substituted, named, under the
// same 4-character floor: `token` is used, `templated` and `quoted` are not, `pin` is short.
#[test]
fn known_secrets_add_the_unused_ones_by_name() {
    let (_d, c, store) = open(&request("{{token}}", "plain"), &[]);
    let ctx = VarContext::new(&c, ContextOptions::new().env("dev").secrets(store)).unwrap();
    let mut known: Vec<(String, String)> = known_secrets(file(&c, "r.yml"), &ctx)
        .unwrap()
        .iter()
        .map(|s| (s.placeholder().to_owned(), s.value().to_owned()))
        .collect();
    known.sort();
    assert_eq!(
        known,
        vec![
            ("{{quoted}}".to_owned(), "qu\"ote\\d".to_owned()),
            ("{{templated}}".to_owned(), "abc-xyz".to_owned()),
            ("{{token}}".to_owned(), "s3cr3t-Token".to_owned()),
        ]
    );
}
