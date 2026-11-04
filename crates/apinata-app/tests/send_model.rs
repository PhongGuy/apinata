use crate::support;

use apinata_app::send_model::SendState;
use apinata_app::settings::Settings;
use apinata_app::{executor, workspace::Workspace};
use apinata_core::{Executor, Outcome};
use gpui_kit::{Entity, TestAppContext};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::*;
use wiremock::ResponseTemplate;

fn state_name(cx: &mut TestAppContext, m: &Entity<apinata_app::send_model::SendModel>) -> String {
    m.read_with(cx, |m, _| match m.state() {
        SendState::Idle => "idle".into(),
        SendState::Sending => "sending".into(),
        SendState::Streaming => "streaming".into(),
        SendState::Cancelled => "cancelled".into(),
        SendState::Failed(msg) => format!("failed: {msg}"),
        SendState::Done(r) => format!("done: {:?}", r.outcome),
    })
}

fn slow(
    cx: &mut TestAppContext,
) -> (
    wiremock::MockServer,
    tempfile::TempDir,
    tempfile::TempDir,
    Entity<Workspace>,
    std::path::PathBuf,
) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
    );
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    (server, col, state, ws, root)
}

#[gpui_kit::test]
fn a_send_shows_the_response(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(201).set_body_raw("{\"a\":1}", "application/json"),
    );
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    let res = result.response.as_ref().expect("a response");
    assert_eq!((res.status(), result.outcome), (201, Outcome::Passed));
    assert_eq!(served(cx, &server), ["/ok"]);
}

#[gpui_kit::test]
fn the_current_draft_is_sent(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    // The edit and the plan go out back to back; the actor applies them in order.
    let draft = format!("{}/draft", server.uri());
    ws.update(cx, |w, cx| w.send(&root, edit_url("a.yml", &draft), cx));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    assert_eq!(served(cx, &server), ["/draft"]);
}

#[gpui_kit::test]
fn a_second_send_is_refused_and_cancel_ends_in_flight(cx: &mut TestAppContext) {
    let (server, _col, _state, ws, root) = slow(cx);
    let model = sender(cx, &ws);
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    assert!(!model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    let started = Instant::now();
    model.update(cx, |m, cx| m.cancel(cx));
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[gpui_kit::test]
fn a_run_that_ends_without_a_result_fails(cx: &mut TestAppContext) {
    let (_server, _col, _state, ws, _root) = slow(cx);
    let model = sender(cx, &ws);
    let (tx, rx) = async_channel::unbounded();
    model.update(cx, |m, cx| m.follow(rx, cx));
    tx.try_send(apinata_core::RunEvent::Started { total: 1 })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(state_name(cx, &model), "sending");
    // A panicked or aborted run drops its sender without reporting a result.
    drop(tx);
    cx.run_until_parked();
    assert_eq!(
        state_name(cx, &model),
        "failed: the request ended unexpectedly"
    );
}

#[gpui_kit::test]
fn cancel_while_planning_drops_the_plan(cx: &mut TestAppContext) {
    let (server, _col, _state, ws, root) = slow(cx);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    // The test scheduler runs the actor only inside run_until_parked, so the plan is unanswered here.
    model.update(cx, |m, cx| m.cancel(cx));
    assert_eq!(state_name(cx, &model), "cancelled");
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(50));
    cx.run_until_parked();
    assert_eq!(state_name(cx, &model), "cancelled");
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn failures_are_states(cx: &mut TestAppContext) {
    let (_server, _col, _state, ws, root) = slow(cx);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("nope.yml"), cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert_eq!(state_name(cx, &model), "failed: nope.yml was not found");
    model.update(cx, |m, cx| m.send(Path::new("/not/open"), ip("a.yml"), cx));
    assert_eq!(state_name(cx, &model), "failed: the collection is not open");
}

#[gpui_kit::test]
fn the_body_cap_and_the_jar_come_from_one_executor(cx: &mut TestAppContext) {
    http_ready(cx);
    let settings = Settings {
        max_body_bytes: Some(4),
        ..Settings::default()
    };
    let exec = Executor::new(executor::exec_options(&settings)).unwrap();
    cx.update(|cx| cx.set_global(executor::SharedExecutor(Arc::new(exec))));
    let server = mock_server(
        cx,
        ResponseTemplate::new(200)
            .set_body_string("0123456789")
            .insert_header("set-cookie", "c=1; Path=/"),
    );
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let res = finished(cx, &model).response.clone().expect("a response");
    assert!(res.truncated());
    assert_eq!(res.body(), b"0123");
    let (a, b) = cx.update(|cx| (executor::shared(cx).unwrap(), executor::shared(cx).unwrap()));
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(a.cookies().list().len(), 1);
}

#[gpui_kit::test]
fn show_replaces_a_send_in_flight(cx: &mut TestAppContext) {
    let known = send_once(cx, ResponseTemplate::new(201), "").result;
    let (server, _col, _state, ws, root) = slow(cx);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    model.update(cx, |m, cx| m.show(known.clone(), cx));
    assert_eq!(state_name(cx, &model), "done: Passed");
    // The dropped send never reports back, and cancel has nothing left to stop.
    model.update(cx, |m, cx| m.cancel(cx));
    std::thread::sleep(Duration::from_millis(50));
    cx.run_until_parked();
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => assert!(Arc::ptr_eq(r, &known)),
        _ => panic!("expected the shown result"),
    });
}
