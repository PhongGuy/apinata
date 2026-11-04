//! A hop whose URL carries `user:pass@` sends that Basic credential as its only Authorization.

use crate::common;
use crate::session_log_support;

use apinata_core::{ExecError, ExecOptions, Executor, Response, Warning};
use base64::Engine;
use common::{executor, resolved_one};
use session_log_support::{assert_absent, header, logged, only};
use std::sync::Arc;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const USER: &str = "ui-user";
const PASS: &str = "LOCPW9q8zz";
const BEARER: &str = "  auth:\n    type: bearer\n    token: OWNTOKEN55\n";
const OWN_HEADER: &str = "  headers:\n    - name: Authorization\n      value: Token HDRTOKEN77\n";

fn basic() -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{PASS}"));
    format!("Basic {encoded}")
}

fn redirect(to: &str) -> ResponseTemplate {
    ResponseTemplate::new(302).insert_header("location", to)
}

/// `server`'s URI with the test credential spliced in after the scheme.
fn with_userinfo(server: &MockServer) -> String {
    server
        .uri()
        .replacen("http://", &format!("http://{USER}:{PASS}@"), 1)
}

async fn mount(server: &MockServer, at: &str, template: ResponseTemplate) {
    Mock::given(path(at))
        .respond_with(template)
        .mount(server)
        .await;
}

/// Every Authorization value the `index`th request `server` received carried.
async fn received(server: &MockServer, index: usize) -> Vec<String> {
    let requests = server.received_requests().await.expect("recorded");
    requests[index]
        .headers
        .get_all("authorization")
        .iter()
        .map(|v| v.to_str().expect("ascii").to_owned())
        .collect()
}

/// Every Authorization value hop `index` of the timeline recorded as sent.
fn recorded(res: &Response, index: usize) -> Vec<String> {
    res.timeline()[index]
        .request_headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.clone())
        .collect()
}

#[tokio::test]
async fn a_same_origin_location_with_userinfo_replaces_the_own_authorization() {
    for own in [BEARER, OWN_HEADER] {
        let server = MockServer::start().await;
        mount(
            &server,
            "/start",
            redirect(&format!("{}/next", with_userinfo(&server))),
        )
        .await;
        mount(&server, "/next", ResponseTemplate::new(200)).await;
        let req = resolved_one("GET", &format!("{}/start", server.uri()), own, "");
        let res = executor().send(&req).await.expect("sent");
        assert_eq!(res.status(), 200);
        assert_eq!(received(&server, 1).await, [basic()], "{own}");
        assert_eq!(recorded(&res, 1), [basic()], "{own}");
    }
}

#[tokio::test]
async fn a_cross_origin_location_with_userinfo_sends_its_basic_whatever_forwarding_says() {
    for forward in ["true", "false"] {
        let (a, b) = (MockServer::start().await, MockServer::start().await);
        mount(
            &a,
            "/start",
            redirect(&format!("{}/other", with_userinfo(&b))),
        )
        .await;
        mount(&b, "/other", ResponseTemplate::new(200)).await;
        let settings = format!("settings:\n  forwardAuthorizationHeader: {forward}\n");
        let req = resolved_one("GET", &format!("{}/start", a.uri()), BEARER, &settings);
        let res = executor().send(&req).await.expect("sent");
        assert_eq!(res.status(), 200);
        assert_eq!(received(&b, 0).await, [basic()], "forwarding {forward}");
        assert_eq!(recorded(&res, 1), [basic()], "forwarding {forward}");
    }
}

#[tokio::test]
async fn a_cross_origin_location_without_userinfo_forwards_only_when_asked() {
    for (forward, expected) in [("true", vec!["Bearer OWNTOKEN55"]), ("false", vec![])] {
        let (a, b) = (MockServer::start().await, MockServer::start().await);
        mount(&a, "/start", redirect(&format!("{}/other", b.uri()))).await;
        mount(&b, "/other", ResponseTemplate::new(200)).await;
        let settings = format!("settings:\n  forwardAuthorizationHeader: {forward}\n");
        let req = resolved_one("GET", &format!("{}/start", a.uri()), BEARER, &settings);
        let res = executor().send(&req).await.expect("sent");
        assert_eq!(res.status(), 200);
        assert_eq!(received(&b, 0).await, expected, "forwarding {forward}");
        assert_eq!(recorded(&res, 1), expected, "forwarding {forward}");
    }
}

#[tokio::test]
async fn a_request_url_with_userinfo_sends_only_its_basic() {
    for own in [BEARER, OWN_HEADER] {
        let server = MockServer::start().await;
        mount(&server, "/one", ResponseTemplate::new(200)).await;
        let url = format!("{}/one", with_userinfo(&server));
        let res = executor()
            .send(&resolved_one("GET", &url, own, ""))
            .await
            .expect("sent");
        assert_eq!(res.status(), 200);
        assert_eq!(received(&server, 0).await, [basic()], "{own}");
        assert_eq!(recorded(&res, 0), [basic()], "{own}");
    }
}

#[tokio::test]
async fn a_digest_challenge_on_a_userinfo_hop_is_not_answered() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/d",
        ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            r#"Digest realm="r", nonce="n0nce", qop="auth", algorithm=MD5"#,
        ),
    )
    .await;
    let digest = "  auth:\n    type: digest\n    username: du\n    password: dp\n";
    let url = format!("{}/d", with_userinfo(&server));
    let res = executor()
        .send(&resolved_one("GET", &url, digest, ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 401);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(received(&server, 0).await, [basic()]);
}

#[tokio::test]
async fn a_userinfo_location_to_a_refused_host_fails_without_naming_the_password() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/start",
        redirect("http://u:SECRETPW4411@example.invalid/x"),
    )
    .await;
    let exec =
        Executor::new(ExecOptions::new().host_policy(Arc::new(|_: &str| false))).expect("client");
    let req = resolved_one("GET", &format!("{}/start", server.uri()), "", "");
    let err = exec.send(&req).await.unwrap_err();
    assert!(
        matches!(&err, ExecError::HostNotAllowed { hop: 1, host, .. } if host == "example.invalid"),
        "{err:?}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    for text in [err.to_string(), format!("{err:?}")] {
        assert!(!text.contains("SECRETPW4411"), "{text}");
    }
}

#[tokio::test]
async fn a_userinfo_hop_and_its_echoed_basic_and_password_never_reach_the_log() {
    let server = MockServer::start().await;
    let encoded = basic()["Basic ".len()..].to_owned();
    mount(
        &server,
        "/start",
        redirect(&format!("{}/next", with_userinfo(&server))),
    )
    .await;
    mount(
        &server,
        "/next",
        ResponseTemplate::new(200)
            .insert_header("x-echo", format!("{encoded} {PASS}").as_str())
            .set_body_string(format!("{{\"echo\":\"{encoded}\",\"pass\":\"{PASS}\"}}")),
    )
    .await;
    let (exec, log) = logged(ExecOptions::new());
    let req = resolved_one("GET", &format!("{}/start", server.uri()), BEARER, "");
    assert_eq!(exec.send(&req).await.expect("sent").status(), 200);
    let e = only(&log);
    assert_eq!(
        header(&e.hops[1].request_headers, "authorization"),
        Some("[redacted]")
    );
    for secret in [PASS, encoded.as_str(), "OWNTOKEN55"] {
        assert_absent(&log, secret);
    }
}

#[tokio::test]
async fn userinfo_reqwest_cannot_decode_leaves_the_own_authorization() {
    // `%FF` is not UTF-8 once decoded, so no Basic is built and the hop keeps its own header.
    let server = MockServer::start().await;
    mount(&server, "/one", ResponseTemplate::new(200)).await;
    let url = server.uri().replacen("http://", "http://%FF:p@", 1);
    let res = executor()
        .send(&resolved_one("GET", &format!("{url}/one"), BEARER, ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    assert_eq!(received(&server, 0).await, ["Bearer OWNTOKEN55"]);
    assert_eq!(recorded(&res, 0), ["Bearer OWNTOKEN55"]);
}

/// Auth modes that put their own credential in Authorization, by name.
const SIGNERS: [(&str, &str); 5] = [
    (
        "oauth1",
        "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs\n    accessToken: tk\n    accessTokenSecret: ts\n    signatureMethod: HMAC-SHA1\n    placement: header\n",
    ),
    (
        "sigv4",
        "  auth:\n    type: awsv4\n    accessKeyId: AKIDTEST\n    secretAccessKey: secret-key\n    sessionToken: \"\"\n    service: execute-api\n    region: eu-west-1\n    profileName: \"\"\n",
    ),
    (
        "edgegrid",
        "  auth:\n    type: akamai-edgegrid\n    accessToken: akab-access-9\n    clientToken: akab-client-9\n    clientSecret: eg-secret-6612\n    headersToSign: \"user-agent\"\n    nonce: n0nce\n    timestamp: 20140321T19:34:21+0000\n",
    ),
    (
        "ntlm",
        "  auth:\n    type: ntlm\n    username: nu\n    password: np\n    domain: CORP\n",
    ),
    (
        "apikey",
        "  auth:\n    type: apikey\n    key: Authorization\n    value: KEYVAL991\n    placement: header\n",
    ),
];

#[tokio::test]
async fn every_auth_mode_gives_way_to_the_basic_on_a_userinfo_hop() {
    for (name, auth) in SIGNERS {
        // The request URL itself carries the credential.
        let server = MockServer::start().await;
        mount(&server, "/one", ResponseTemplate::new(200)).await;
        let url = format!("{}/one", with_userinfo(&server));
        let res = executor()
            .send(&resolved_one("GET", &url, auth, ""))
            .await
            .expect(name);
        assert_eq!(res.status(), 200, "{name}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "{name}");
        assert_eq!(received(&server, 0).await, [basic()], "{name}");
        assert_eq!(recorded(&res, 0), [basic()], "{name}");

        // A same-origin redirect hands the credential over.
        let server = MockServer::start().await;
        mount(
            &server,
            "/start",
            redirect(&format!("{}/next", with_userinfo(&server))),
        )
        .await;
        mount(&server, "/next", ResponseTemplate::new(200)).await;
        let res = executor()
            .send(&resolved_one(
                "GET",
                &format!("{}/start", server.uri()),
                auth,
                "",
            ))
            .await
            .expect(name);
        assert_eq!(res.status(), 200, "{name}");
        let hop = res.timeline().len() - 1;
        assert_eq!(recorded(&res, hop), [basic()], "{name}");
        let last = server.received_requests().await.unwrap().len() - 1;
        assert_eq!(received(&server, last).await, [basic()], "{name}");
    }
}

// A username alone can be the whole credential (a token sent as the user), so no record of a
// sent request keeps it, with or without a password, on the first hop or after a redirect.
#[tokio::test]
async fn a_url_username_never_reaches_the_log() {
    for userinfo in ["acct-7781", "acct-7781:pw-55"] {
        let server = MockServer::start().await;
        let spliced = server
            .uri()
            .replacen("http://", &format!("http://{userinfo}@"), 1);
        mount(&server, "/start", redirect(&format!("{spliced}/next"))).await;
        mount(&server, "/next", ResponseTemplate::new(200)).await;
        let (exec, log) = logged(ExecOptions::new());
        let req = resolved_one("GET", &format!("{spliced}/start"), "", "");
        assert_eq!(exec.send(&req).await.expect("sent").status(), 200);
        let e = only(&log);
        assert_eq!(e.hops.len(), 2, "{userinfo}");
        for secret in ["acct-7781", "pw-55"] {
            assert_absent(&log, secret);
        }
    }
}

const NTLM: &str =
    "  auth:\n    type: ntlm\n    username: nu\n    password: np\n    domain: CORP\n";

fn ntlm_challenge() -> ResponseTemplate {
    ResponseTemplate::new(401).insert_header("www-authenticate", "NTLM")
}

// The URL's Basic is a userinfo hop's only Authorization, so no NTLM Type 1 is sent and the
// server's 401 is what the user sees; the reference loops here instead.
#[tokio::test]
async fn an_ntlm_challenge_on_a_userinfo_hop_is_not_answered() {
    let server = MockServer::start().await;
    mount(&server, "/n", ntlm_challenge()).await;
    let url = format!("{}/n", with_userinfo(&server));
    let res = executor()
        .send(&resolved_one("GET", &url, NTLM, ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 401);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(received(&server, 0).await, [basic()]);
}

#[tokio::test]
async fn an_ntlm_challenge_after_a_redirect_to_userinfo_is_not_answered() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/start",
        redirect(&format!("{}/next", with_userinfo(&server))),
    )
    .await;
    mount(&server, "/next", ntlm_challenge()).await;
    let req = resolved_one("GET", &format!("{}/start", server.uri()), NTLM, "");
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 401);
    let requests = server.received_requests().await.unwrap();
    let next: Vec<usize> = (0..requests.len())
        .filter(|&i| requests[i].url.path() == "/next")
        .collect();
    assert_eq!(next.len(), 1, "the challenge was answered");
    assert_eq!(received(&server, next[0]).await, [basic()]);
}

// A request with a path param is rebuilt from the URL's origin, path and query, as the
// reference does, so its userinfo never reaches the wire and its own auth mode runs.
const PATH_ID: &str = "  params:\n    - name: id\n      value: \"7\"\n      type: path\n";
const PATH_ID_OFF: &str =
    "  params:\n    - name: id\n      value: \"7\"\n      type: path\n      disabled: true\n";

fn path_param_url(server: &MockServer) -> String {
    let url = server
        .uri()
        .replacen("http://", "http://acct-7781:pw-55@", 1);
    format!("{url}/users/:id")
}

fn holds_no_userinfo(res: &Response) {
    let mut urls = vec![res.url().to_owned()];
    urls.extend(res.timeline().iter().map(|hop| hop.url.clone()));
    for url in urls {
        for secret in ["acct-7781", "pw-55"] {
            assert!(!url.contains(secret), "{url}");
        }
    }
}

#[tokio::test]
async fn a_path_param_request_drops_the_url_userinfo() {
    let server = MockServer::start().await;
    mount(&server, "/users/7", ResponseTemplate::new(200)).await;
    let req = resolved_one("GET", &path_param_url(&server), PATH_ID, "");
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
    assert!(received(&server, 0).await.is_empty());
    holds_no_userinfo(&res);
}

#[tokio::test]
async fn a_path_param_request_sends_its_own_bearer_not_the_url_basic() {
    let server = MockServer::start().await;
    mount(&server, "/users/7", ResponseTemplate::new(200)).await;
    let under = format!("{PATH_ID}{BEARER}");
    let req = resolved_one("GET", &path_param_url(&server), &under, "");
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
    assert_eq!(received(&server, 0).await, ["Bearer OWNTOKEN55"]);
    holds_no_userinfo(&res);
}

#[tokio::test]
async fn a_path_param_request_answers_a_digest_challenge() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/users/7",
        ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            r#"Digest realm="r", nonce="n0nce", qop="auth", algorithm=MD5"#,
        ),
    )
    .await;
    let under = format!("{PATH_ID}  auth:\n    type: digest\n    username: du\n    password: dp\n");
    let req = resolved_one("GET", &path_param_url(&server), &under, "");
    executor().send(&req).await.expect("sent");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert!(received(&server, 0).await.is_empty());
    let answer = received(&server, 1).await;
    assert!(
        answer.len() == 1 && answer[0].starts_with("Digest "),
        "{answer:?}"
    );
}

#[tokio::test]
async fn a_disabled_path_param_still_drops_the_url_userinfo() {
    let server = MockServer::start().await;
    mount(&server, "/users/:id", ResponseTemplate::new(200)).await;
    let req = resolved_one("GET", &path_param_url(&server), PATH_ID_OFF, "");
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
    assert!(received(&server, 0).await.is_empty());
    holds_no_userinfo(&res);
}

#[tokio::test]
async fn query_params_alone_keep_the_url_basic() {
    let server = MockServer::start().await;
    mount(&server, "/one", ResponseTemplate::new(200)).await;
    let url = format!("{}/one", with_userinfo(&server));
    let query = "  params:\n    - name: q\n      value: \"1\"\n      type: query\n";
    let res = executor()
        .send(&resolved_one("GET", &url, query, ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    assert_eq!(received(&server, 0).await, [basic()]);
}

// Dropping the URL's userinfo for a path param is told to the user, without the credential.
const DROPPED: Warning = Warning::UrlCredentialsDropped;

#[tokio::test]
async fn a_path_param_url_with_userinfo_warns_that_its_credentials_were_dropped() {
    let server = MockServer::start().await;
    let req = resolved_one("GET", &path_param_url(&server), PATH_ID, "");
    assert!(req.warnings().contains(&DROPPED), "{:?}", req.warnings());
}

#[tokio::test]
async fn a_path_param_url_with_only_a_username_warns_too() {
    let server = MockServer::start().await;
    let url = server.uri().replacen("http://", "http://acct-7781@", 1);
    let req = resolved_one("GET", &format!("{url}/users/:id"), PATH_ID, "");
    assert!(req.warnings().contains(&DROPPED), "{:?}", req.warnings());
}

#[tokio::test]
async fn a_path_param_url_with_only_a_password_warns_and_never_sends_it() {
    let server = MockServer::start().await;
    mount(&server, "/users/7", ResponseTemplate::new(200)).await;
    let url = server.uri().replacen("http://", "http://:pw-55@", 1);
    let req = resolved_one("GET", &format!("{url}/users/:id"), PATH_ID, "");
    assert!(req.warnings().contains(&DROPPED), "{:?}", req.warnings());
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
    assert!(received(&server, 0).await.is_empty());
    holds_no_userinfo(&res);
}

#[tokio::test]
async fn a_path_param_url_without_userinfo_does_not_warn() {
    let server = MockServer::start().await;
    let url = format!("{}/users/:id", server.uri());
    let req = resolved_one("GET", &url, PATH_ID, "");
    assert!(!req.warnings().contains(&DROPPED), "{:?}", req.warnings());
}

#[tokio::test]
async fn userinfo_with_query_params_alone_does_not_warn() {
    let server = MockServer::start().await;
    let url = format!("{}/one", with_userinfo(&server));
    let query = "  params:\n    - name: q\n      value: \"1\"\n      type: query\n";
    let req = resolved_one("GET", &url, query, "");
    assert!(!req.warnings().contains(&DROPPED), "{:?}", req.warnings());
}

#[test]
fn the_dropped_credentials_warning_names_no_credential() {
    let text = DROPPED.to_string();
    assert_eq!(
        text,
        "credentials in the URL were not sent because the request has path params"
    );
    for secret in ["acct-7781", "pw-55", USER, PASS] {
        assert!(!text.contains(secret), "{text}");
    }
}
