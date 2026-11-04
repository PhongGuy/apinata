use apinata_app::compare_source::*;
use apinata_app::last_response::{KeptSend, Stored};
use apinata_core::{
    BodyState, ExampleBody, ExampleBodyKind, ExampleResponse, Outcome, RequestResult, Response,
};
use std::time::{Duration, SystemTime};

use crate::support;
use support::ip;

fn kept(seq: u64) -> KeptSend {
    KeptSend {
        seq,
        saved_at: SystemTime::UNIX_EPOCH,
        status: 200,
        time: Duration::ZERO,
        size: 0,
        method: String::new(),
        url: String::new(),
    }
}
fn history(seqs: &[u64]) -> Vec<KeptSend> {
    seqs.iter().map(|s| kept(*s)).collect()
}

#[test]
fn previous_of_the_latest_is_the_second_newest() {
    assert_eq!(previous_seq(&history(&[3, 2, 1, 0]), None), Some(2));
}
#[test]
fn previous_of_a_kept_send_is_the_next_older() {
    let h = history(&[3, 2, 1, 0]);
    assert_eq!(previous_seq(&h, Some(2)), Some(1));
    assert_eq!(previous_seq(&h, Some(1)), Some(0));
}
#[test]
fn the_oldest_send_has_no_previous() {
    assert_eq!(previous_seq(&history(&[3, 2, 1, 0]), Some(0)), None);
}
#[test]
fn one_send_or_none_has_no_previous() {
    assert_eq!(previous_seq(&history(&[0]), None), None);
    assert_eq!(previous_seq(&[], None), None);
}
#[test]
fn an_unknown_viewed_seq_has_no_previous() {
    assert_eq!(previous_seq(&history(&[3, 2]), Some(9)), None);
}

#[test]
fn choices_list_previous_then_examples_with_reasons() {
    let ex = [
        ExampleRef {
            name: "ok".into(),
            has_response: true,
        },
        ExampleRef {
            name: "bare".into(),
            has_response: false,
        },
    ];
    let c = choices(&history(&[1]), None, &ex);
    let got: Vec<_> = c.iter().map(|c| (c.label.as_str(), c.disabled)).collect();
    assert_eq!(
        got,
        [
            ("Previous send", Some("No earlier send")),
            ("ok", None),
            ("bare", Some("This example has no response"))
        ]
    );
    assert_eq!(c[1].source, CompareSource::Example(0));
    assert_eq!(choices(&history(&[1, 0]), None, &[])[0].disabled, None);
}

#[test]
fn the_live_snapshot_scrubs_secrets_from_headers_and_body() {
    let r = Response::new(200)
        .with_status_text("OK")
        .with_header("x-token", "s3cr3t")
        .with_text_body(r#"{"k":"s3cr3t"}"#);
    let s = snapshot_of_response(&r, &["s3cr3t".to_owned()]);
    assert_eq!(s.headers()[0].1, "[redacted]");
    assert_eq!(s.body(), Some(r#"{"k":"[redacted]"}"#));
    assert_eq!(
        (s.status(), s.status_text(), s.body_state()),
        (200, "OK", BodyState::Text)
    );
}
#[test]
fn a_truncated_response_is_truncated() {
    let r = Response::new(200).with_text_body("abc").with_truncated(10);
    assert_eq!(
        snapshot_of_response(&r, &[]).body_state(),
        BodyState::Truncated
    );
}
#[test]
fn invalid_utf8_without_a_charset_is_binary() {
    let r = Response::new(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(vec![0xff, 0xfe], "");
    assert_eq!(
        snapshot_of_response(&r, &[]).body_state(),
        BodyState::Binary
    );
}
#[test]
fn an_empty_body_is_empty_text() {
    let s = snapshot_of_response(&Response::new(204), &[]);
    assert_eq!((s.body(), s.body_state()), (Some(""), BodyState::Text));
}
#[test]
fn a_stored_send_whose_body_was_dropped_is_dropped() {
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200)),
        Vec::new(),
    );
    let stored = Stored {
        result,
        saved_at: SystemTime::UNIX_EPOCH,
        body_dropped: true,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    assert_eq!(snapshot_of_stored(&stored).body_state(), BodyState::Dropped);
}
#[test]
fn an_example_becomes_a_snapshot() {
    let mut e = ExampleResponse::new(404);
    e.status_text = Some("Not Found".into());
    e.headers
        .push(("content-type".into(), "application/json".into()));
    e.body = Some(ExampleBody::new(ExampleBodyKind::Json, r#"{"a":1}"#));
    let s = snapshot_of_example(&e);
    assert_eq!(
        (s.status(), s.status_text(), s.body(), s.body_state()),
        (404, "Not Found", Some(r#"{"a":1}"#), BodyState::Text)
    );
    e.body = Some(ExampleBody::new(ExampleBodyKind::Binary, "AAEC"));
    assert_eq!(snapshot_of_example(&e).body_state(), BodyState::Binary);
}

fn url_header_response() -> Response {
    Response::new(302)
        .with_header("location", "http://joe:pa55-xyz@127.0.0.1:9/next")
        .with_header("set-cookie", "sid=cookie-val-xyz; Path=/")
        .with_header("authorization", "Bearer tok-xyz")
}

fn assert_masked(s: &apinata_core::ResponseSnapshot) {
    for (name, value) in s.headers() {
        for raw in ["pa55-xyz", "cookie-val-xyz", "tok-xyz"] {
            assert!(!value.contains(raw), "{name}: {value}");
        }
    }
    assert_eq!(s.headers()[0].1, "http://127.0.0.1:9/next");
    assert_eq!(s.headers()[1].1, "sid=[redacted]");
}

#[test]
fn both_compare_sides_mask_response_headers_as_the_cli_does() {
    assert_masked(&snapshot_of_response(&url_header_response(), &[]));
    let stored = Stored {
        result: RequestResult::from_parts(
            ip("a.yml"),
            "a",
            Outcome::Passed,
            Some(url_header_response()),
            Vec::new(),
        ),
        saved_at: SystemTime::UNIX_EPOCH,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    assert_masked(&snapshot_of_stored(&stored));
}
