//! `export::openapi` servers: literal and templated origins, defaults, environments, secrets.

use crate::export_support;

use apinata_core::export::openapi;
use export_support::*;
use serde_json::json;

const VARS: &str = "request:\n  variables:\n    - name: baseUrl\n      value: https://default.test\n    - name: port\n      value: \"8080\"\n    - name: tenant\n      value: leak-me\n      secret: true\n    - name: nested\n      value: \"{{baseUrl}}/v1\"\n";

fn env(name: &str, body: &str) -> (String, String) {
    (
        format!("environments/{name}.yml"),
        format!("name: {name}\n{body}"),
    )
}

fn get(name: &str, seq: u32, url: &str) -> (String, String) {
    (
        format!("{name}.yml"),
        http_at(name, seq, "GET", url, INHERIT),
    )
}

#[test]
fn a_literal_origin_is_one_server_and_the_first_origin_is_top_level() {
    let files = [
        get("a", 1, "https://x.test/a"),
        get("b", 2, "https://x.test/b"),
        get("c", 3, "http://o.test:81/c"),
    ];
    let (_d, c) = open_with_root(ROOT, &files);
    let v = openapi_doc(&c);
    assert_eq!(keys(&v), ["openapi", "info", "servers", "paths"]);
    assert_eq!(v["servers"], json!([{"url": "https://x.test"}]));
    assert_eq!(
        keys(&v["paths"]["/b"]["get"]),
        ["summary", "operationId", "responses"]
    );
    assert_eq!(
        keys(&v["paths"]["/c"]["get"]),
        ["summary", "operationId", "servers", "responses"]
    );
    assert_eq!(
        v["paths"]["/c"]["get"]["servers"],
        json!([{"url": "http://o.test:81"}])
    );
}

#[test]
fn a_variable_origin_is_a_templated_server_then_one_per_resolving_environment() {
    let files = [
        env(
            "b-prod",
            "variables:\n  - name: baseUrl\n    value: https://prod.test\n",
        ),
        env(
            "a-dev",
            "variables:\n  - name: baseUrl\n    value: https://dev.test\n  - name: port\n    value: 9000\n",
        ),
        env(
            "c-secret",
            "variables:\n  - name: baseUrl\n    secret: true\n",
        ),
        env(
            "d-nested",
            "variables:\n  - name: baseUrl\n    value: \"{{host}}/api\"\n",
        ),
        env("e-missing", "variables:\n  - name: other\n    value: x\n"),
        env(
            "f-off",
            "variables:\n  - name: baseUrl\n    value: https://off.test\n    disabled: true\n",
        ),
        get("a", 1, "{{baseUrl}}/a"),
        get("b", 2, "http://localhost:{{port}}/b"),
        get("c", 3, "{{baseUrl}}/c"),
    ];
    let (_d, c) = open_with_root(&format!("{ROOT}{VARS}"), &files);
    let v = openapi_doc(&c);
    assert_eq!(
        v["servers"],
        json!([
        {"url": "{baseUrl}", "variables": {"baseUrl": {"default": "https://default.test"}}},
        {"url": "https://dev.test", "description": "a-dev"},
        {"url": "https://prod.test", "description": "b-prod"}])
    );
    assert_eq!(
        v["paths"]["/b"]["get"]["servers"],
        json!([
        {"url": "http://localhost:{port}", "variables": {"port": {"default": "8080"}}},
        {"url": "http://localhost:9000", "description": "a-dev"}])
    );
    assert!(v["paths"]["/c"]["get"].get("servers").is_none());
}

#[test]
fn secret_templated_or_missing_defaults_are_empty_and_secrets_never_leak() {
    let files = [
        env(
            "base",
            "variables:\n  - name: tenant\n    value: env-leak\n    secret: true\n",
        ),
        // An own plain row cannot hide an inherited secret, so `child` gives no server.
        env(
            "child",
            "extends: base\nvariables:\n  - name: tenant\n    value: child-plain\n",
        ),
        get("a", 1, "https://{{tenant}}.x.test/a"),
        get("b", 2, "{{nested}}/b"),
        get("c", 3, "{{unknown}}/c"),
    ];
    let (_d, c) = open_with_root(&format!("{ROOT}{VARS}"), &files);
    let v = openapi_doc(&c);
    assert_eq!(
        v["servers"],
        json!([{"url": "https://{tenant}.x.test", "variables": {"tenant": {"default": ""}}}])
    );
    assert_eq!(
        v["paths"]["/b"]["get"]["servers"],
        json!([{"url": "{nested}", "variables": {"nested": {"default": ""}}}])
    );
    assert_eq!(
        v["paths"]["/c"]["get"]["servers"],
        json!([{"url": "{unknown}", "variables": {"unknown": {"default": ""}}}])
    );
    let json = openapi(&c).json;
    for secret in ["leak-me", "env-leak", "child-plain"] {
        assert!(!json.contains(secret), "{secret} leaked");
    }
}

#[test]
fn user_names_and_passwords_are_cut_from_server_urls_with_a_note() {
    let files = [
        env(
            "prod",
            "variables:\n  - name: baseUrl\n    value: https://env-user:env-pass@prod.test/api\n",
        ),
        get("a", 1, "https://u-secret:p-secret@api.test/x"),
        get("b", 2, "https://api.test/y"),
        get("c", 3, "http://only-user@o.test:81/z"),
        get("d", 4, "{{baseUrl}}/w"),
    ];
    let root = format!(
        "{ROOT}request:\n  variables:\n    - name: baseUrl\n      value: https://def-user:def-pass@def.test\n"
    );
    let (_d, c) = open_with_root(&root, &files);
    let v = openapi_doc(&c);
    assert_eq!(v["servers"], json!([{"url": "https://api.test"}]));
    assert!(v["paths"]["/y"]["get"].get("servers").is_none());
    assert_eq!(
        v["paths"]["/z"]["get"]["servers"],
        json!([{"url": "http://o.test:81"}])
    );
    assert_eq!(
        v["paths"]["/w"]["get"]["servers"],
        json!([
            {"url": "{baseUrl}", "variables": {"baseUrl": {"default": "https://def.test"}}},
            {"url": "https://prod.test/api", "description": "prod"}])
    );
    let cut = "the user name and password in the URL were left out";
    assert_eq!(
        openapi_notes(&c),
        pairs(&[("a / url", cut), ("c / url", cut)])
    );
    let json = openapi(&c).json;
    for secret in [
        "u-secret",
        "p-secret",
        "only-user",
        "env-user",
        "env-pass",
        "def-user",
        "def-pass",
    ] {
        assert!(!json.contains(secret), "{secret} leaked");
    }
}
