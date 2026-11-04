//! WSSE and Akamai EdgeGrid on the wire; the mock server recomputes each value.

use crate::common;

use apinata_core::{Hop, ResolvedRequest};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::{at, executor, resolved_one, setup};
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::{Digest as _, Sha256};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SECRET: &str = "eg-secret-1";
const SIGNED: &str = "content-type,x-custom,user-agent,accept";
const FIXED: &str = "    nonce: n0nce\n    timestamp: 20140321T19:34:21+0000\n";
const JSON: &str = "  body:\n    type: json\n    data: '{\"a\":1}'\n";
const CUSTOM: &str = "  headers:\n    - name: X-Custom\n      value: a  b\n";

fn edgegrid(fields: &str) -> String {
    edgegrid_signing(SIGNED, fields)
}

/// EdgeGrid auth that signs the comma-separated `signed` headers.
fn edgegrid_signing(signed: &str, fields: &str) -> String {
    format!(
        "  auth:\n    type: akamai-edgegrid\n    accessToken: akab-access-1\n    clientToken: akab-client-1\n    clientSecret: {SECRET}\n    headersToSign: \"{signed}\"\n{fields}"
    )
}

fn header<'a>(hop: &'a Hop, name: &str) -> Option<&'a str> {
    hop.request_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn received<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers.get(name).and_then(|v| v.to_str().ok())
}

fn hmac_b64(key: &[u8], data: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(data.as_bytes());
    STANDARD.encode(mac.finalize().into_bytes())
}

fn reply(then: Option<&'static str>) -> ResponseTemplate {
    match then {
        Some(to) => ResponseTemplate::new(302).insert_header("location", to),
        None => ResponseTemplate::new(200),
    }
}

/// Answers 200, or a 302 to `then`, when the EG1 signature over the request as received
/// verifies; else 401 with the data it signed.
struct VerifyEdgeGrid {
    then: Option<&'static str>,
}

impl Respond for VerifyEdgeGrid {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        VerifyEdgeGridOver {
            signed: SIGNED,
            then: self.then,
        }
        .respond(req)
    }
}

/// [`VerifyEdgeGrid`] over the comma-separated `signed` headers instead of [`SIGNED`].
struct VerifyEdgeGridOver {
    signed: &'static str,
    then: Option<&'static str>,
}

impl Respond for VerifyEdgeGridOver {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let Some((head, signature)) =
            received(req, "authorization").and_then(|a| a.split_once("signature="))
        else {
            return ResponseTemplate::new(401).set_body_string("no signature");
        };
        let timestamp = head
            .split(';')
            .find_map(|p| p.strip_prefix("timestamp="))
            .unwrap_or_default();
        // wiremock reports an origin-form target as `http://localhost`, so Host gives the authority.
        let host = received(req, "host").unwrap_or_default();
        let target = match req.url.query() {
            Some(q) if !q.is_empty() => format!("{}?{q}", req.url.path()),
            _ => req.url.path().to_owned(),
        };
        let canonical: Vec<String> = (self.signed.split(','))
            .filter_map(|n| {
                received(req, n)
                    .map(|v| format!("{n}:{}", v.split_whitespace().collect::<Vec<_>>().join(" ")))
            })
            .collect();
        let canonical = canonical.join("\t");
        let hash = if req.method.as_str() == "POST" && !req.body.is_empty() {
            STANDARD.encode(Sha256::digest(&req.body))
        } else {
            String::new()
        };
        let data = [
            req.method.as_str(),
            "http",
            host,
            target.as_str(),
            canonical.as_str(),
            hash.as_str(),
            head,
        ]
        .join("\t");
        let key = hmac_b64(SECRET.as_bytes(), timestamp);
        if hmac_b64(key.as_bytes(), &data) == signature {
            reply(self.then)
        } else {
            ResponseTemplate::new(401).set_body_string(format!("bad signature over {data:?}"))
        }
    }
}

/// Answers 200, or a 302 to `then`, when exactly one X-WSSE header digests `password`.
struct VerifyWsse {
    password: &'static str,
    then: Option<&'static str>,
}

impl Respond for VerifyWsse {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let all: Vec<&str> = req
            .headers
            .get_all("x-wsse")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        let [value] = all.as_slice() else {
            return ResponseTemplate::new(401)
                .set_body_string(format!("{} X-WSSE headers", all.len()));
        };
        let field = |name: &str| {
            value
                .split(&format!("{name}=\""))
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap_or_default()
                .to_owned()
        };
        let (nonce, created) = (field("Nonce"), field("Created"));
        let hex: String = Sha1::digest(format!("{nonce}{created}{}", self.password))
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if field("PasswordDigest") == STANDARD.encode(hex)
            && nonce.len() == 32
            && created.ends_with('Z')
        {
            reply(self.then)
        } else {
            ResponseTemplate::new(401).set_body_string(format!("bad digest in {value}"))
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
async fn edgegrid_signs_over_the_default_headers_with_a_fixed_nonce() {
    let server = MockServer::start().await;
    mount(&server, "/p", VerifyEdgeGrid { then: None }).await;
    let under = format!("{CUSTOM}{}{JSON}", edgegrid(FIXED));
    let res = executor()
        .send(&resolved_one(
            "POST",
            &format!("{}/p?a=1", server.uri()),
            &under,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let auth = header(&res.timeline()[0], "authorization").expect("signed");
    assert!(auth.starts_with("EG1-HMAC-SHA256 client_token=akab-client-1;access_token=akab-access-1;timestamp=20140321T19:34:21+0000;nonce=n0nce;signature="), "{auth}");
}

#[tokio::test]
async fn each_hop_is_signed_again_and_a_302_hop_has_no_content_hash() {
    let server = MockServer::start().await;
    mount(&server, "/start", VerifyEdgeGrid { then: Some("/end") }).await;
    mount(&server, "/end", VerifyEdgeGrid { then: None }).await;
    let under = format!("{}{JSON}", edgegrid(FIXED));
    let res = executor()
        .send(&resolved_one(
            "POST",
            &format!("{}/start", server.uri()),
            &under,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let hops = res.timeline();
    assert_eq!((hops.len(), hops[1].method.as_str()), (2, "GET"));
    assert_ne!(
        header(&hops[0], "authorization"),
        header(&hops[1], "authorization")
    );
}

#[tokio::test]
async fn a_blank_nonce_and_timestamp_are_fresh_on_each_hop() {
    let server = MockServer::start().await;
    mount(&server, "/start", VerifyEdgeGrid { then: Some("/end") }).await;
    mount(&server, "/end", VerifyEdgeGrid { then: None }).await;
    let under = edgegrid("    nonce: \"\"\n    timestamp: \"\"\n");
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/start", server.uri()),
            &under,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let nonce = |hop: &Hop| {
        header(hop, "authorization")
            .unwrap()
            .split(';')
            .find_map(|p| p.strip_prefix("nonce="))
            .unwrap()
            .to_owned()
    };
    let hops = res.timeline();
    assert_eq!(nonce(&hops[0]).len(), 36);
    assert_ne!(nonce(&hops[0]), nonce(&hops[1]));
}

#[tokio::test]
async fn wsse_is_one_header_over_the_interpolated_password_on_every_hop() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/start",
        VerifyWsse {
            password: "wsse-pass-1",
            then: Some("/end"),
        },
    )
    .await;
    mount(
        &server,
        "/end",
        VerifyWsse {
            password: "wsse-pass-1",
            then: None,
        },
    )
    .await;
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: pw\n      value: wsse-pass-1\n";
    let r = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{}/start\"\n  headers:\n    - name: x-wsse\n      value: mine\n  auth:\n    type: wsse\n    username: bob\n    password: \"{{{{pw}}}}\"\n",
        server.uri()
    );
    let (_d, c, ctx) = setup(&[("opencollection.yml", root), ("r.yml", &r)]);
    let res = executor()
        .send(&at(&c, &ctx, "r.yml").expect("resolves"))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let hops = res.timeline();
    assert_eq!(hops.len(), 2);
    assert_eq!(
        header(&hops[0], "x-wsse"),
        header(&hops[1], "x-wsse"),
        "built once per send"
    );
    assert!(
        header(&hops[0], "x-wsse")
            .unwrap()
            .starts_with("UsernameToken Username=\"bob\", PasswordDigest=\"")
    );
}

#[tokio::test]
async fn folder_and_collection_auth_is_inherited_on_the_wire() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/w",
        VerifyWsse {
            password: "fp",
            then: None,
        },
    )
    .await;
    mount(&server, "/e", VerifyEdgeGrid { then: None }).await;
    // The collection signs with EdgeGrid; folder `w` overrides it with WSSE, folder `e` inherits.
    let root = format!(
        "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n{}",
        edgegrid(FIXED)
    );
    let folder =
        |name: &str, auth: &str| format!("info:\n  name: {name}\n  type: folder\nrequest:\n{auth}");
    let req = |p: &str| {
        format!(
            "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{}/{p}\"\n  auth: inherit\n",
            server.uri()
        )
    };
    let (_d, c, ctx) = setup(&[
        ("opencollection.yml", &root),
        (
            "w/folder.yml",
            &folder(
                "w",
                "  auth:\n    type: wsse\n    username: folder\n    password: fp\n",
            ),
        ),
        ("w/r.yml", &req("w")),
        ("e/folder.yml", &folder("e", "  auth: inherit\n")),
        ("e/r.yml", &req("e")),
    ]);
    for rel in ["w/r.yml", "e/r.yml"] {
        let res = executor()
            .send(&at(&c, &ctx, rel).expect("resolves"))
            .await
            .expect("sent");
        assert_eq!(
            res.status(),
            200,
            "{rel}: {}",
            String::from_utf8_lossy(res.body())
        );
    }
}

/// A WSSE GET of `{from}/start`, whose answer redirects it, with credential forwarding set.
fn wsse_redirect(from: &str, forward: bool) -> ResolvedRequest {
    let under = "  auth:\n    type: wsse\n    username: bob\n    password: wsse-pass-1\n";
    let settings = format!("settings:\n  forwardAuthorizationHeader: {forward}\n");
    resolved_one("GET", &format!("{from}/start"), under, &settings)
}

/// Two servers where `/start` on the first answers 302 to `/end` on the second.
async fn cross_origin() -> (MockServer, MockServer) {
    let (one, two) = (MockServer::start().await, MockServer::start().await);
    let to = format!("{}/end", two.uri());
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", to.as_str()))
        .mount(&one)
        .await;
    (one, two)
}

#[tokio::test]
async fn a_cross_origin_hop_drops_the_wsse_header() {
    let (one, two) = cross_origin().await;
    mount(&two, "/end", ResponseTemplate::new(200)).await;
    let res = executor()
        .send(&wsse_redirect(&one.uri(), false))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    let hops = res.timeline();
    assert!(header(&hops[0], "x-wsse").is_some());
    assert_eq!(header(&hops[1], "x-wsse"), None);
    let arrived = two.received_requests().await.expect("recorded");
    assert_eq!(received(&arrived[0], "x-wsse"), None);
}

#[tokio::test]
async fn a_same_origin_hop_keeps_the_wsse_header() {
    let server = MockServer::start().await;
    mount(&server, "/start", reply(Some("/end"))).await;
    mount(&server, "/end", ResponseTemplate::new(200)).await;
    let res = executor()
        .send(&wsse_redirect(&server.uri(), false))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    let arrived = server.received_requests().await.expect("recorded");
    assert_eq!(arrived.len(), 2);
    assert!(received(&arrived[1], "x-wsse").is_some());
    assert_eq!(
        received(&arrived[0], "x-wsse"),
        received(&arrived[1], "x-wsse")
    );
}

#[tokio::test]
async fn a_cross_origin_hop_keeps_the_wsse_header_when_forwarding_is_on() {
    let (one, two) = cross_origin().await;
    mount(&two, "/end", ResponseTemplate::new(200)).await;
    let res = executor()
        .send(&wsse_redirect(&one.uri(), true))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    let hops = res.timeline();
    let first = header(&hops[0], "x-wsse");
    assert!(first.is_some());
    assert_eq!(header(&hops[1], "x-wsse"), first);
    let arrived = two.received_requests().await.expect("recorded");
    assert_eq!(received(&arrived[0], "x-wsse"), first);
}

/// An EdgeGrid GET of `{from}/start`, whose answer redirects it, with credential forwarding set.
fn edgegrid_redirect(from: &str, forward: bool) -> ResolvedRequest {
    let settings = format!("settings:\n  forwardAuthorizationHeader: {forward}\n");
    resolved_one("GET", &format!("{from}/start"), &edgegrid(FIXED), &settings)
}

#[tokio::test]
async fn a_cross_origin_hop_is_not_edgegrid_signed_by_default() {
    let (one, two) = cross_origin().await;
    mount(&two, "/end", ResponseTemplate::new(200)).await;
    let res = executor()
        .send(&edgegrid_redirect(&one.uri(), false))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    let hops = res.timeline();
    assert!(header(&hops[0], "authorization").is_some());
    assert_eq!(header(&hops[1], "authorization"), None);
    let arrived = two.received_requests().await.expect("recorded");
    assert_eq!(received(&arrived[0], "authorization"), None);
}

#[tokio::test]
async fn a_cross_origin_hop_is_edgegrid_signed_for_its_host_when_forwarding_is_on() {
    let (one, two) = cross_origin().await;
    mount(&two, "/end", VerifyEdgeGrid { then: None }).await;
    let res = executor()
        .send(&edgegrid_redirect(&one.uri(), true))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
}

#[tokio::test]
async fn an_omitted_default_header_is_left_out_of_the_edgegrid_signature() {
    let server = MockServer::start().await;
    mount(&server, "/", VerifyEdgeGrid { then: None }).await;
    // The signed list names user-agent, but the request omits it, so the verifier (which signs
    // only what arrived) agrees only if the signer left it out too.
    let under = edgegrid(FIXED);
    let top = "settings:\n  omitHeaders:\n    - User-Agent\n";
    let res = executor()
        .send(&resolved_one("GET", &server.uri(), &under, top))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
}

/// The Cookie `/p` received when sent with EdgeGrid signing only `cookie`, with `own` headers,
/// after `/set` stored `b=3` in the executor's jar; the send must verify.
async fn signed_cookie(own: &str) -> Option<String> {
    let server = MockServer::start().await;
    mount(
        &server,
        "/set",
        ResponseTemplate::new(200).insert_header("set-cookie", "b=3; Path=/"),
    )
    .await;
    let verify = VerifyEdgeGridOver {
        signed: "cookie",
        then: None,
    };
    mount(&server, "/p", verify).await;
    let exec = executor();
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    let under = format!("{own}{}", edgegrid_signing("cookie", FIXED));
    let res = exec
        .send(&resolved_one(
            "GET",
            &format!("{}/p", server.uri()),
            &under,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let arrived = server.received_requests().await.expect("recorded");
    received(&arrived[1], "cookie").map(str::to_owned)
}

#[tokio::test]
async fn edgegrid_signs_the_cookie_merged_with_the_jar() {
    let own = "  headers:\n    - name: Cookie\n      value: a=1; b=2\n";
    assert_eq!(signed_cookie(own).await.as_deref(), Some("a=1; b=3"));
}

#[tokio::test]
async fn edgegrid_signs_the_jar_cookie_when_there_is_no_own_header() {
    assert_eq!(signed_cookie("").await.as_deref(), Some("b=3"));
}

#[tokio::test]
async fn a_same_origin_redirect_hop_signs_the_jars_cookie() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/set",
        ResponseTemplate::new(200).insert_header("set-cookie", "j=7; Path=/end"),
    )
    .await;
    let signs = |then| VerifyEdgeGridOver {
        signed: "cookie",
        then,
    };
    mount(&server, "/start", signs(Some("/end"))).await;
    mount(&server, "/end", signs(None)).await;
    let exec = executor();
    let set = resolved_one("GET", &format!("{}/set", server.uri()), "", "");
    exec.send(&set).await.expect("sent");
    let under = edgegrid_signing("cookie", FIXED);
    let start = resolved_one("GET", &format!("{}/start", server.uri()), &under, "");
    let res = exec.send(&start).await.expect("sent");
    assert_eq!(res.status(), 200, "{}", String::from_utf8_lossy(res.body()));
    let arrived = server.received_requests().await.expect("recorded");
    assert_eq!(arrived.len(), 3);
    assert_eq!(received(&arrived[1], "cookie"), None);
    assert_eq!(received(&arrived[2], "cookie"), Some("j=7"));
}
