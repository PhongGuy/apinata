use crate::common;

use apinata_core::{Body, ContextOptions, VarContext, prepare, resolve, resolve_prepared};
use common::{request, setup};
use std::time::Duration;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: token\n      value: t0k\n    - name: host\n      value: x.test\n    - name: blank\n      value: \"\"\n";

fn http(name: &str, url: &str, under_http: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\nhttp:\n  method: post\n  url: \"{url}\"\n{under_http}"
    )
}

fn files() -> Vec<(&'static str, String)> {
    vec![
        ("opencollection.yml", ROOT.to_owned()),
        (
            "a.yml",
            http(
                "a",
                "https://{{host}}/a",
                "  headers:\n    - name: X-A\n      value: \"{{token}}\"\n  auth:\n    type: bearer\n    token: \"{{token}}\"\n  body:\n    type: json\n    data: '{\"t\": \"{{token}}\"}'\n",
            ),
        ),
        (
            "b.yml",
            http(
                "b",
                "https://{{host}}/b",
                "  auth:\n    type: apikey\n    key: k\n    value: \"{{token}}\"\n    placement: query\n",
            ),
        ),
        (
            "c.yml",
            http(
                "c",
                "https://{{host}}/c",
                "  auth:\n    type: basic\n    username: aladdin\n    password: opensesame\n",
            ),
        ),
        (
            "d.yml",
            http(
                "d",
                "https://{{host}}/d",
                "  auth:\n    type: apikey\n    key: X-Key\n    value: \"{{token}}\"\n    placement: header\n",
            ),
        ),
        (
            "e.yml",
            http(
                "e",
                "https://{{host}}/e",
                "  auth:\n    type: apikey\n    key: \"{{blank}}\"\n    value: \"{{missing}}\"\n    placement: header\n",
            ),
        ),
        (
            "f.yml",
            http(
                "f",
                "https://{{host}}/f",
                "  headers:\n    - name: X-Dup\n      value: one\n    - name: x-dup\n      value: two\n",
            ),
        ),
    ]
}

fn open() -> (
    tempfile::TempDir,
    apinata_core::Collection,
    apinata_core::VarContext,
) {
    let owned = files();
    let borrowed: Vec<(&str, &str)> = owned.iter().map(|(p, t)| (*p, t.as_str())).collect();
    setup(&borrowed)
}

fn authorization(r: &apinata_core::ResolvedRequest) -> Vec<(String, String)> {
    r.headers()
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("authorization"))
        .cloned()
        .collect()
}

#[test]
fn resolve_is_prepare_then_resolve_prepared() {
    let (_dir, c, ctx) = open();
    for rel in ["a.yml", "b.yml", "c.yml", "d.yml", "e.yml", "f.yml"] {
        let r = request(&c, rel);
        let split = prepare(r, &ctx).and_then(|p| resolve_prepared(p, &ctx));
        assert_eq!(resolve(r, &ctx), split, "{rel}");
    }
}

#[test]
fn prepare_exposes_templates_before_interpolation() {
    let (_dir, c, ctx) = open();
    let p = prepare(request(&c, "a.yml"), &ctx).expect("prepares");
    assert_eq!(p.method(), "POST");
    assert_eq!(p.url(), "https://{{host}}/a");
    assert_eq!(p.header("authorization"), Some("Bearer {{token}}"));
    assert_eq!(p.header("x-a"), Some("{{token}}"));
    assert_eq!(p.body_text(), Some("{\"t\": \"{{token}}\"}"));
    assert!(p.is_json_body());
    assert_eq!(p.auth_mode(), "bearer");
    let d = prepare(request(&c, "d.yml"), &ctx).expect("prepares");
    assert_eq!(d.header("x-key"), Some("{{token}}"));
}

#[test]
fn a_script_set_header_replaces_the_bearer_template() {
    let (_dir, c, ctx) = open();
    let mut p = prepare(request(&c, "a.yml"), &ctx).expect("prepares");
    p.set_header("AUTHORIZATION", "Custom {{token}}");
    let r = resolve_prepared(p, &ctx).expect("resolves");
    assert_eq!(
        authorization(&r),
        [("Authorization".to_owned(), "Custom t0k".to_owned())]
    );
}

#[test]
fn basic_auth_replaces_a_script_set_authorization_header() {
    let (_dir, c, ctx) = open();
    let mut p = prepare(request(&c, "c.yml"), &ctx).expect("prepares");
    p.set_header("authorization", "Bearer mine");
    let r = resolve_prepared(p, &ctx).expect("resolves");
    let auth = authorization(&r);
    assert_eq!(auth.len(), 1, "{auth:?}");
    assert_eq!(auth[0].1, "Basic YWxhZGRpbjpvcGVuc2VzYW1l");
}

#[test]
fn setters_reach_the_resolved_request() {
    let (_dir, c, ctx) = open();
    let mut p = prepare(request(&c, "a.yml"), &ctx).expect("prepares");
    p.set_method("put");
    p.set_url("https://{{host}}/changed?q=1");
    p.remove_header("x-a");
    assert!(p.set_body_text("{\"n\": \"{{token}}\"}"));
    p.set_timeout(Some(Duration::from_millis(1500)));
    p.set_max_redirects(0);
    let r = resolve_prepared(p, &ctx).expect("resolves");
    assert_eq!(r.method(), "PUT");
    assert_eq!(r.url(), "https://x.test/changed?q=1");
    assert!(
        !r.headers()
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("x-a"))
    );
    assert!(
        matches!(r.body(), Body::Text(t) if t == "{\"n\": \"t0k\"}"),
        "{:?}",
        r.body()
    );
    assert_eq!(r.timeout(), Some(Duration::from_millis(1500)));
    assert_eq!(r.max_redirects(), 0);
}

#[test]
fn the_apikey_query_stays_configuration() {
    let (_dir, c, ctx) = open();
    let p = prepare(request(&c, "b.yml"), &ctx).expect("prepares");
    assert_eq!(p.url(), "https://{{host}}/b");
    let r = resolve_prepared(p, &ctx).expect("resolves");
    assert_eq!(r.url(), "https://x.test/b?k=t0k");
}

#[test]
fn the_bearer_token_stays_sensitive_after_a_header_edit() {
    let (_dir, c, _) = open();
    let options = ContextOptions::new().sensitive_env_var("token", "s3cr3t");
    let ctx = VarContext::new(&c, options).expect("context");
    let mut p = prepare(request(&c, "a.yml"), &ctx).expect("prepares");
    p.set_header("X-Other", "1");
    let r = resolve_prepared(p, &ctx).expect("resolves");
    assert_eq!(
        authorization(&r),
        [("Authorization".to_owned(), "Bearer s3cr3t".to_owned())]
    );
    assert!(r.sensitive_values().contains(&"s3cr3t".to_owned()));
}

#[test]
fn an_apikey_key_expanding_to_empty_drops_the_header_after_an_edit() {
    let (_dir, c, ctx) = open();
    let mut p = prepare(request(&c, "e.yml"), &ctx).expect("prepares");
    assert_eq!(p.header("{{blank}}"), Some("{{missing}}"));
    p.set_header("X-Other", "1");
    // The value names an undefined variable, so resolving succeeds only if it is never read.
    let r = resolve_prepared(p, &ctx).expect("resolves");
    let names: Vec<&str> = r.headers().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["X-Other"]);
}

#[test]
fn set_header_drops_later_case_variants() {
    let (_dir, c, ctx) = open();
    let mut p = prepare(request(&c, "f.yml"), &ctx).expect("prepares");
    assert_eq!(p.header("X-DUP"), Some("two"));
    p.set_header("X-DUP", "three");
    assert_eq!(p.headers(), [("X-Dup".to_owned(), "three".to_owned())]);
    let r = resolve_prepared(p, &ctx).expect("resolves");
    assert_eq!(r.headers(), [("X-Dup".to_owned(), "three".to_owned())]);
}
