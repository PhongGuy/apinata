use apinata_app::last_response::{BODY_LIMIT, ResponseStore, STREAM_BODY_LIMIT};
use apinata_core::{ItemPath, Outcome, RequestResult, Response, StreamEnd};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn ip(s: &str) -> ItemPath {
    ItemPath::new(s).unwrap()
}

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn result(response: Response) -> RequestResult {
    RequestResult::from_parts(ip("a.yml"), "A", Outcome::Passed, Some(response), vec![])
}

fn stored(response: Response) -> apinata_app::last_response::Stored {
    let data = tempfile::tempdir().unwrap();
    let store = ResponseStore::new(data.path().join("r"));
    store
        .save(Path::new("/c"), &ip("a.yml"), &result(response), at(1))
        .unwrap();
    store.load(Path::new("/c"), &ip("a.yml")).unwrap()
}

#[test]
fn a_big_stream_keeps_a_one_mib_head_marked_truncated() {
    let body = "data: x\n\n".repeat(400_000); // about 3.6 MiB, under the 5 MiB drop limit
    assert!(body.len() > STREAM_BODY_LIMIT && body.len() < BODY_LIMIT);
    let got = stored(
        Response::new(200)
            .with_text_body(body.clone())
            .with_stream_end(StreamEnd::Stopped),
    );
    assert!(!got.body_dropped);
    let r = got.result.response.unwrap();
    assert!(r.truncated());
    assert!(r.body().len() <= STREAM_BODY_LIMIT && r.body().len() > STREAM_BODY_LIMIT - 4);
    assert_eq!(r.body(), &body.as_bytes()[..r.body().len()]);
    assert_eq!(r.size(), body.len() as u64, "the original size is kept");
}

#[test]
fn a_stream_over_the_old_drop_limit_is_still_kept_as_a_head() {
    let body = "y".repeat(BODY_LIMIT + 10);
    let got = stored(
        Response::new(200)
            .with_text_body(body)
            .with_stream_end(StreamEnd::Closed),
    );
    assert!(!got.body_dropped);
    assert!(got.result.response.unwrap().truncated());
}

#[test]
fn a_small_stream_is_kept_whole() {
    let got = stored(
        Response::new(200)
            .with_text_body("data: a\n\n")
            .with_stream_end(StreamEnd::Closed),
    );
    let r = got.result.response.unwrap();
    assert!(!r.truncated() && !got.body_dropped);
    assert_eq!(r.body(), b"data: a\n\n");
}

#[test]
fn an_ordinary_body_between_one_and_five_mib_is_kept_whole() {
    let body = "z".repeat(STREAM_BODY_LIMIT + 1000);
    let got = stored(Response::new(200).with_text_body(body.clone()));
    let r = got.result.response.unwrap();
    assert!(!r.truncated() && !got.body_dropped);
    assert_eq!(r.body().len(), body.len());
}

#[test]
fn an_ordinary_body_over_five_mib_is_still_dropped() {
    let got = stored(Response::new(200).with_text_body("w".repeat(BODY_LIMIT + 1)));
    assert!(got.body_dropped);
}

#[test]
fn the_cut_never_splits_a_multibyte_char() {
    // Three-byte chars never line up with the 1 MiB mark (1 048 576 = 3 * 349 525 + 1).
    let body = "\u{20ac}".repeat(400_000);
    let got = stored(
        Response::new(200)
            .with_text_body(body)
            .with_stream_end(StreamEnd::Stopped),
    );
    let r = got.result.response.unwrap();
    assert!(std::str::from_utf8(r.body()).is_ok());
    assert!(!r.text().contains('\u{fffd}'));
}
