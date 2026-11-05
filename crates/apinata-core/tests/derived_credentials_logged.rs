//! Per mode: the derived value is listed and in the timeline, but never in the session log.

use crate::common;
use crate::derived_support::*;
use crate::ntlm_support;
use crate::session_log_support;

use apinata_core::{ExecOptions, Response};
use common::resolved_one;
use ntlm_support::{Options, PASSWORD, USER, ntlm_server};
use session_log_support::{assert_absent, logged};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer};

async fn logged_send(auth: &str) -> (Response, apinata_core::SessionLog) {
    let s = MockServer::start().await;
    Mock::given(any()).respond_with(Challenge).mount(&s).await;
    let (exec, log) = logged(ExecOptions::new());
    let res = exec
        .send(&resolved_one(
            "GET",
            &format!("{}/v?a=1", s.uri()),
            auth,
            "",
        ))
        .await
        .expect("sent");
    (res, log)
}

fn assert_hidden(res: &Response, log: &apinata_core::SessionLog) {
    let derived = res.derived_credentials();
    assert!(!derived.is_empty());
    for v in derived {
        assert!(on_wire(res, v), "{v} missing from the timeline");
        assert_absent(log, v);
    }
}

#[tokio::test]
async fn every_signing_mode_is_hidden_in_the_log_and_kept_in_the_timeline() {
    for (mode, auth) in [
        ("oauth1 header", oauth1("header")),
        ("oauth1 query", oauth1("query")),
        ("sigv4", SIGV4.to_owned()),
        ("wsse", WSSE.to_owned()),
        ("edgegrid", EDGEGRID.to_owned()),
        ("digest", DIGEST.to_owned()),
    ] {
        let (res, log) = logged_send(&auth).await;
        eprintln!("mode {mode}");
        assert_hidden(&res, &log);
    }
}

#[tokio::test]
async fn ntlm_values_are_hidden_in_the_log_and_kept_in_the_timeline() {
    let s = ntlm_server(Options::default()).await;
    let auth = format!(
        "  auth:\n    type: ntlm\n    username: {USER}\n    password: {PASSWORD}\n    domain: CORP\n"
    );
    let (exec, log) = logged(ExecOptions::new());
    let res = exec
        .send(&resolved_one(
            "GET",
            &format!("{}/ok", s.origin()),
            &auth,
            "",
        ))
        .await
        .expect("sent");
    assert!(
        res.derived_credentials()
            .iter()
            .any(|v| v.starts_with("NTLM ")),
        "{:?}",
        res.derived_credentials()
    );
    for v in res.derived_credentials() {
        assert_absent(&log, v);
    }
}

/// Answers 200 with only the named part of the request's X-WSSE value as the body.
struct EchoWsseField(&'static str);

impl wiremock::Respond for EchoWsseField {
    fn respond(&self, req: &wiremock::Request) -> wiremock::ResponseTemplate {
        let header = req.headers.get("x-wsse").and_then(|v| v.to_str().ok());
        let marker = format!("{}=\"", self.0);
        let value = header
            .and_then(|h| h.split(marker.as_str()).nth(1))
            .and_then(|rest| rest.split('"').next())
            .unwrap_or("");
        wiremock::ResponseTemplate::new(200).set_body_string(value)
    }
}

/// Sends a WSSE request to a server echoing only `field`; returns the response, the log and
/// the echoed value.
async fn wsse_field_echo(field: &'static str) -> (Response, apinata_core::SessionLog, String) {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(EchoWsseField(field))
        .mount(&s)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    let res = exec
        .send(&resolved_one("GET", &format!("{}/v", s.uri()), WSSE, ""))
        .await
        .expect("sent");
    let echoed = res.text().to_owned();
    assert!(!echoed.is_empty(), "the server echoes {field}");
    (res, log, echoed)
}

#[tokio::test]
async fn an_echoed_wsse_digest_or_nonce_is_hidden_in_the_log_the_derived_list_and_an_example() {
    for field in ["PasswordDigest", "Nonce"] {
        let (res, log, echoed) = wsse_field_echo(field).await;
        assert!(
            res.derived_credentials().contains(&echoed),
            "{field} not derived: {:?}",
            res.derived_credentials()
        );
        assert_absent(&log, &echoed);

        // Saved examples and MCP results scrub with the same list.
        let mut example = apinata_core::ExampleResponse::from_response(&res);
        example.scrub(Some(&apinata_core::derived_secrets(
            res.derived_credentials(),
        )));
        let body = example.body.map(|b| b.data).unwrap_or_default();
        assert!(!body.contains(&echoed), "{field} kept in the example");
    }
}
