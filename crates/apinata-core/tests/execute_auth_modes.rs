//! Digest challenges and SigV4 signing on the wire.

use crate::common;

use apinata_core::{AwsSharedFiles, Collection, ContextOptions, Hop, VarContext, resolve};
use common::{executor, request, resolved_one, write_tree};
use md5::{Digest as _, Md5};
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn header<'a>(hop: &'a Hop, name: &str) -> Option<&'a str> {
    hop.request_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// The value of `key` in a digest `Authorization` header.
fn field(auth: &str, key: &str) -> String {
    let start = auth
        .find(&format!("{key}=\""))
        .unwrap_or_else(|| panic!("no {key} in {auth}"))
        + key.len()
        + 2;
    auth[start..].split('"').next().unwrap().to_owned()
}

fn hex_md5(s: &str) -> String {
    // md-5 0.11 digests have no `LowerHex`, so the hex is built byte by byte.
    Md5::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Answers 200 when the MD5 qop=auth digest response is right for `user:realm:pass`, else 401.
struct DigestServer {
    user: &'static str,
    pass: &'static str,
    qop: bool,
}

impl Respond for DigestServer {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let challenge = if self.qop {
            r#"Digest realm="r", nonce="n0", qop="auth,auth-int", opaque="op""#
        } else {
            r#"Digest realm="r", nonce="n0""#
        };
        let unauthorized = ResponseTemplate::new(401)
            .insert_header("www-authenticate", challenge)
            .insert_header("set-cookie", "sid=1; Path=/");
        let Some(auth) = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .filter(|a| a.starts_with("Digest "))
        else {
            return unauthorized;
        };
        let ha1 = hex_md5(&format!("{}:r:{}", self.user, self.pass));
        let ha2 = hex_md5(&format!("{}:{}", req.method, field(auth, "uri")));
        let want = if self.qop {
            hex_md5(&format!(
                "{ha1}:n0:{}:{}:auth:{ha2}",
                field(auth, "nc"),
                field(auth, "cnonce")
            ))
        } else {
            hex_md5(&format!("{ha1}:n0:{ha2}"))
        };
        if field(auth, "response") == want {
            ResponseTemplate::new(200).set_body_string("ok")
        } else {
            unauthorized
        }
    }
}

fn digest_auth(user: &str, pass: &str) -> String {
    format!("  auth:\n    type: digest\n    username: \"{user}\"\n    password: \"{pass}\"\n")
}

#[tokio::test]
async fn a_digest_challenge_is_answered_once_with_the_body_resent() {
    let server = MockServer::start().await;
    Mock::given(path("/d"))
        .respond_with(DigestServer {
            user: "u",
            pass: "p",
            qop: true,
        })
        .mount(&server)
        .await;
    let body = "  body:\n    type: text\n    data: payload\n";
    let req = resolved_one(
        "POST",
        &format!("{}/d?x=1", server.uri()),
        &format!("{}{body}", digest_auth("u", "p")),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
    let hops = res.timeline();
    assert_eq!(
        hops.iter().map(|h| h.status).collect::<Vec<_>>(),
        [401, 200]
    );
    let auth = header(&hops[1], "authorization").expect("digest sent");
    assert!(
        auth.starts_with(r#"Digest username="u", realm="r", nonce="n0", uri="/d?x=1", response=""#),
        "{auth}"
    );
    assert!(auth.ends_with(r#"opaque="op""#), "{auth}");
    assert_eq!(hops[1].request_body, b"payload");
    assert_eq!(header(&hops[0], "authorization"), None);
    assert_eq!(header(&hops[1], "cookie"), Some("sid=1"));
}

#[tokio::test]
async fn the_digest_challenge_hop_records_the_cookie_it_set() {
    let server = MockServer::start().await;
    Mock::given(path("/d"))
        .respond_with(DigestServer {
            user: "u",
            pass: "p",
            qop: true,
        })
        .mount(&server)
        .await;
    let req = resolved_one(
        "GET",
        &format!("{}/d", server.uri()),
        &digest_auth("u", "p"),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    let hops = res.timeline();
    assert_eq!((hops.len(), hops[1].status), (2, 200));
    let first: Vec<_> = hops[0]
        .cookies
        .iter()
        .map(|c| (c.name.as_str(), c.outcome))
        .collect();
    assert_eq!(first, [("sid", apinata_core::CookieOutcome::Stored)]);
    assert!(hops[1].cookies.is_empty());
}

#[tokio::test]
async fn a_challenge_without_qop_uses_the_legacy_response() {
    let server = MockServer::start().await;
    Mock::given(path("/d"))
        .respond_with(DigestServer {
            user: "u",
            pass: "p",
            qop: false,
        })
        .mount(&server)
        .await;
    let req = resolved_one(
        "GET",
        &format!("{}/d", server.uri()),
        &digest_auth("u", "p"),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn a_wrong_password_returns_the_second_401_without_a_third_try() {
    let server = MockServer::start().await;
    Mock::given(path("/d"))
        .respond_with(DigestServer {
            user: "u",
            pass: "p",
            qop: true,
        })
        .mount(&server)
        .await;
    let req = resolved_one(
        "GET",
        &format!("{}/d", server.uri()),
        &digest_auth("u", "wrong"),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 401);
    assert_eq!(res.timeline().len(), 2);
}

#[tokio::test]
async fn no_retry_with_an_own_authorization_header_blank_credentials_or_a_bad_algorithm() {
    let server = MockServer::start().await;
    Mock::given(path("/d"))
        .respond_with(DigestServer {
            user: "u",
            pass: "p",
            qop: true,
        })
        .mount(&server)
        .await;
    Mock::given(path("/sha512"))
        .respond_with(ResponseTemplate::new(401).insert_header(
            "www-authenticate",
            r#"Digest realm="r", nonce="n", algorithm=SHA-512-256"#,
        ))
        .mount(&server)
        .await;
    let own = format!(
        "{}  headers:\n    - name: Authorization\n      value: Bearer x\n",
        digest_auth("u", "p")
    );
    let cases = [
        ("/d", own),
        ("/d", digest_auth("u", " ")),
        ("/sha512", digest_auth("u", "p")),
    ];
    for (at, under_http) in cases {
        let req = resolved_one("GET", &format!("{}{at}", server.uri()), &under_http, "");
        let res = executor().send(&req).await.expect("sent");
        assert_eq!(
            (res.status(), res.timeline().len()),
            (401, 1),
            "{at} {under_http}"
        );
    }
}

#[tokio::test]
async fn digest_after_a_redirect_uses_the_redirected_path() {
    let server = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/d"))
        .mount(&server)
        .await;
    Mock::given(path("/d"))
        .respond_with(DigestServer {
            user: "u",
            pass: "p",
            qop: true,
        })
        .mount(&server)
        .await;
    let req = resolved_one(
        "GET",
        &format!("{}/start", server.uri()),
        &digest_auth("u", "p"),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(
        res.timeline().iter().map(|h| h.status).collect::<Vec<_>>(),
        [302, 401, 200]
    );
    assert_eq!(
        field(header(&res.timeline()[2], "authorization").unwrap(), "uri"),
        "/d"
    );
}

fn aws_auth(token: &str) -> String {
    format!(
        "  auth:\n    type: awsv4\n    accessKeyId: AKIDTEST\n    secretAccessKey: secret-key\n    sessionToken: \"{token}\"\n    service: execute-api\n    region: eu-west-1\n    profileName: \"\"\n"
    )
}

#[tokio::test]
async fn sigv4_signs_each_same_origin_hop() {
    let server = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/b"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/b"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let req = resolved_one(
        "GET",
        &format!("{}/a", server.uri()),
        &aws_auth("sess-tok"),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
    let hops = res.timeline();
    let (a, b) = (
        header(&hops[0], "authorization").unwrap(),
        header(&hops[1], "authorization").unwrap(),
    );
    for auth in [a, b] {
        assert!(
            auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDTEST/"),
            "{auth}"
        );
        assert!(
            auth.contains("/eu-west-1/execute-api/aws4_request, SignedHeaders="),
            "{auth}"
        );
        assert!(
            auth.contains("host;x-amz-date;x-amz-security-token"),
            "{auth}"
        );
    }
    assert_ne!(a, b, "the second hop is signed for its own path");
    assert_eq!(header(&hops[1], "x-amz-security-token"), Some("sess-tok"));
    assert!(header(&hops[1], "x-amz-date").is_some());
}

#[tokio::test]
async fn a_cross_origin_hop_is_not_signed() {
    let (one, two) = (MockServer::start().await, MockServer::start().await);
    Mock::given(path("/a"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/b", two.uri())),
        )
        .mount(&one)
        .await;
    Mock::given(path("/b"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&two)
        .await;
    let req = resolved_one(
        "GET",
        &format!("{}/a", one.uri()),
        &aws_auth("sess-tok"),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    let last = &res.timeline()[1];
    assert_eq!(header(last, "authorization"), None);
    assert!(
        last.request_headers
            .iter()
            .all(|(n, _)| !n.to_ascii_lowercase().starts_with("x-amz-")),
        "{:?}",
        last.request_headers
    );
}

#[tokio::test]
async fn blank_keys_send_unsigned() {
    let server = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let block = aws_auth("").replace("AKIDTEST", "\"\"");
    let req = resolved_one("GET", &format!("{}/a", server.uri()), &block, "");
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(header(&res.timeline()[0], "authorization"), None);
}

#[tokio::test]
async fn profile_keys_reach_the_wire() {
    let server = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let aws = write_tree(&[
        (
            "credentials",
            "[dev]\naws_access_key_id = AKIAPROFILE\naws_secret_access_key = s\naws_session_token = ptok\n",
        ),
        ("config", ""),
    ]);
    let text = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{}/a\"\n  auth:\n    type: awsv4\n    accessKeyId: \"\"\n    secretAccessKey: \"\"\n    sessionToken: \"\"\n    service: execute-api\n    region: eu-west-1\n    profileName: dev\n",
        server.uri()
    );
    let dir = write_tree(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        ("r.yml", &text),
    ]);
    let c = Collection::open(dir.path()).unwrap();
    let files = AwsSharedFiles::new(aws.path().join("credentials"), aws.path().join("config"));
    let ctx = VarContext::new(&c, ContextOptions::new().aws_shared_files(files)).unwrap();
    let req = resolve(request(&c, "r.yml"), &ctx).unwrap();
    let res = executor().send(&req).await.expect("sent");
    let hop = &res.timeline()[0];
    assert!(
        header(hop, "authorization")
            .unwrap()
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIAPROFILE/")
    );
    assert_eq!(header(hop, "x-amz-security-token"), Some("ptok"));
}
