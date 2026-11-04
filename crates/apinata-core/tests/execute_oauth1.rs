//! OAuth 1.0 signing on the wire; the mock server recomputes the HMAC-SHA1 signature.

use crate::common;

use apinata_core::Hop;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::{executor, resolved_one};
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use sha1::{Digest as _, Sha1};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const CS: &str = "cs-1";
const TS: &str = "ts-1";
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const JSON: &str = "  body:\n    type: json\n    data: '{\"a\":1}'\n";

fn auth(placement: &str, extra: &str) -> String {
    format!(
        "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: {CS}\n    accessToken: tk\n    accessTokenSecret: {TS}\n    signatureMethod: HMAC-SHA1\n    placement: {placement}\n{extra}"
    )
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, UNRESERVED).to_string()
}

fn header<'a>(hop: &'a Hop, name: &str) -> Option<&'a str> {
    hop.request_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Every parameter the request carries: query, form body and `Authorization` (realm left out).
fn params(req: &Request) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = req
        .url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let ct = req
        .headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ct.starts_with("application/x-www-form-urlencoded") {
        out.extend(url::form_urlencoded::parse(&req.body).into_owned());
    }
    if let Some(rest) = req
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|a| a.strip_prefix("OAuth "))
    {
        for part in rest.split(", ") {
            let (k, v) = part.split_once('=').expect("k=\"v\"");
            if k != "realm" {
                let v = percent_decode_str(v.trim_matches('"'))
                    .decode_utf8_lossy()
                    .into_owned();
                out.push((k.to_owned(), v));
            }
        }
    }
    out
}

/// Answers 200 when exactly one HMAC-SHA1 signature verifies (and, with `body_hash`, the body
/// hash matches the body), else 401 with the reason.
struct Verify {
    body_hash: bool,
}

impl Respond for Verify {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let mut all = params(req);
        let Some(at) = all.iter().position(|(k, _)| k == "oauth_signature") else {
            return ResponseTemplate::new(401).set_body_string("no signature");
        };
        let (_, signature) = all.remove(at);
        if all.iter().any(|(k, _)| k == "oauth_signature") {
            return ResponseTemplate::new(401).set_body_string("two signatures");
        }
        if self.body_hash {
            let want = STANDARD.encode(Sha1::digest(&req.body));
            if !all
                .iter()
                .any(|(k, v)| k == "oauth_body_hash" && *v == want)
            {
                return ResponseTemplate::new(401).set_body_string("body hash");
            }
        }
        let mut pairs: Vec<(String, String)> = all.iter().map(|(k, v)| (enc(k), enc(v))).collect();
        pairs.sort();
        let joined: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
        // wiremock reports an origin-form target as `http://localhost`, so Host gives the authority.
        let host = req
            .headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("localhost");
        let base_url = format!("http://{host}{}", req.url.path());
        let base = format!(
            "{}&{}&{}",
            req.method.as_str(),
            enc(&base_url),
            enc(&joined.join("&"))
        );
        let mut mac =
            Hmac::<Sha1>::new_from_slice(format!("{}&{}", enc(CS), enc(TS)).as_bytes()).unwrap();
        mac.update(base.as_bytes());
        if STANDARD.encode(mac.finalize().into_bytes()) == signature {
            ResponseTemplate::new(200)
        } else {
            ResponseTemplate::new(401).set_body_string(format!("bad signature over {base}"))
        }
    }
}

/// Redirects to `to` plus the incoming query, as a server echoing it would.
struct Echo {
    status: u16,
    to: String,
}

impl Respond for Echo {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let query = req.url.query().map(|q| format!("?{q}")).unwrap_or_default();
        ResponseTemplate::new(self.status).insert_header("location", format!("{}{query}", self.to))
    }
}

async fn verifying(at: &str, body_hash: bool) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path(at))
        .respond_with(Verify { body_hash })
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn each_placement_verifies_on_the_server() {
    for (placement, method, body) in [
        ("header", "POST", JSON),
        ("query", "GET", ""),
        ("body", "PUT", JSON),
    ] {
        let server = verifying("/p", false).await;
        let under = format!("{}{body}", auth(placement, ""));
        let req = resolved_one(method, &format!("{}/p?a=1", server.uri()), &under, "");
        let res = executor().send(&req).await.expect("sent");
        assert_eq!(
            res.status(),
            200,
            "{placement}: {}",
            String::from_utf8_lossy(res.body())
        );
        let hop = &res.timeline()[0];
        match placement {
            "header" => assert!(
                header(hop, "authorization").is_some_and(|a| a.starts_with("OAuth ")),
                "{hop:?}"
            ),
            "query" => assert!(hop.url.contains("oauth_signature="), "{}", hop.url),
            _ => {
                assert_eq!(
                    header(hop, "content-type"),
                    Some("application/x-www-form-urlencoded")
                );
                assert!(
                    String::from_utf8_lossy(&hop.request_body).starts_with("oauth_consumer_key=")
                );
            }
        }
    }
}

#[tokio::test]
async fn form_bodies_body_hashes_and_realms_sign() {
    let server = verifying("/f", false).await;
    let form = "  body:\n    type: form-urlencoded\n    data:\n      - name: b\n        value: x\n";
    let under = format!("{}{form}", auth("header", "    realm: photos\n"));
    let res = executor()
        .send(&resolved_one(
            "POST",
            &format!("{}/f", server.uri()),
            &under,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let a = header(&res.timeline()[0], "authorization").unwrap();
    assert!(
        a.starts_with("OAuth realm=\"photos\", oauth_consumer_key=\"ck\""),
        "{a}"
    );
    assert!(!a.contains("oauth_body_hash"), "{a}");

    let server = verifying("/h", true).await;
    let text = "  body:\n    type: text\n    data: hello\n";
    let under = format!("{}{text}", auth("header", "    includeBodyHash: true\n"));
    let res = executor()
        .send(&resolved_one(
            "POST",
            &format!("{}/h", server.uri()),
            &under,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
}

#[tokio::test]
async fn a_same_origin_redirect_is_re_signed_without_the_old_signature() {
    let server = verifying("/b", false).await;
    Mock::given(path("/a"))
        .respond_with(Echo {
            status: 307,
            to: format!("{}/b", server.uri()),
        })
        .mount(&server)
        .await;
    let req = resolved_one(
        "GET",
        &format!("{}/a?x=1", server.uri()),
        &auth("query", ""),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let [first, second] = res.timeline() else {
        panic!("two hops: {:?}", res.timeline())
    };
    assert_eq!(
        second.url.matches("oauth_signature=").count(),
        1,
        "{}",
        second.url
    );
    let nonce = |u: &str| {
        u.split("oauth_nonce=")
            .nth(1)
            .map(|s| s.split('&').next().unwrap().to_owned())
    };
    assert_ne!(nonce(&first.url), nonce(&second.url));
}

#[tokio::test]
async fn a_cross_origin_hop_carries_no_oauth_parameters() {
    for placement in ["header", "query"] {
        let (one, two) = (MockServer::start().await, MockServer::start().await);
        Mock::given(path("/a"))
            .respond_with(Echo {
                status: 302,
                to: format!("{}/b", two.uri()),
            })
            .mount(&one)
            .await;
        Mock::given(path("/b"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&two)
            .await;
        let req = resolved_one("GET", &format!("{}/a", one.uri()), &auth(placement, ""), "");
        let res = executor().send(&req).await.expect("sent");
        let last = &res.timeline()[1];
        assert_eq!(header(last, "authorization"), None, "{placement}");
        assert!(!last.url.contains("oauth_"), "{placement}: {}", last.url);
    }
}

#[tokio::test]
async fn body_placement_drops_a_stale_content_length() {
    let server = verifying("/l", false).await;
    let headers = "  headers:\n    - name: Content-Length\n      value: \"7\"\n";
    let under = format!("{}{JSON}{headers}", auth("body", ""));
    let req = resolved_one("POST", &format!("{}/l", server.uri()), &under, "");
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    assert_eq!(header(&res.timeline()[0], "content-length"), None);
}

/// `text` with each value replaced, longest first, as substring redaction does.
fn scrub(text: &str, values: &[String]) -> String {
    let mut values: Vec<&String> = values.iter().collect();
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    values
        .iter()
        .fold(text.to_owned(), |t, v| t.replace(v.as_str(), "[redacted]"))
}

fn form(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[tokio::test]
async fn plaintext_signatures_with_reserved_characters_are_redactable() {
    let (cs, ts) = ("c s&+/=?x", "t:s%#y");
    for (placement, method) in [("query", "GET"), ("body", "POST")] {
        let server = MockServer::start().await;
        Mock::given(path("/s"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let under = format!(
            "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: '{cs}'\n    accessToken: tk\n    accessTokenSecret: '{ts}'\n    signatureMethod: PLAINTEXT\n    placement: {placement}\n"
        );
        let req = resolved_one(method, &format!("{}/s", server.uri()), &under, "");
        let res = executor().send(&req).await.expect("sent");
        let hop = &res.timeline()[0];
        let sent = format!("{} {}", hop.url, String::from_utf8_lossy(&hop.request_body));
        let signature = form(&format!("{}&{}", enc(cs), enc(ts)));
        assert!(sent.contains(&signature), "{placement}: {sent}");
        let scrubbed = scrub(&sent, req.sensitive_values());
        for secret in [cs, ts] {
            for shown in [secret.to_owned(), enc(secret), form(&enc(secret))] {
                assert!(
                    !scrubbed.contains(&shown),
                    "{placement}: {shown} in {scrubbed}"
                );
            }
        }
    }
}
