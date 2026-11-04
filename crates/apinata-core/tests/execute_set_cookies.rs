//! What each response hop set: every Set-Cookie value with the jar's verdict on it.

use crate::common;
use crate::network_support::resolve_in;

use apinata_core::{ContextOptions, CookieOutcome, CookieRejection, ExecOptions, Executor};
use common::{executor, resolved_one};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mount(server: &MockServer, at: &str, template: ResponseTemplate) {
    Mock::given(path(at))
        .respond_with(template)
        .mount(server)
        .await;
}

/// `/start` on 127.0.0.1 sets `a=1` and redirects to `localhost`, whose `/next` sets `b=2`.
async fn two_hosts() -> MockServer {
    let server = MockServer::start().await;
    let port = server.address().port();
    mount(
        &server,
        "/start",
        ResponseTemplate::new(302)
            .insert_header("set-cookie", "a=1; Path=/")
            .insert_header("location", format!("http://localhost:{port}/next").as_str()),
    )
    .await;
    mount(
        &server,
        "/next",
        ResponseTemplate::new(200).insert_header("set-cookie", "b=2; Path=/"),
    )
    .await;
    server
}

/// (hop URL, name, value, outcome) for each Set-Cookie of `res`, in order.
fn rows(res: &apinata_core::Response) -> Vec<(String, String, String, CookieOutcome)> {
    res.set_cookies()
        .map(|(origin, c)| {
            let hop = origin.hop().expect("every row here comes from a hop");
            (hop.url.clone(), c.name.clone(), c.value.clone(), c.outcome)
        })
        .collect()
}

#[tokio::test]
async fn a_redirect_chain_lists_each_hops_cookies_with_its_url() {
    let server = two_hosts().await;
    let port = server.address().port();
    let exec = executor();
    let req = resolved_one("GET", &format!("{}/start", server.uri()), "", "");
    let res = exec.send(&req).await.expect("sent");
    assert_eq!(
        rows(&res),
        [
            (
                format!("http://127.0.0.1:{port}/start"),
                "a".to_owned(),
                "1".to_owned(),
                CookieOutcome::Stored
            ),
            (
                format!("http://localhost:{port}/next"),
                "b".to_owned(),
                "2".to_owned(),
                CookieOutcome::Stored
            ),
        ]
    );
    assert_eq!(res.timeline()[1].cookies.len(), 1);
}

#[tokio::test]
async fn a_foreign_domain_is_rejected_with_its_reason() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/rej",
        ResponseTemplate::new(200)
            .append_header("set-cookie", "d=4; Domain=other.example; Path=/")
            .append_header("set-cookie", "x=1; Path=/; Max-Age=0"),
    )
    .await;
    let exec = executor();
    let req = resolved_one("GET", &format!("{}/rej", server.uri()), "", "");
    let res = exec.send(&req).await.expect("sent");
    let got: Vec<_> = res.set_cookies().map(|(_, c)| c.clone()).collect();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, "d");
    assert_eq!(got[0].domain.as_deref(), Some("other.example"));
    assert_eq!(got[0].path.as_deref(), Some("/"));
    assert_eq!(
        got[0].outcome,
        CookieOutcome::Rejected(CookieRejection::DomainMismatch)
    );
    assert_eq!(
        CookieRejection::DomainMismatch.to_string(),
        "Domain does not match host"
    );
    assert_eq!(
        got[1].outcome,
        CookieOutcome::Rejected(CookieRejection::Expired)
    );
    assert!(exec.cookies().list().is_empty());
}

const OVERRIDE: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nextensions:\n  apinata:\n    hostOverrides:\n      - host: \"app.test\"\n        address: \"127.0.0.1\"\n";

#[tokio::test]
async fn a_secure_cookie_over_plain_http_to_a_named_host_is_rejected() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/sec",
        ResponseTemplate::new(200).insert_header("set-cookie", "s=5; Secure; Path=/"),
    )
    .await;
    let port = server.address().port();
    let named = resolve_in(
        OVERRIDE,
        &format!("http://app.test:{port}/sec"),
        ContextOptions::new(),
    )
    .result
    .expect("resolves");
    let exec = executor();
    let res = exec.send(&named).await.expect("sent");
    let got: Vec<_> = res.set_cookies().map(|(_, c)| c.clone()).collect();
    assert!(got[0].secure);
    assert_eq!(
        got[0].outcome,
        CookieOutcome::Rejected(CookieRejection::InsecureSecure)
    );
    assert_eq!(
        CookieRejection::InsecureSecure.to_string(),
        "Secure cookie over http"
    );
    assert!(exec.cookies().list().is_empty());

    // Loopback counts as a secure origin, as it does when the jar sends.
    let local = resolved_one("GET", &format!("{}/sec", server.uri()), "", "");
    let res = exec.send(&local).await.expect("sent");
    assert_eq!(rows(&res)[0].3, CookieOutcome::Stored);
    assert_eq!(exec.cookies().list().len(), 1);
}

#[tokio::test]
async fn an_expired_cookie_that_deletes_a_stored_one_reads_removed() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/set",
        ResponseTemplate::new(200).insert_header("set-cookie", "r=1; Path=/"),
    )
    .await;
    mount(
        &server,
        "/del",
        ResponseTemplate::new(200).insert_header("set-cookie", "r=; Path=/; Max-Age=0"),
    )
    .await;
    let exec = executor();
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    let del = resolved_one("GET", &format!("{}/del", server.uri()), "", "");
    let res = exec.send(&del).await.expect("sent");
    assert_eq!(rows(&res)[0].3, CookieOutcome::Removed);
    assert!(exec.cookies().list().is_empty());
}

#[tokio::test]
async fn with_storing_off_every_cookie_reads_storing_off() {
    let server = two_hosts().await;
    let exec = Executor::new(ExecOptions::new().store_cookies(false)).expect("client");
    let req = resolved_one("GET", &format!("{}/start", server.uri()), "", "");
    let res = exec.send(&req).await.expect("sent");
    let outcomes: Vec<_> = rows(&res).into_iter().map(|r| (r.1, r.3)).collect();
    let off = CookieOutcome::Rejected(CookieRejection::StoringOff);
    assert_eq!(outcomes, [("a".to_owned(), off), ("b".to_owned(), off)]);
    assert_eq!(
        CookieRejection::StoringOff.to_string(),
        "Cookie storing is off"
    );
    assert!(exec.cookies().list().is_empty());
}

#[tokio::test]
async fn no_cookie_value_reaches_debug() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/v",
        ResponseTemplate::new(200).insert_header("set-cookie", "sid=SECRETV; Path=/"),
    )
    .await;
    let exec = executor();
    let req = resolved_one("GET", &format!("{}/v", server.uri()), "", "");
    let res = exec.send(&req).await.expect("sent");
    let shown = format!("{:?} {:?}", res.timeline(), rows(&res)[0].3);
    let cookie: Vec<_> = res.set_cookies().map(|(_, c)| format!("{c:?}")).collect();
    assert!(
        !shown.contains("SECRETV") && !cookie[0].contains("SECRETV"),
        "{cookie:?}"
    );
    assert!(cookie[0].contains("sid"), "{cookie:?}");
}

const SUFFIX_OVERRIDE: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nextensions:\n  apinata:\n    hostOverrides:\n      - host: \"x.co.uk\"\n        address: \"127.0.0.1\"\n";

/// A host under `co.uk`, reached on loopback through a host override, so the Domain matches
/// and only the public-suffix rule can refuse it.
#[tokio::test]
async fn a_public_suffix_domain_is_rejected_with_its_reason() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/psl",
        ResponseTemplate::new(200).insert_header("set-cookie", "p=1; Domain=co.uk; Path=/"),
    )
    .await;
    let port = server.address().port();
    let named = resolve_in(
        SUFFIX_OVERRIDE,
        &format!("http://x.co.uk:{port}/psl"),
        ContextOptions::new(),
    )
    .result
    .expect("resolves");
    let exec = executor();
    let res = exec.send(&named).await.expect("sent");
    let got: Vec<_> = res.set_cookies().map(|(_, c)| c.outcome).collect();
    assert_eq!(
        got,
        [CookieOutcome::Rejected(CookieRejection::PublicSuffix)]
    );
    assert_eq!(
        CookieRejection::PublicSuffix.to_string(),
        "Domain is a public suffix"
    );
    assert!(exec.cookies().list().is_empty());
}
