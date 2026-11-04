//! Bruno's manual redirect loop: limits, method rewriting, cross-origin header stripping, timing.

use crate::common;

use common::{executor, resolved_one};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn redirect(status: u16, to: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).insert_header("location", to)
}

/// `/r0` .. `/r5` each redirect to the next; `/r6` answers 200.
async fn chain() -> MockServer {
    let server = MockServer::start().await;
    for n in 0..6 {
        Mock::given(path(format!("/r{n}")))
            .respond_with(redirect(302, &format!("/r{}", n + 1)))
            .mount(&server)
            .await;
    }
    Mock::given(path("/r6"))
        .respond_with(ResponseTemplate::new(200).set_body_string("end"))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn the_limit_returns_the_last_redirect_instead_of_failing() {
    let server = chain().await;
    let url = format!("{}/r0", server.uri());
    let res = executor()
        .send(&resolved_one("GET", &url, "", ""))
        .await
        .expect("sent");
    assert_eq!(res.status(), 302);
    assert_eq!(res.url(), format!("{}/r5", server.uri()));
    assert!(
        res.headers()
            .contains(&("location".to_owned(), "/r6".to_owned()))
    );
    assert_eq!(server.received_requests().await.expect("recorded").len(), 6);

    let six = resolved_one("GET", &url, "", "settings:\n  maxRedirects: 6\n");
    let res = executor().send(&six).await.expect("sent");
    assert_eq!((res.status(), res.text()), (200, "end"));
    assert_eq!(res.url(), format!("{}/r6", server.uri()));
}

#[tokio::test]
async fn follow_redirects_false_and_a_missing_location_return_the_3xx() {
    let server = chain().await;
    Mock::given(path("/nowhere"))
        .respond_with(ResponseTemplate::new(301))
        .mount(&server)
        .await;
    let url = format!("{}/r0", server.uri());
    let off = resolved_one("GET", &url, "", "settings:\n  followRedirects: false\n");
    assert_eq!(executor().send(&off).await.expect("sent").status(), 302);
    let zero = resolved_one("GET", &url, "", "settings:\n  maxRedirects: 0\n");
    assert_eq!(executor().send(&zero).await.expect("sent").status(), 302);
    let nowhere = resolved_one("GET", &format!("{}/nowhere", server.uri()), "", "");
    assert_eq!(executor().send(&nowhere).await.expect("sent").status(), 301);
}

#[tokio::test]
async fn method_changing_codes_switch_to_get_and_drop_the_body() {
    let json = "  headers:\n    - name: X-Keep\n      value: k\n  body:\n    type: json\n    data: '{\"a\":1}'\n";
    for (code, expected_method, keeps_body) in [
        (301, "GET", false),
        (302, "GET", false),
        (303, "GET", false),
        (307, "POST", true),
        (308, "POST", true),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/from"))
            .respond_with(redirect(code, "/to"))
            .mount(&server)
            .await;
        Mock::given(path("/to"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let req = resolved_one("POST", &format!("{}/from", server.uri()), json, "");
        assert_eq!(
            executor().send(&req).await.expect("sent").status(),
            200,
            "{code}"
        );
        let received = server.received_requests().await.expect("recorded");
        let hop = &received[1];
        assert_eq!(hop.method.as_str(), expected_method, "{code}");
        assert_eq!(
            hop.headers.get("x-keep").map(|v| v.as_bytes()),
            Some(&b"k"[..]),
            "{code}"
        );
        assert_eq!(
            hop.headers.contains_key("content-type"),
            keeps_body,
            "{code}"
        );
        let body: &[u8] = if keeps_body { b"{\"a\":1}" } else { b"" };
        assert_eq!(hop.body, body, "{code}");
    }
}

#[tokio::test]
async fn head_stays_head_and_relative_locations_resolve_against_the_hop() {
    let server = MockServer::start().await;
    Mock::given(path("/dir/a"))
        .respond_with(redirect(302, "b"))
        .mount(&server)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/dir/b"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let req = resolved_one("HEAD", &format!("{}/dir/a", server.uri()), "", "");
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 204);
    assert_eq!(res.url(), format!("{}/dir/b", server.uri()));
}

#[tokio::test]
async fn authorization_crosses_origins_unless_forwarding_is_off() {
    let bearer = "  auth:\n    type: bearer\n    token: t\n";
    for (forward, expected) in [("true", Some("Bearer t")), ("false", None)] {
        let (a, b) = (MockServer::start().await, MockServer::start().await);
        Mock::given(path("/start"))
            .respond_with(redirect(302, "/same"))
            .mount(&a)
            .await;
        Mock::given(path("/same"))
            .respond_with(redirect(307, &format!("{}/other", b.uri())))
            .mount(&a)
            .await;
        Mock::given(path("/other"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&b)
            .await;
        let settings = format!("settings:\n  forwardAuthorizationHeader: {forward}\n");
        let req = resolved_one("GET", &format!("{}/start", a.uri()), bearer, &settings);
        assert_eq!(executor().send(&req).await.expect("sent").status(), 200);
        let on_a = a.received_requests().await.expect("recorded");
        let same_origin = on_a[1]
            .headers
            .get("authorization")
            .map(|v| v.to_str().expect("ascii"));
        assert_eq!(
            same_origin,
            Some("Bearer t"),
            "same-origin hop keeps it ({forward})"
        );
        let on_b = b.received_requests().await.expect("recorded");
        let crossed = on_b[0]
            .headers
            .get("authorization")
            .map(|v| v.to_str().expect("ascii"));
        assert_eq!(crossed, expected, "cross-origin hop ({forward})");
    }
}

#[tokio::test]
async fn x_amz_headers_stay_on_the_origin_even_when_forwarding() {
    // bruno-cli drops x-amz-* on a cross-origin hop whatever forwardAuthorizationHeader says.
    let own = "  headers:\n    - name: X-Amz-Date\n      value: 20261009T000000Z\n    - name: x-amz-security-token\n      value: tok\n    - name: X-Other\n      value: kept\n";
    let settings = "settings:\n  forwardAuthorizationHeader: true\n";
    let (a, b) = (MockServer::start().await, MockServer::start().await);
    Mock::given(path("/start"))
        .respond_with(redirect(302, "/same"))
        .mount(&a)
        .await;
    Mock::given(path("/same"))
        .respond_with(redirect(302, &format!("{}/other", b.uri())))
        .mount(&a)
        .await;
    Mock::given(path("/other"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&b)
        .await;
    let req = resolved_one("GET", &format!("{}/start", a.uri()), own, settings);
    assert_eq!(executor().send(&req).await.expect("sent").status(), 200);
    let on_a = a.received_requests().await.expect("recorded");
    assert!(
        on_a[1].headers.contains_key("x-amz-date"),
        "same origin keeps it"
    );
    assert!(on_a[1].headers.contains_key("x-amz-security-token"));
    let on_b = b.received_requests().await.expect("recorded");
    assert!(
        !on_b[0].headers.contains_key("x-amz-date"),
        "{:?}",
        on_b[0].headers
    );
    assert!(!on_b[0].headers.contains_key("x-amz-security-token"));
    assert!(on_b[0].headers.contains_key("x-other"));
}

#[tokio::test]
async fn response_time_sums_every_hop() {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(redirect(302, "/slower").set_delay(Duration::from_millis(1000)))
        .mount(&server)
        .await;
    Mock::given(path("/slower"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(1000)))
        .mount(&server)
        .await;
    // The timeout bounds each hop, as axios's does, so 1600 ms admits two 1000 ms hops with
    // room for a loaded test run, while their 2000 ms sum would exceed one shared timeout.
    let settings = "settings:\n  timeout: 1600\n";
    let req = resolved_one("GET", &format!("{}/slow", server.uri()), "", settings);
    let res = executor().send(&req).await.expect("sent");
    assert_eq!(res.status(), 200);
    assert!(
        res.response_time() >= Duration::from_millis(2000),
        "{:?}",
        res.response_time()
    );
}
