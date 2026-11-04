//! The NTLM handshake on the wire against a 127.0.0.1 NTLM origin.

use crate::common;
use crate::ntlm_support;

use apinata_core::{
    CookieJar, ExecError, ExecOptions, Executor, NtlmCause, ResolvedRequest, SessionLog,
};
use common::{executor, resolved_one};
use ntlm_support::{Options, PASSWORD, Stage, USER, ntlm_server};
use std::sync::Arc;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const JSON: &str = "  body:\n    type: json\n    data: '{\"a\":1}'\n";

fn auth(password: &str) -> String {
    format!(
        "  auth:\n    type: ntlm\n    username: {USER}\n    password: {password}\n    domain: CORP\n"
    )
}

fn req(method: &str, url: &str, under: &str) -> ResolvedRequest {
    resolved_one(method, url, under, "")
}

#[tokio::test]
async fn the_handshake_completes_on_one_connection() {
    let s = ntlm_server(Options::default()).await;
    let res = executor()
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(PASSWORD)))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "GET 0"));
    let seen = s.seen();
    let stages: Vec<Stage> = seen.iter().map(|r| r.stage()).collect();
    assert_eq!(stages, [Stage::Probe, Stage::Type1, Stage::Type3]);
    assert!(seen.iter().all(|r| r.conn == seen[0].conn));
    assert_eq!(s.connections(), 1);
}

#[tokio::test]
async fn the_body_is_resent_on_every_leg() {
    let s = ntlm_server(Options::default()).await;
    let under = format!("{}{JSON}", auth(PASSWORD));
    let res = executor()
        .send(&req("POST", &format!("{}/ok", s.origin()), &under))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "POST 7"));
    let seen = s.seen();
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter()
            .all(|r| r.method == "POST" && r.body == b"{\"a\":1}"),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_wrong_password_returns_the_final_401() {
    let s = ntlm_server(Options::default()).await;
    let res = executor()
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth("nope")))
        .await
        .expect("a 401 is a response");
    assert_eq!(res.status(), 401);
    assert_eq!(s.seen().len(), 3);
    assert_eq!(res.timeline().len(), 3);
}

#[tokio::test]
async fn a_server_that_offers_only_basic_gets_the_probe_back() {
    let s = ntlm_server(Options::default()).await;
    let res = executor()
        .send(&req(
            "GET",
            &format!("{}/basic", s.origin()),
            &auth(PASSWORD),
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 401);
    assert_eq!(s.seen().len(), 1);
}

#[tokio::test]
async fn a_server_that_never_challenges_sees_one_request() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string("open"))
        .mount(&server)
        .await;
    let res = executor()
        .send(&req("GET", &format!("{}/x", server.uri()), &auth(PASSWORD)))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "open"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn connection_close_on_the_challenge_is_a_typed_error_without_credentials() {
    let s = ntlm_server(Options {
        close_on_401: true,
        ..Options::default()
    })
    .await;
    let err = executor()
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(PASSWORD)))
        .await
        .expect_err("cannot pin");
    assert!(
        matches!(
            err,
            ExecError::Ntlm {
                cause: NtlmCause::ConnectionNotKept,
                ..
            }
        ),
        "{err:?}"
    );
    for shown in [err.to_string(), format!("{err:?}")] {
        assert!(
            !shown.contains(PASSWORD) && !shown.contains("NTLM T"),
            "{shown}"
        );
    }
}

#[tokio::test]
async fn an_unparseable_type2_is_a_typed_error() {
    let s = ntlm_server(Options {
        junk_challenge: true,
        ..Options::default()
    })
    .await;
    let err = executor()
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(PASSWORD)))
        .await
        .expect_err("junk");
    assert!(
        matches!(
            err,
            ExecError::Ntlm {
                cause: NtlmCause::ChallengeUnusable,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn challenge_bodies_are_drained_past_the_body_cap_but_not_past_one_mib() {
    let big = ntlm_server(Options {
        challenge_body: 1_000_000,
        ..Options::default()
    })
    .await;
    let exec = Executor::new(ExecOptions::new().max_body_bytes(Some(3))).unwrap();
    let res = exec
        .send(&req(
            "GET",
            &format!("{}/ok", big.origin()),
            &auth(PASSWORD),
        ))
        .await
        .expect("sent");
    // The final response obeys the cap; the 1 MB challenge bodies did not.
    assert_eq!(
        (res.status(), res.text(), res.truncated()),
        (200, "GET", true)
    );
    let huge = ntlm_server(Options {
        challenge_body: 1_048_577,
        ..Options::default()
    })
    .await;
    let err = executor()
        .send(&req(
            "GET",
            &format!("{}/ok", huge.origin()),
            &auth(PASSWORD),
        ))
        .await
        .expect_err("over the cap");
    assert!(
        matches!(
            err,
            ExecError::Ntlm {
                cause: NtlmCause::ChallengeTooLarge,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn cookies_set_by_the_challenge_legs_reach_the_jar() {
    let s = ntlm_server(Options {
        cookie: true,
        ..Options::default()
    })
    .await;
    let jar = Arc::new(CookieJar::default());
    let exec = Executor::new(ExecOptions::new().cookie_jar(jar.clone())).unwrap();
    exec.send(&req("GET", &format!("{}/ok", s.origin()), &auth(PASSWORD)))
        .await
        .expect("sent");
    assert_eq!(jar.list().len(), 1);
}

#[tokio::test]
async fn each_challenge_leg_records_the_cookie_it_set() {
    let s = ntlm_server(Options {
        cookie: true,
        ..Options::default()
    })
    .await;
    let res = executor()
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(PASSWORD)))
        .await
        .expect("sent");
    let legs: Vec<Vec<(&str, apinata_core::CookieOutcome)>> = (res.timeline().iter())
        .map(|h| {
            h.cookies
                .iter()
                .map(|c| (c.name.as_str(), c.outcome))
                .collect()
        })
        .collect();
    let stored = apinata_core::CookieOutcome::Stored;
    assert_eq!(legs, [vec![("sid", stored)], vec![("sid", stored)], vec![]]);
}

#[tokio::test]
async fn the_final_leg_carries_the_type_3_and_the_log_hides_the_blobs() {
    let s = ntlm_server(Options::default()).await;
    let log = SessionLog::new();
    let exec = Executor::new(ExecOptions::new().session_log(log.clone())).unwrap();
    let res = exec
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(PASSWORD)))
        .await
        .expect("sent");
    let hop = res.timeline().last().expect("the final leg");
    let sent = hop
        .request_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))
        .expect("authorization");
    assert!(sent.1.starts_with("NTLM "), "{sent:?}");
    let type3 = s.seen().last().unwrap().auth.clone().unwrap();
    let type1 = s.seen()[1].auth.clone().unwrap();
    let entry = log.network_since(0).entries.remove(0);
    let shown = format!("{entry:?}");
    for secret in [PASSWORD, type3.as_str(), type1.as_str()] {
        assert!(!shown.contains(secret), "{secret} leaked: {shown}");
    }
}

#[tokio::test]
async fn the_timeline_lists_every_handshake_leg_and_masks_each() {
    let s = ntlm_server(Options::default()).await;
    let log = SessionLog::new();
    let exec = Executor::new(ExecOptions::new().session_log(log.clone())).unwrap();
    let res = exec
        .send(&req("GET", &format!("{}/ok", s.origin()), &auth(PASSWORD)))
        .await
        .expect("sent");
    let line = res.timeline();
    assert_eq!(line.len(), 3, "{line:?}");
    let statuses: Vec<u16> = line.iter().map(|h| h.status).collect();
    assert_eq!(statuses, [401, 401, 200]);
    let authorization = |i: usize| {
        line[i]
            .request_headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.clone())
    };
    let seen = s.seen();
    assert_eq!(authorization(0), None);
    assert_eq!(authorization(1), seen[1].auth);
    assert_eq!(authorization(2), seen[2].auth);
    let challenge = line[1]
        .response_headers
        .iter()
        .find(|(n, _)| n == "www-authenticate")
        .map(|(_, v)| v.clone())
        .expect("the Type 2 challenge");
    // Every blob a consumer must mask is in the response's derived list.
    for blob in [
        authorization(1).unwrap(),
        authorization(2).unwrap(),
        challenge.clone(),
    ] {
        assert!(res.derived_credentials().contains(&blob), "{blob}");
    }
    // Nothing raw reaches the Debug output of any leg or of the log entry.
    let entry = log.network_since(0).entries.remove(0);
    let shown = format!("{line:?} {entry:?}");
    for secret in [
        PASSWORD,
        seen[1].auth.as_deref().unwrap(),
        seen[2].auth.as_deref().unwrap(),
        challenge.as_str(),
    ] {
        assert!(!shown.contains(secret), "{secret} leaked: {shown}");
    }
}
