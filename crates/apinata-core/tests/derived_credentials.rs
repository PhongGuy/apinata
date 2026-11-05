//! The executor lists what it derived per send, with or without a session log.

use crate::common;
use crate::derived_support::*;
use crate::session_log_support;

use apinata_core::{ExecOptions, Response, derived_secrets};
use common::{executor, resolved_one};
use session_log_support::{assert_absent, logged};
use wiremock::matchers::{any, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any()).respond_with(Challenge).mount(&s).await;
    s
}

async fn send(auth: &str) -> Response {
    let s = server().await;
    executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/v?a=1", s.uri()),
            auth,
            "",
        ))
        .await
        .expect("sent")
}

/// The derived list of `res`, after checking it is non-empty, unique, on the wire and free of
/// the user's own secrets.
fn checked(res: &Response) -> &[String] {
    let derived = res.derived_credentials();
    assert!(!derived.is_empty(), "nothing derived");
    for v in derived {
        assert!(!v.is_empty());
        assert!(on_wire(res, v), "{v} not sent:\n{}", sent_text(res));
        for secret in USER_SECRETS {
            assert!(!v.contains(secret), "{secret} in {v}");
        }
    }
    let mut sorted = derived.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), derived.len(), "{derived:?}");
    derived
}

#[test]
fn the_builder_dedupes_drops_empties_and_debug_hides_values() {
    let res = Response::new(200).with_derived_credentials(vec![
        "abcd-1".into(),
        String::new(),
        "abcd-1".into(),
        "efgh-2".into(),
    ]);
    assert_eq!(res.derived_credentials(), ["abcd-1", "efgh-2"]);
    assert!(!format!("{res:?}").contains("abcd-1"));
    assert!(Response::new(200).derived_credentials().is_empty());
}

#[tokio::test]
async fn oauth1_in_a_header_derives_its_signature() {
    let res = send(&oauth1("header")).await;
    assert_eq!(checked(&res).len(), 1);
}

#[tokio::test]
async fn oauth1_in_the_query_derives_its_signature() {
    let res = send(&oauth1("query")).await;
    let derived = checked(&res);
    assert_eq!(derived.len(), 1);
    assert!(res.timeline()[0].url.contains("oauth_signature="));
}

#[tokio::test]
async fn sigv4_derives_the_authorization_value_and_the_bare_signature() {
    let res = send(SIGV4).await;
    let derived = checked(&res);
    assert!(derived.iter().any(|v| v.starts_with("AWS4-HMAC-SHA256 ")));
    assert!(
        derived
            .iter()
            .any(|v| v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()))
    );
}

#[tokio::test]
async fn wsse_derives_its_whole_header_value_its_digest_and_its_nonce() {
    let res = send(WSSE).await;
    let derived = checked(&res);
    assert_eq!(derived.len(), 3);
    assert!(derived[0].starts_with("UsernameToken "));
    assert!(derived[1..].iter().all(|v| derived[0].contains(v.as_str())));
}

#[tokio::test]
async fn edgegrid_derives_its_authorization_value() {
    let res = send(EDGEGRID).await;
    assert!(checked(&res)[0].starts_with("EG1-HMAC-SHA256 "));
}

#[tokio::test]
async fn digest_derives_the_answer_sent_on_the_retried_hop() {
    let res = send(DIGEST).await;
    assert_eq!(res.timeline().len(), 2);
    let derived = checked(&res);
    assert_eq!(derived.len(), 1);
    assert!(derived[0].starts_with("Digest "));
}

#[tokio::test]
async fn a_send_without_signing_derives_nothing() {
    assert!(send("").await.derived_credentials().is_empty());
}

#[tokio::test]
async fn a_value_placed_before_a_redirect_is_still_listed() {
    let s = MockServer::start().await;
    Mock::given(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/end"))
        .mount(&s)
        .await;
    Mock::given(path("/end")).respond_with(Echo).mount(&s).await;
    let res = executor()
        .send(&resolved_one(
            "GET",
            &format!("{}/start", s.uri()),
            WSSE,
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.timeline().len(), 2);
    assert_eq!(checked(&res).len(), 3);
}

#[tokio::test]
async fn a_sigv4_signature_the_server_echoes_is_absent_from_the_session_log() {
    let s = server().await;
    let (exec, log) = logged(ExecOptions::new());
    let res = exec
        .send(&resolved_one("GET", &format!("{}/v", s.uri()), SIGV4, ""))
        .await
        .expect("sent");
    let signature = res
        .derived_credentials()
        .iter()
        .find(|v| v.len() == 64)
        .cloned()
        .expect("signature");
    assert!(res.text().contains(&signature), "the server echoes it");
    assert!(
        sent_text(&res).contains(&signature),
        "the timeline is verbatim"
    );
    assert_absent(&log, &signature);
}

#[test]
fn derived_secrets_are_labelled_deduped_longest_first_and_skip_short_values() {
    let found = derived_secrets(&[
        "abcd".to_owned(),
        "abcd-longer-value".to_owned(),
        "abcd".to_owned(),
        "xyz".to_owned(),
        String::new(),
    ]);
    let values: Vec<&str> = found.iter().map(|s| s.value()).collect();
    assert_eq!(values, ["abcd-longer-value", "abcd"]);
    assert!(found.iter().all(|s| s.label() == "derived credential"));
    assert!(found.iter().all(|s| s.placeholder() == "[redacted]"));
}
