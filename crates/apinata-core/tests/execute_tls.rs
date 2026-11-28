//! `--insecure`: certificates are verified by default and accepted when asked; a hop records
//! the connection's TLS details and local address.

use crate::common;

use apinata_core::{ConnectCause, ExecError, ExecOptions, Executor, TlsVersion};
use common::resolved_one;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// A self-signed certificate for `localhost` and `127.0.0.1`, valid until 2126.
const CERT: &str = "
-----BEGIN CERTIFICATE-----
MIIBoTCCAUegAwIBAgIUFIBqvfCcF05xnBExaqLFzAG96MYwCgYIKoZIzj0EAwIw
FzEVMBMGA1UEAwwMYXBpbmF0YS10ZXN0MCAXDTI2MTAwOTA1MDMxM1oYDzIxMjYw
OTE1MDUwMzEzWjAXMRUwEwYDVQQDDAxhcGluYXRhLXRlc3QwWTATBgcqhkjOPQIB
BggqhkjOPQMBBwNCAAT4LqVaRLaBVS+l6zG3D0hJhiOKySg8X6J3r+7NoCxM/7Ua
jl+teuS/Hv8sS4b5DrjNwdg5S4yigSdWk56f/SWBo28wbTAdBgNVHQ4EFgQUsYA1
fOv6AaZgpupbIYQppcsxadYwHwYDVR0jBBgwFoAUsYA1fOv6AaZgpupbIYQppcsx
adYwDwYDVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEw
CgYIKoZIzj0EAwIDSAAwRQIgW78PGr1EL47OJvRV50d7nfIDyL+PTV1QciVzwFeG
pZoCIQCQ5L5QFFNh13q0EVPIK+kzXpBaCRlz989/cAVRwiaWKg==
-----END CERTIFICATE-----
";
/// The certificate's private key; it protects nothing.
const KEY: &str = "
-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg7OQzuCwTpdphOUYm
iWiOvC4cK98jZlaqF0EQgsdGN7ahRANCAAT4LqVaRLaBVS+l6zG3D0hJhiOKySg8
X6J3r+7NoCxM/7Uajl+teuS/Hv8sS4b5DrjNwdg5S4yigSdWk56f/SWB
-----END PRIVATE KEY-----
";

/// Serve `ok` over TLS to every connection on a random port; returns the base URL.
async fn https_server() -> String {
    serve(
        CertificateDer::from_pem_slice(CERT.as_bytes()).expect("cert"),
        PrivateKeyDer::from_pem_slice(KEY.as_bytes()).expect("key"),
    )
    .await
}

/// Like `https_server`, with the certificate and key read from the network fixtures by stem.
async fn https_server_fixture(stem: &str) -> String {
    let dir = format!("{}/tests/fixtures/network", env!("CARGO_MANIFEST_DIR"));
    serve(
        CertificateDer::from_pem_file(format!("{dir}/{stem}.pem")).expect("cert"),
        PrivateKeyDer::from_pem_file(format!("{dir}/{stem}.key")).expect("key"),
    )
    .await
}

async fn serve(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> String {
    serve_reply(cert, key, "", "ok").await
}

/// Like `https_server`, answering every request with `body` as `content_type`; an empty type
/// sends no content-type header.
async fn https_reply(content_type: &'static str, body: &'static str) -> String {
    serve_reply(
        CertificateDer::from_pem_slice(CERT.as_bytes()).expect("cert"),
        PrivateKeyDer::from_pem_slice(KEY.as_bytes()).expect("key"),
        content_type,
        body,
    )
    .await
}

async fn serve_reply(
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    content_type: &'static str,
    body: &'static str,
) -> String {
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("tls config");
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.expect("accept");
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // A failed handshake is the verifying client hanging up; nothing to answer.
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = vec![0; 4096];
                let _ = tls.read(&mut buf).await;
                let typed = if content_type.is_empty() {
                    String::new()
                } else {
                    format!("content-type: {content_type}\r\n")
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\n{typed}content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(reply.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    format!("https://127.0.0.1:{port}/")
}

#[tokio::test]
async fn self_signed_certificates_fail_unless_insecure() {
    let url = https_server().await;
    let req = resolved_one("GET", &url, "", "");
    let strict = Executor::new(ExecOptions::new()).expect("client");
    let err = strict.send(&req).await.expect_err("untrusted certificate");
    assert!(matches!(err, ExecError::Connect { .. }), "{err:?}");
    let insecure = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let res = insecure.send(&req).await.expect("accepted");
    assert_eq!((res.status(), res.text()), (200, "ok"));
}

#[tokio::test]
async fn an_untrusted_certificate_is_a_tls_cause() {
    let url = https_server().await;
    let req = resolved_one("GET", &url, "", "");
    let strict = Executor::new(ExecOptions::new()).expect("client");
    let err = strict.send(&req).await.expect_err("untrusted certificate");
    assert!(
        matches!(
            err,
            ExecError::Connect {
                cause: ConnectCause::TlsUnknownIssuer,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_https_hop_reports_connect_and_tls_as_one_phase() {
    let url = https_server().await;
    let insecure = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let res = insecure
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect("accepted");
    let t = res.timeline()[0].timing.clone().expect("timing");
    assert_eq!(t.connect_kind, apinata_core::ConnectKind::ConnectAndTls);
    assert_eq!(t.dns, None, "{t:?}");
    assert!(t.connect.is_some() && !t.reused && !t.via_proxy, "{t:?}");
}

/// A self-signed certificate that is also a CA (openssl's default) is refused by rustls as a
/// CA used as an end entity; the cause must still be the untrusted-certificate one.
#[tokio::test]
async fn a_self_signed_ca_certificate_is_an_unknown_issuer_unless_insecure() {
    let url = https_server_fixture("selfsigned-ca-leaf").await;
    let req = resolved_one("GET", &url, "", "");
    let strict = Executor::new(ExecOptions::new()).expect("client");
    let err = strict.send(&req).await.expect_err("untrusted certificate");
    assert!(
        matches!(
            err,
            ExecError::Connect {
                cause: ConnectCause::TlsUnknownIssuer,
                ..
            }
        ),
        "{err:?}"
    );
    let insecure = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let res = insecure.send(&req).await.expect("accepted");
    assert_eq!((res.status(), res.text()), (200, "ok"));
}

#[tokio::test]
async fn an_https_hop_carries_the_tls_version_and_the_peer_certificate() {
    let url = https_server().await;
    let insecure = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let res = insecure
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect("accepted");
    let hop = &res.timeline()[0];
    let tls = hop.tls().expect("tls details");
    assert_eq!(tls.version, Some(TlsVersion::Tls1_3), "{tls:?}");
    assert_eq!(
        tls.version.map(|v| v.to_string()).as_deref(),
        Some("TLS 1.3")
    );
    assert_eq!(tls.subject.as_deref(), Some("CN=apinata-test"));
    assert_eq!(tls.issuer.as_deref(), Some("CN=apinata-test"));
    // The fixture's notAfter, 2126-09-15 05:03:13 UTC.
    let expires = UNIX_EPOCH + Duration::from_secs(4_945_122_193);
    assert_eq!(tls.expires, Some(expires));
    assert_eq!(
        tls.expires_label().as_deref(),
        Some("2126-09-15 05:03:13 UTC")
    );
}

#[tokio::test]
async fn a_hop_reports_its_local_address_and_its_header_size_as_http11_text() {
    let url = https_server().await;
    let insecure = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let res = insecure
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect("accepted");
    let hop = &res.timeline()[0];
    let local = hop.local_addr().expect("local address");
    assert_eq!(local.ip().to_string(), "127.0.0.1");
    assert_ne!(Some(local), hop.remote_addr());
    // The server's reply up to and including the blank line ending its header block.
    let head = "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n";
    assert_eq!(hop.response_header_size(), head.len() as u64);
}

#[tokio::test]
async fn a_plain_http_hop_has_no_tls_details_but_a_local_address() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let exec = Executor::new(ExecOptions::new()).expect("client");
    let res = exec
        .send(&resolved_one("GET", &format!("{}/", server.uri()), "", ""))
        .await
        .expect("sent");
    let hop = &res.timeline()[0];
    assert!(hop.tls().is_none(), "{:?}", hop.tls());
    assert!(hop.local_addr().is_some());
}

const SKIP_CHECK: &str = "settings:\n  verifySsl: false\n";
const CHECK: &str = "settings:\n  verifySsl: true\n";

#[tokio::test]
async fn a_request_can_skip_the_check_on_its_own() {
    let url = https_server().await;
    let strict = Executor::new(ExecOptions::new()).expect("client");
    let res = strict
        .send(&resolved_one("GET", &url, "", SKIP_CHECK))
        .await
        .expect("accepted for this request");
    assert_eq!((res.status(), res.text()), (200, "ok"));
    let tls = res.timeline()[0].tls().expect("tls hop");
    assert!(!tls.verified, "{tls:?}");
    // The next request inherits verification, and the pooled unverified connection is not reused.
    let err = strict
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect_err("inherits verification");
    assert!(matches!(err, ExecError::Connect { .. }), "{err:?}");
}

#[tokio::test]
async fn a_request_can_insist_on_the_check_over_the_default() {
    let url = https_server().await;
    let lax = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let err = lax
        .send(&resolved_one("GET", &url, "", CHECK))
        .await
        .expect_err("this request verifies");
    assert!(matches!(err, ExecError::Connect { .. }), "{err:?}");
    let res = lax
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect("inherits the default");
    assert!(!res.timeline()[0].tls().expect("tls").verified);
}

#[tokio::test]
async fn a_forced_skip_beats_the_request() {
    let url = https_server().await;
    let forced = Executor::new(ExecOptions::new().force_insecure(true)).expect("client");
    let res = forced
        .send(&resolved_one("GET", &url, "", CHECK))
        .await
        .expect("the flag wins");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn under_a_host_policy_a_request_skips_the_check_only_for_loopback() {
    let url = https_server().await;
    let policy = |host: &str| host == "trusted.invalid";
    let guarded = Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("client");
    let res = guarded
        .send(&resolved_one("GET", &url, "", SKIP_CHECK))
        .await
        .expect("loopback may skip");
    assert_eq!(res.status(), 200);
    let err = tokio::time::timeout(
        Duration::from_secs(5),
        guarded.send(&resolved_one(
            "GET",
            "https://trusted.invalid/",
            "",
            SKIP_CHECK,
        )),
    )
    .await
    .expect("refused before any I/O")
    .expect_err("a trusted remote host may not skip");
    assert!(
        matches!(&err, ExecError::InsecureNotAllowed { hop: 0, host, .. } if host == "trusted.invalid"),
        "{err:?}"
    );
    // An explicit flag is the user's own choice, so it still applies.
    let forced = Executor::new(
        ExecOptions::new()
            .host_policy(Arc::new(policy))
            .force_insecure(true),
    )
    .expect("client");
    let err = tokio::time::timeout(
        Duration::from_secs(5),
        forced.send(&resolved_one(
            "GET",
            "https://trusted.invalid/",
            "",
            SKIP_CHECK,
        )),
    )
    .await
    .expect("ends")
    .expect_err("no such host");
    assert!(
        !matches!(err, ExecError::InsecureNotAllowed { .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn under_a_host_policy_every_redirect_hop_is_checked_again() {
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "https://trusted.invalid/"),
        )
        .mount(&server)
        .await;
    let policy = |host: &str| host == "trusted.invalid";
    let guarded = Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("client");
    let err = tokio::time::timeout(
        Duration::from_secs(5),
        guarded.send(&resolved_one("GET", &server.uri(), "", SKIP_CHECK)),
    )
    .await
    .expect("refused before any I/O")
    .expect_err("the redirect target is remote");
    assert!(
        matches!(&err, ExecError::InsecureNotAllowed { hop: 1, .. }),
        "{err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_json_report_marks_a_response_whose_check_was_skipped() {
    use crate::script_support::{ROOT, on, write_tree};
    use apinata_core::{
        CancelHandle, Collection, ContextOptions, ResultsReport, RunEvent, VarContext, run,
    };
    let url = https_server().await;
    let file = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\nsettings:\n  verifySsl: false\n"
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let report = run(
        plan,
        ctx,
        exec,
        on(),
        CancelHandle::new(),
        Box::new(|_: RunEvent| {}),
    )
    .await;
    let results: Vec<_> = report.results.iter().map(|r| (**r).clone()).collect();
    let json = ResultsReport::new(vec![], &results)
        .with_responses(&results, &apinata_core::ResponseDetail::default())
        .to_json();
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["results"][0]["response"]["status"], 200, "{json}");
    assert_eq!(value["results"][0]["tls_unverified"], true, "{json}");
}

/// A request under a collection root of `root_extra`, its own `top` after the `http` block.
fn resolved_under(root_extra: &str, url: &str, top: &str) -> apinata_core::ResolvedRequest {
    let root = format!("opencollection: \"1.0.0\"\ninfo:\n  name: t\n{root_extra}");
    let text =
        format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{top}");
    let (_dir, c, ctx) = common::setup(&[("opencollection.yml", &root), ("r.yml", &text)]);
    common::at(&c, &ctx, "r.yml").expect("resolves")
}

/// An executor whose host policy trusts exactly `trusted`.
fn trusting(trusted: &'static [&'static str]) -> Executor {
    let policy = move |host: &str| trusted.contains(&host);
    Executor::new(ExecOptions::new().host_policy(Arc::new(policy))).expect("client")
}

async fn refused_fast(exec: &Executor, req: &apinata_core::ResolvedRequest) -> ExecError {
    tokio::time::timeout(Duration::from_secs(5), exec.send(req))
        .await
        .expect("refused before any I/O")
        .expect_err("refused")
}

#[tokio::test]
async fn under_a_host_policy_localhost_mapped_elsewhere_may_not_skip_the_check() {
    let overrides = "extensions:\n  apinata:\n    hostOverrides:\n      - host: localhost\n        address: 192.0.2.1\n";
    let req = resolved_under(overrides, "https://localhost/", SKIP_CHECK);
    let err = refused_fast(&trusting(&["192.0.2.1"]), &req).await;
    assert!(
        matches!(&err, ExecError::InsecureNotAllowed { hop: 0, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn under_a_host_policy_a_proxied_hop_may_not_skip_the_check() {
    let proxy = "config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: 192.0.2.1\n      port: \"9\"\n      auth:\n        username: \"\"\n        password: \"\"\n      bypassProxy: \"\"\n";
    let req = resolved_under(proxy, "https://localhost/", SKIP_CHECK);
    let err = refused_fast(&trusting(&["192.0.2.1"]), &req).await;
    assert!(
        matches!(&err, ExecError::InsecureNotAllowed { hop: 0, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn under_a_host_policy_localhost_and_ipv6_loopback_may_skip_the_check() {
    let url = https_server().await;
    let port = url.trim_end_matches('/').rsplit(':').next().expect("port");
    let exec = trusting(&[]);
    // The server listens on 127.0.0.1, so localhost may reach it or fail to connect.
    for url in [
        format!("https://localhost:{port}/"),
        format!("https://[::1]:{port}/"),
    ] {
        let sent = tokio::time::timeout(
            Duration::from_secs(5),
            exec.send(&resolved_one("GET", &url, "", SKIP_CHECK)),
        )
        .await
        .expect("ends");
        if let Err(err) = sent {
            assert!(
                !matches!(err, ExecError::InsecureNotAllowed { .. }),
                "{url}: {err:?}"
            );
        }
    }
}

#[tokio::test]
async fn under_a_host_policy_a_redirect_to_a_mapped_localhost_is_checked_again() {
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(302).insert_header("location", "https://localhost/"))
        .mount(&server)
        .await;
    let overrides = "extensions:\n  apinata:\n    hostOverrides:\n      - host: localhost\n        address: 192.0.2.1\n";
    let req = resolved_under(overrides, &server.uri(), SKIP_CHECK);
    let err = refused_fast(&trusting(&["192.0.2.1"]), &req).await;
    assert!(
        matches!(&err, ExecError::InsecureNotAllowed { hop: 1, .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn the_default_json_report_marks_a_skipped_check() {
    use crate::script_support::{ROOT, on, write_tree};
    use apinata_core::{
        CancelHandle, Collection, ContextOptions, ResultsReport, RunEvent, VarContext, run,
    };
    let url = https_server().await;
    let file = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\nsettings:\n  verifySsl: false\n"
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let report = run(
        plan,
        ctx,
        exec,
        on(),
        CancelHandle::new(),
        Box::new(|_: RunEvent| {}),
    )
    .await;
    let results: Vec<_> = report.results.iter().map(|r| (**r).clone()).collect();
    let json = ResultsReport::new(vec![], &results).to_json();
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["results"][0]["tls_unverified"], true, "{json}");
    assert!(value["results"][0].get("response").is_none(), "{json}");
}

/// A token endpoint over TLS with the self-signed certificate; its base URL has no trailing slash.
async fn token_server() -> String {
    let url = https_reply(
        "application/json",
        r#"{"access_token":"tok-tls","token_type":"Bearer","expires_in":3600}"#,
    )
    .await;
    url.trim_end_matches('/').to_owned()
}

/// One request using client-credentials OAuth 2.0 against `provider`, with `top` after it.
fn oauth_fixture(provider: &str, top: &str) -> crate::oauth2_support::Fixture {
    use crate::oauth2_support::{Fixture, ROOT, client_credentials, request};
    let text = request(
        "a",
        1,
        "https://api.test/r",
        &client_credentials(provider, ""),
    ) + top;
    Fixture::new(&[("opencollection.yml", ROOT), ("a.yml", &text)])
}

async fn get_token(
    exec: &Executor,
    f: &crate::oauth2_support::Fixture,
    target: apinata_core::OAuth2Target<'_>,
) -> Result<apinata_core::TokenStatus, apinata_core::OAuth2Error> {
    use apinata_core::{AuthorizeOptions, CancelHandle};
    let (how, cancel) = (AuthorizeOptions::new(), CancelHandle::new());
    let fetch = exec.fetch_token(target, &f.ctx, &how, &cancel);
    tokio::time::timeout(Duration::from_secs(10), fetch)
        .await
        .expect("ends")
}

fn connect_failed(result: Result<apinata_core::TokenStatus, apinata_core::OAuth2Error>) -> bool {
    matches!(
        result,
        Err(apinata_core::OAuth2Error::Exec(ExecError::Connect { .. }))
    )
}

#[tokio::test]
async fn the_token_fetch_follows_the_requests_own_check() {
    use apinata_core::OAuth2Target;
    let provider = token_server().await;
    let strict = Executor::new(ExecOptions::new()).expect("client");
    let plain = oauth_fixture(&provider, "");
    let result = get_token(
        &strict,
        &plain,
        OAuth2Target::Request(plain.request("a.yml")),
    )
    .await;
    assert!(connect_failed(result), "the default verifies");
    let off = oauth_fixture(&provider, SKIP_CHECK);
    get_token(&strict, &off, OAuth2Target::Request(off.request("a.yml")))
        .await
        .expect("this request skips the check, so its token fetch does too");
    let lax = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    let on = oauth_fixture(&provider, CHECK);
    let result = get_token(&lax, &on, OAuth2Target::Request(on.request("a.yml"))).await;
    assert!(connect_failed(result), "this request insists on the check");
}

#[tokio::test]
async fn a_collection_token_fetch_follows_the_default() {
    use crate::oauth2_support::{Fixture, ROOT, client_credentials};
    use apinata_core::OAuth2Target;
    let provider = token_server().await;
    let root = format!(
        "{ROOT}request:\n  auth:\n{}",
        client_credentials(&provider, "")
    );
    let f = Fixture::new(&[("opencollection.yml", root.as_str())]);
    let strict = Executor::new(ExecOptions::new()).expect("client");
    assert!(connect_failed(
        get_token(&strict, &f, OAuth2Target::Collection).await
    ));
    let lax = Executor::new(ExecOptions::new().insecure(true)).expect("client");
    get_token(&lax, &f, OAuth2Target::Collection)
        .await
        .expect("the default skips the check");
}

#[tokio::test]
async fn under_a_host_policy_a_token_fetch_skips_the_check_only_for_loopback() {
    use apinata_core::{OAuth2Error, OAuth2Target};
    let provider = token_server().await;
    let exec = trusting(&["auth.invalid"]);
    let local = oauth_fixture(&provider, SKIP_CHECK);
    get_token(&exec, &local, OAuth2Target::Request(local.request("a.yml")))
        .await
        .expect("loopback may skip");
    let remote = oauth_fixture("https://auth.invalid", SKIP_CHECK);
    let result = get_token(
        &exec,
        &remote,
        OAuth2Target::Request(remote.request("a.yml")),
    )
    .await;
    assert!(
        matches!(
            &result,
            Err(OAuth2Error::Exec(ExecError::InsecureNotAllowed { hop: 0, host, .. })) if host == "auth.invalid"
        ),
        "{result:?}"
    );
}
