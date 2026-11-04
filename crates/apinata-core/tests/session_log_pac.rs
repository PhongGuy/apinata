//! A PAC-routed send logs each hop's PAC line, redacted like the hop's other text.

use crate::network_support;
use crate::session_log_support;

use apinata_core::{ContextOptions, ExecOptions, GlobalProxy, NetworkSettings, PacSource};
use network_support::resolve_in;
use session_log_support::{assert_absent, logged, only};
use tempfile::TempDir;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const PLAIN: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const SECRET: &str = "pac-sekrit-4471";

fn pac(script: &str) -> (TempDir, ExecOptions) {
    let dir = TempDir::new().expect("tempdir");
    let file = dir.path().join("proxy.pac");
    std::fs::write(&file, script).expect("pac");
    let source = PacSource::parse(file.to_str().expect("utf-8 path"), None).expect("source");
    let opts =
        ExecOptions::new().network(NetworkSettings::new().proxy(GlobalProxy::Pac { source }));
    (dir, opts)
}

/// A 302 from `/start` to `/ping`, then 200.
async fn redirecting() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/ping"))
        .mount(&server)
        .await;
    Mock::given(path("/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn each_logged_hop_keeps_its_pac_route() {
    let server = redirecting().await;
    let (_dir, opts) = pac("function FindProxyForURL(url, host) { return 'DIRECT'; }");
    let (exec, log) = logged(opts);
    let r = resolve_in(
        PLAIN,
        &format!("{}/start", server.uri()),
        ContextOptions::new(),
    );
    exec.send(r.request()).await.expect("sends");
    let e = only(&log);
    let routes: Vec<Option<&str>> = e.hops.iter().map(|h| h.pac.as_deref()).collect();
    assert_eq!(routes, [Some("DIRECT"), Some("DIRECT")]);
}

#[tokio::test]
async fn a_pac_fallback_line_is_logged_without_the_secret_it_echoes() {
    let server = redirecting().await;
    let (_dir, opts) =
        pac("function FindProxyForURL(url, host) { throw new Error('boom ' + url); }");
    let (exec, log) = logged(opts);
    let r = resolve_in(
        PLAIN,
        &format!("{}/start?k={{{{tok}}}}", server.uri()),
        ContextOptions::new().sensitive_env_var("tok", SECRET),
    );
    exec.send(r.request()).await.expect("sends");
    let e = only(&log);
    let first = e.hops[0].pac.as_deref().expect("a PAC line");
    assert!(
        first.starts_with("script failed: ") && first.contains("boom"),
        "{first}"
    );
    // The script echoed the URL, so the line held the secret before redaction.
    assert!(first.contains("k=[redacted]"), "{first}");
    assert_absent(&log, SECRET);
}
