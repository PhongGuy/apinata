//! Hawk through resolve and send: the mock server recomputes each hop's MAC.

use crate::common;
use crate::derived_support::{on_wire, sent_text};

use apinata_core::{Hop, ResolveError};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::{at, executor, resolved_one, setup};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest as _, Sha256};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = "hawk-key-value-1";
const JSON: &str = "  body:\n    type: json\n    data: '{\"a\":1}'\n";

fn hawk(fields: &str) -> String {
    format!("  auth:\n    type: hawk\n    authId: id-1\n    authKey: {KEY}\n{fields}")
}

fn header<'a>(hop: &'a Hop, name: &str) -> Option<&'a str> {
    hop.request_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// The quoted attribute `name` of a Hawk header.
fn attr(value: &str, name: &str) -> String {
    value
        .split(&format!("{name}=\""))
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap_or_default()
        .to_owned()
}

/// Answers 200, or a 302 to `then`, when the Hawk MAC (and body hash, when sent) over the
/// request as received verifies with [`KEY`]; else 401 saying why.
struct VerifyHawk {
    then: Option<&'static str>,
}

impl Respond for VerifyHawk {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let Some(value) = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.starts_with("Hawk "))
        else {
            return ResponseTemplate::new(401).set_body_string("no Hawk header");
        };
        let hash = attr(value, "hash");
        if !hash.is_empty() {
            let ct = req
                .headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            let media = ct
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let mut text = format!("hawk.1.payload\n{media}\n").into_bytes();
            text.extend_from_slice(&req.body);
            text.push(b'\n');
            if STANDARD.encode(Sha256::digest(&text)) != hash {
                return ResponseTemplate::new(401).set_body_string("bad hash");
            }
        }
        // wiremock reports an origin-form target as `http://localhost`, so Host gives the authority.
        let host = req
            .headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let (name, port) = host.rsplit_once(':').unwrap_or((host, "80"));
        let resource = match req.url.query() {
            Some(q) => format!("{}?{q}", req.url.path()),
            None => req.url.path().to_owned(),
        };
        let normalized = format!(
            "hawk.1.header\n{}\n{}\n{}\n{resource}\n{name}\n{port}\n{hash}\n{}\n",
            attr(value, "ts"),
            attr(value, "nonce"),
            req.method.as_str(),
            attr(value, "ext"),
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(KEY.as_bytes()).unwrap();
        mac.update(normalized.as_bytes());
        if STANDARD.encode(mac.finalize().into_bytes()) != attr(value, "mac") {
            return ResponseTemplate::new(401)
                .set_body_string(format!("bad mac over {normalized:?}"));
        }
        match self.then {
            Some(to) => ResponseTemplate::new(302).insert_header("location", to),
            None => ResponseTemplate::new(200),
        }
    }
}

async fn mount(server: &MockServer, at: &str, responder: impl Respond + 'static) {
    Mock::given(path(at))
        .respond_with(responder)
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_post_with_a_payload_hash_verifies_and_carries_the_fixed_values() {
    let server = MockServer::start().await;
    mount(&server, "/p", VerifyHawk { then: None }).await;
    let auth = hawk(
        "    ext: some-ext\n    nonce: j4h3g2\n    timestamp: 1353832234\n    includePayloadHash: true\n",
    );
    let res = executor()
        .send(&resolved_one(
            "POST",
            &format!("{}/p?b=1&a=2", server.uri()),
            &format!("{auth}{JSON}"),
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let value = header(&res.timeline()[0], "authorization").expect("signed");
    assert!(
        value.starts_with("Hawk id=\"id-1\", ts=\"1353832234\", nonce=\"j4h3g2\", hash=\""),
        "{value}"
    );
    assert!(value.contains(", ext=\"some-ext\", mac=\""), "{value}");
}

#[tokio::test]
async fn each_hop_is_signed_again_with_a_fresh_nonce() {
    let server = MockServer::start().await;
    mount(&server, "/start", VerifyHawk { then: Some("/end") }).await;
    mount(&server, "/end", VerifyHawk { then: None }).await;
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/start", server.uri()),
            &hawk(""),
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let hops = res.timeline();
    assert_eq!(hops.len(), 2);
    let nonce = |hop: &Hop| attr(header(hop, "authorization").expect("signed"), "nonce");
    assert_eq!(nonce(&hops[0]).len(), 6);
    assert_ne!(nonce(&hops[0]), nonce(&hops[1]));
}

#[tokio::test]
async fn a_hop_to_another_origin_is_not_signed_when_forwarding_is_off() {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    let elsewhere: &'static str = Box::leak(format!("{}/end", second.uri()).into_boxed_str());
    mount(
        &first,
        "/start",
        VerifyHawk {
            then: Some(elsewhere),
        },
    )
    .await;
    Mock::given(path("/end"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&second)
        .await;
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/start", first.uri()),
            &hawk(""),
            "settings:\n  forwardAuthorizationHeader: false\n",
        ))
        .await
        .expect("sent");
    let hops = res.timeline();
    assert_eq!(hops.len(), 2, "{}", String::from_utf8_lossy(res.body()));
    assert!(header(&hops[0], "authorization").is_some());
    assert_eq!(header(&hops[1], "authorization"), None);
}

#[tokio::test]
async fn the_header_and_mac_are_derived_and_the_key_is_sensitive() {
    let server = MockServer::start().await;
    mount(&server, "/p", VerifyHawk { then: None }).await;
    let req = resolved_one("GET", &format!("{}/p", server.uri()), &hawk(""), "");
    assert!(req.sensitive_values().iter().any(|v| v == KEY));
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let derived = res.derived_credentials();
    let value = header(&res.timeline()[0], "authorization").expect("signed");
    assert!(derived.iter().any(|v| v == value), "{derived:?}");
    assert!(
        derived.iter().any(|v| *v == attr(value, "mac")),
        "{derived:?}"
    );
    assert!(!on_wire(&res, KEY), "{}", sent_text(&res));
}

#[test]
fn a_missing_id_fails_resolve_without_quoting_the_key() {
    let (_d, c, ctx) = setup(&[
        (
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        ),
        (
            "r.yml",
            &format!(
                "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/\n  auth:\n    type: hawk\n    authKey: {KEY}\n"
            ),
        ),
    ]);
    let err = at(&c, &ctx, "r.yml").expect_err("refused");
    assert!(matches!(err, ResolveError::Hawk { .. }), "{err:?}");
    assert!(err.to_string().contains("authId"), "{err}");
    assert!(!err.to_string().contains(KEY), "{err}");
}
