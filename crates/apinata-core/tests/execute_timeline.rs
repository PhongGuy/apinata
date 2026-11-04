//! The per-hop timeline: what each hop sent and received.

use crate::common;

use common::{executor, resolved_one};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mount(server: &MockServer, at: &str, template: ResponseTemplate) {
    Mock::given(path(at))
        .respond_with(template)
        .mount(server)
        .await;
}

fn value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

#[tokio::test]
async fn every_hop_is_recorded_with_what_was_sent_and_received() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/login",
        ResponseTemplate::new(302)
            .insert_header("location", "/home?token=SECRET")
            .insert_header("set-cookie", "sid=SECRET; Path=/"),
    )
    .await;
    mount(
        &server,
        "/home",
        ResponseTemplate::new(200).set_body_string("hi"),
    )
    .await;
    let req = resolved_one(
        "POST",
        &format!("{}/login", server.uri()),
        "  headers:\n    - name: X-Trace\n      value: t1\n  body:\n    type: text\n    data: abc\n",
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    let hops = res.timeline();
    assert_eq!(hops.len(), 2);
    assert_eq!(
        (
            hops[0].method.as_str(),
            hops[0].url.as_str(),
            hops[0].status
        ),
        ("POST", format!("{}/login", server.uri()).as_str(), 302)
    );
    assert_eq!(
        (
            hops[1].method.as_str(),
            hops[1].url.as_str(),
            hops[1].status
        ),
        (
            "GET",
            format!("{}/home?token=SECRET", server.uri()).as_str(),
            200
        )
    );
    let first = &hops[0].request_headers;
    assert_eq!(value(first, "x-trace"), Some("t1"));
    assert_eq!(value(first, "content-type"), Some("text/plain"));
    assert_eq!(
        value(first, "accept"),
        Some("application/json, text/plain, */*")
    );
    assert!(value(first, "user-agent").is_some_and(|ua| ua.starts_with("apinata/")));
    assert_eq!(value(first, "cookie"), None);
    // The 302 turned the POST into a bodiless GET carrying the cookie the first hop set.
    let second = &hops[1].request_headers;
    assert_eq!(value(second, "cookie"), Some("sid=SECRET"));
    assert_eq!(value(second, "content-type"), None);
    assert_eq!(
        value(&hops[0].response_headers, "location"),
        Some("/home?token=SECRET")
    );
    assert_eq!(
        res.response_time(),
        hops[0].elapsed + hops[1].elapsed,
        "the response time sums the hops"
    );
    let shown = format!("{res:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    assert!(
        shown.contains("set-cookie") && shown.contains("x-trace"),
        "{shown}"
    );
}

#[tokio::test]
async fn each_hop_records_the_body_it_sent() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/login",
        ResponseTemplate::new(302).insert_header("location", "/home"),
    )
    .await;
    mount(&server, "/home", ResponseTemplate::new(200)).await;
    let req = resolved_one(
        "POST",
        &format!("{}/login", server.uri()),
        "  body:\n    type: text\n    data: s3cret-body\n",
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    let hops = res.timeline();
    assert_eq!(hops[0].request_body, b"s3cret-body");
    assert_eq!(hops[0].request_body_size, 11);
    assert!(hops[1].request_body.is_empty(), "the 302 resent no body");
    assert_eq!(hops[1].request_body_size, 0);
    let debug = format!("{:?}", hops[0]);
    assert!(!debug.contains("s3cret-body"), "{debug}");
}

#[tokio::test]
async fn a_large_body_is_recorded_only_up_to_the_limit() {
    use apinata_core::HOP_BODY_LIMIT;
    let server = MockServer::start().await;
    mount(&server, "/up", ResponseTemplate::new(200)).await;
    let data = "a".repeat(HOP_BODY_LIMIT + 10);
    let req = resolved_one(
        "POST",
        &format!("{}/up", server.uri()),
        &format!("  body:\n    type: text\n    data: {data}\n"),
        "",
    );
    let res = executor().send(&req).await.expect("sent");
    let hop = &res.timeline()[0];
    assert_eq!(hop.request_body.len(), HOP_BODY_LIMIT);
    assert_eq!(hop.request_body_size, (HOP_BODY_LIMIT + 10) as u64);
}
