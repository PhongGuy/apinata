//! A tab's send model showing an older kept send, and going back to the latest.

use crate::support;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::last_response::Stored;
use apinata_app::send_model::{Restored, SendModel, SendState, forget_all_restored};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::{Entity, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

fn at(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn kept(status: u16) -> RequestResult {
    let response = Response::new(status).with_text_body("k");
    RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    )
}

fn stored(status: u16, ms: u64) -> Stored {
    Stored {
        result: kept(status),
        saved_at: at(ms),
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    }
}

fn restored(ms: u64) -> Restored {
    Restored {
        saved_at: at(ms),
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    }
}

fn shown(cx: &mut TestAppContext, model: &Entity<SendModel>) -> Option<u16> {
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.response.as_ref().map(|r| r.status()),
        _ => None,
    })
}

#[gpui_kit::test]
fn an_older_send_shows_until_back_to_latest(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.restore(Arc::new(kept(203)), restored(3), cx));
    model.update(cx, |m, cx| m.view_kept(0, stored(201, 1), cx));
    assert_eq!(shown(cx, &model), Some(201));
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), Some(0));
    assert_eq!(model.read_with(cx, |m, _| m.restored()), Some(restored(1)));
    model.update(cx, |m, cx| m.view_kept(1, stored(202, 2), cx));
    assert_eq!(shown(cx, &model), Some(202));
    model.update(cx, |m, cx| m.back_to_latest(cx));
    assert_eq!(shown(cx, &model), Some(203));
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), None);
    assert_eq!(model.read_with(cx, |m, _| m.restored()), Some(restored(3)));
    model.update(cx, |m, cx| m.back_to_latest(cx));
    assert_eq!(shown(cx, &model), Some(203), "a second back is a no-op");
}

#[gpui_kit::test]
fn a_restore_landing_after_a_history_pick_becomes_the_latest(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.view_kept(0, stored(201, 1), cx));
    model.update(cx, |m, cx| m.restore(Arc::new(kept(203)), restored(3), cx));
    assert_eq!(shown(cx, &model), Some(201), "the pick stays on screen");
    model.update(cx, |m, cx| m.back_to_latest(cx));
    assert_eq!(shown(cx, &model), Some(203));
    assert_eq!(model.read_with(cx, |m, _| m.restored()), Some(restored(3)));
}

#[gpui_kit::test]
fn a_send_leaves_the_history_view_and_is_not_ignored(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("live"));
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.view_kept(0, stored(201, 1), cx));
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), None);
    model.update(cx, |m, cx| m.view_kept(0, stored(201, 1), cx));
    assert!(
        model.read_with(cx, |m, _| m.is_sending()),
        "ignored while sending"
    );
    let result = finished(cx, &model);
    assert_eq!(result.response.as_ref().unwrap().text(), "live");
    model.update(cx, |m, cx| m.back_to_latest(cx));
    assert_eq!(shown(cx, &model), Some(200));
}

#[gpui_kit::test]
fn clearing_drops_a_history_view(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.view_kept(0, stored(201, 1), cx));
    cx.update(forget_all_restored);
    assert!(model.read_with(cx, |m, _| matches!(m.state(), SendState::Idle)));
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), None);
    model.update(cx, |m, cx| m.back_to_latest(cx));
    assert!(model.read_with(cx, |m, _| matches!(m.state(), SendState::Idle)));
}

#[gpui_kit::test]
fn a_live_result_of_a_followed_run_ends_a_history_view(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let (tx, rx) = async_channel::unbounded();
    model.update(cx, |m, cx| m.follow(rx, cx));
    let finished = |index, status| apinata_core::RunEvent::RequestFinished {
        index,
        result: Arc::new(kept(status)),
    };
    tx.try_send(finished(0, 200)).unwrap();
    cx.run_until_parked();
    model.update(cx, |m, cx| m.view_kept(0, stored(201, 1), cx));
    assert_eq!(shown(cx, &model), Some(201), "a pick between results shows");
    tx.try_send(finished(1, 202)).unwrap();
    cx.run_until_parked();
    assert_eq!(shown(cx, &model), Some(202));
    assert_eq!(model.read_with(cx, |m, _| m.viewing()), None);
    assert_eq!(model.read_with(cx, |m, _| m.restored()), None);
    model.update(cx, |m, cx| m.back_to_latest(cx));
    assert_eq!(shown(cx, &model), Some(202), "no stale stash to go back to");
}
