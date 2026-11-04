//! The check: transport, privacy, caps, version and signature failures.
#![cfg(feature = "update")]

use crate::update_support;

use crate::common::dead_port;
use apinata_core::update::{UpdateError, check};
use std::time::Duration;
use update_support::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ART: &str = "https://example.com/apinata";

async fn served() -> (MockServer, TestKey) {
    (MockServer::start().await, TestKey::generate())
}

async fn mount(s: &MockServer, at: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(response)
        .mount(s)
        .await;
}

#[tokio::test]
async fn a_newer_signed_release_is_offered() {
    let (s, k) = served().await;
    publish(&s, &k, &manifest("2.0.0", ART, b"new")).await;
    let a = check(&config(&s.uri(), &k, "1.0.0"))
        .await
        .unwrap()
        .expect("offered");
    assert_eq!(
        (a.version.as_str(), a.size, a.url.as_str()),
        ("2.0.0", 3, ART)
    );
}

#[tokio::test]
async fn the_check_sends_only_the_private_header_set() {
    let (s, k) = served().await;
    publish(&s, &k, &manifest("2.0.0", ART, b"new")).await;
    check(&config(&s.uri(), &k, "1.0.0")).await.unwrap();
    let requests = s.received_requests().await.unwrap();
    let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
    assert_eq!(paths, ["/latest.json", "/latest.json.sig"]);
    for r in &requests {
        let mut names: Vec<&str> = r.headers.keys().map(|n| n.as_str()).collect();
        names.sort();
        assert_eq!(names, ["accept", "accept-encoding", "host", "user-agent"]);
        let ua = format!("apinata/{}", env!("CARGO_PKG_VERSION"));
        assert_eq!(r.headers["user-agent"], ua.as_str());
        assert_eq!(r.headers["accept-encoding"], "identity");
    }
}

#[tokio::test]
async fn a_replayed_or_equal_release_offers_nothing() {
    for old in ["1.0.0", "0.9.0"] {
        let (s, k) = served().await;
        publish(&s, &k, &manifest(old, ART, b"x")).await;
        assert_eq!(
            check(&config(&s.uri(), &k, "1.0.0"))
                .await
                .unwrap()
                .map(|a| a.version),
            None,
            "{old}"
        );
    }
}

#[tokio::test]
async fn a_manifest_edited_after_signing_is_refused() {
    let (s, k) = served().await;
    let signed = manifest("2.0.0", ART, b"new");
    mount(
        &s,
        "/latest.json",
        ResponseTemplate::new(200).set_body_string(signed.replace("2.0.0", "3.0.0")),
    )
    .await;
    mount(
        &s,
        "/latest.json.sig",
        ResponseTemplate::new(200).set_body_string(k.sign(signed.as_bytes())),
    )
    .await;
    let got = check(&config(&s.uri(), &k, "1.0.0")).await;
    assert!(
        matches!(got, Err(UpdateError::ManifestSignature(_))),
        "{got:?}"
    );
}

#[tokio::test]
async fn a_badly_signed_body_is_never_parsed() {
    let (s, k) = served().await;
    mount(
        &s,
        "/latest.json",
        ResponseTemplate::new(200).set_body_string("this is not json"),
    )
    .await;
    mount(
        &s,
        "/latest.json.sig",
        ResponseTemplate::new(200).set_body_string(k.sign(b"something else")),
    )
    .await;
    let got = check(&config(&s.uri(), &k, "1.0.0")).await;
    assert!(
        matches!(got, Err(UpdateError::ManifestSignature(_))),
        "a parse error would mean parse came first: {got:?}"
    );
}

#[tokio::test]
async fn a_missing_signature_is_a_signature_error() {
    let (s, k) = served().await;
    mount(
        &s,
        "/latest.json",
        ResponseTemplate::new(200).set_body_string(manifest("2.0.0", ART, b"x")),
    )
    .await;
    mount(&s, "/latest.json.sig", ResponseTemplate::new(404)).await;
    let got = check(&config(&s.uri(), &k, "1.0.0")).await;
    assert!(
        matches!(got, Err(UpdateError::ManifestSignature(_))),
        "{got:?}"
    );
}

#[tokio::test]
async fn a_release_signed_by_another_key_is_unknown() {
    let (s, signer) = served().await;
    publish(&s, &signer, &manifest("2.0.0", ART, b"x")).await;
    let trusted = TestKey::generate();
    assert_eq!(
        check(&config(&s.uri(), &trusted, "1.0.0"))
            .await
            .unwrap_err(),
        UpdateError::UnknownKey
    );
}

#[tokio::test]
async fn a_signed_pre_release_is_invalid() {
    let (s, k) = served().await;
    publish(&s, &k, &manifest("2.0.0-rc.1", ART, b"x")).await;
    let got = check(&config(&s.uri(), &k, "1.0.0")).await;
    assert!(
        matches!(got, Err(UpdateError::ManifestInvalid(_))),
        "{got:?}"
    );
}

#[tokio::test]
async fn a_server_error_is_http() {
    let (s, k) = served().await;
    mount(&s, "/latest.json", ResponseTemplate::new(500)).await;
    assert_eq!(
        check(&config(&s.uri(), &k, "1.0.0")).await.unwrap_err(),
        UpdateError::Http(500)
    );
}

#[tokio::test]
async fn oversized_bodies_are_refused() {
    let (s, k) = served().await;
    mount(
        &s,
        "/latest.json",
        ResponseTemplate::new(200).set_body_bytes(vec![b' '; 64 * 1024 + 1]),
    )
    .await;
    let got = check(&config(&s.uri(), &k, "1.0.0")).await;
    assert!(
        matches!(got, Err(UpdateError::ManifestInvalid(_))),
        "manifest: {got:?}"
    );

    let (s, k) = served().await;
    mount(
        &s,
        "/latest.json",
        ResponseTemplate::new(200).set_body_string(manifest("2.0.0", ART, b"x")),
    )
    .await;
    mount(
        &s,
        "/latest.json.sig",
        ResponseTemplate::new(200).set_body_bytes(vec![b'A'; 4097]),
    )
    .await;
    let got = check(&config(&s.uri(), &k, "1.0.0")).await;
    assert!(
        matches!(got, Err(UpdateError::ManifestInvalid(_))),
        "signature: {got:?}"
    );
}

#[tokio::test]
async fn an_unreachable_server_is_offline() {
    let port = dead_port();
    let got = check(&config(
        &format!("http://127.0.0.1:{port}"),
        &TestKey::generate(),
        "1.0.0",
    ))
    .await;
    assert!(matches!(got, Err(UpdateError::Offline(_))), "{got:?}");
}

#[tokio::test]
async fn a_slow_manifest_times_out() {
    let (s, k) = served().await;
    let slow = ResponseTemplate::new(200)
        .set_body_string("{}")
        .set_delay(Duration::from_secs(5));
    mount(&s, "/latest.json", slow).await;
    let mut cfg = config(&s.uri(), &k, "1.0.0");
    cfg.limits.manifest = Duration::from_millis(200);
    assert_eq!(check(&cfg).await.unwrap_err(), UpdateError::Timeout);
}

#[tokio::test]
async fn redirects_are_followed_within_the_rules() {
    let (s, k) = served().await;
    publish(&s, &k, &manifest("2.0.0", ART, b"x")).await;
    let to_latest =
        ResponseTemplate::new(302).insert_header("location", format!("{}/latest.json", s.uri()));
    mount(&s, "/moved/latest.json", to_latest).await;
    mount(
        &s,
        "/moved/latest.json.sig",
        ResponseTemplate::new(302)
            .insert_header("location", format!("{}/latest.json.sig", s.uri())),
    )
    .await;
    let mut cfg = config(&s.uri(), &k, "1.0.0");
    cfg.manifest_url = format!("{}/moved/latest.json", s.uri());
    assert!(check(&cfg).await.unwrap().is_some());

    // A hop to plain http off loopback is refused before any connection.
    mount(
        &s,
        "/insecure/latest.json",
        ResponseTemplate::new(302).insert_header("location", "http://example.invalid/latest.json"),
    )
    .await;
    cfg.manifest_url = format!("{}/insecure/latest.json", s.uri());
    assert!(matches!(check(&cfg).await, Err(UpdateError::Offline(_))));

    // Six hops is one too many.
    for i in 0..6 {
        let next = format!("{}/hop{}/latest.json", s.uri(), i + 1);
        mount(
            &s,
            &format!("/hop{i}/latest.json"),
            ResponseTemplate::new(302).insert_header("location", next),
        )
        .await;
    }
    cfg.manifest_url = format!("{}/hop0/latest.json", s.uri());
    assert!(matches!(check(&cfg).await, Err(UpdateError::Offline(_))));
}

#[tokio::test]
async fn a_redirected_hop_sends_no_referer() {
    let (s, k) = served().await;
    publish(&s, &k, &manifest("2.0.0", ART, b"x")).await;
    let to_latest =
        ResponseTemplate::new(302).insert_header("location", format!("{}/latest.json", s.uri()));
    mount(&s, "/moved/latest.json", to_latest).await;
    let mut cfg = config(&s.uri(), &k, "1.0.0");
    cfg.manifest_url = format!("{}/moved/latest.json", s.uri());
    let _ = check(&cfg).await;
    let requests = s.received_requests().await.unwrap();
    assert!(
        requests.iter().any(|r| r.url.path() == "/latest.json"),
        "the hop was followed"
    );
    for r in &requests {
        let mut names: Vec<&str> = r.headers.keys().map(|n| n.as_str()).collect();
        names.sort();
        assert_eq!(
            names,
            ["accept", "accept-encoding", "host", "user-agent"],
            "{}",
            r.url
        );
    }
}
