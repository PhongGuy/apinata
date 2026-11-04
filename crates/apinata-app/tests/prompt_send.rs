//! A send with prompt variables asks once in the dialog, remembers answers for the session,
//! and a cancel sends nothing and leaves the pane as it was.

use crate::support;

use std::sync::Arc;

use apinata_app::send_model::SendState;
use apinata_core::{ItemPath, Outcome, RequestResult, ResolveError, Response, RunError};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

fn prompted(uri: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", &format!("{uri}/{{{{?code}}}}"))),
    ])
}

fn asked(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.run_until_parked();
    frame(cx, s.win);
    present(cx, s.win, "prompt-vars-dialog")
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn continue_sends_the_answer_and_the_next_send_prefills_it(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    replace_input(cx, s.win, None, "prompt-var-0", "A1");
    click(cx, &s, "prompt-vars-continue");
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Passed);
    assert_eq!(served(cx, &server), ["/A1"]);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "prompt-vars-continue");
    finished(cx, &model);
    assert_eq!(served(cx, &server), ["/A1", "/A1"]);
}

#[gpui_kit::test]
fn cancel_keeps_the_shown_response(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    let shown = Arc::new(RequestResult::from_parts(
        ItemPath::new("a.yml").unwrap(),
        "a",
        Outcome::Passed,
        Some(Response::new(204)),
        Vec::new(),
    ));
    model.update(cx, |m, cx| m.show(shown.clone(), cx));
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "prompt-vars-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => assert!(Arc::ptr_eq(r, &shown)),
        _ => panic!("the shown response is gone"),
    });
    assert!(!asked(cx, &s));
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn kept_responses_cleared_while_asking_stay_cleared_on_cancel(cx: &mut TestAppContext) {
    use apinata_app::send_model::{Restored, forget_all_restored};
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    let kept = Arc::new(RequestResult::from_parts(
        ItemPath::new("a.yml").unwrap(),
        "a",
        Outcome::Passed,
        Some(Response::new(203)),
        Vec::new(),
    ));
    let restored = Restored {
        saved_at: std::time::SystemTime::UNIX_EPOCH,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    model.update(cx, |m, cx| m.restore(kept, restored, cx));
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    cx.update(forget_all_restored);
    click(cx, &s, "prompt-vars-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    model.read_with(cx, |m, _| {
        assert!(
            matches!(m.state(), SendState::Idle),
            "the cleared response came back"
        );
        assert_eq!(m.restored(), None);
        assert_eq!(m.viewing(), None);
    });
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn cancel_from_idle_stays_idle(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    cx.simulate_keystrokes(s.win, "escape");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert!(model.read_with(cx, |m, _| matches!(m.state(), SendState::Idle)));
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn no_prompt_no_dialog(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Passed);
    assert!(!asked(cx, &s));
    assert_eq!(served(cx, &server), ["/ok"]);
}

#[gpui_kit::test]
fn without_a_window_the_send_errors_by_name(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &model);
    assert!(
        matches!(&result.error, Some(RunError::Resolve(ResolveError::PromptRequired { labels })) if labels == &["code".to_owned()]),
        "{:?}",
        result.error
    );
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn cancelling_the_send_closes_the_open_prompt(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    model.update(cx, |m, cx| m.cancel(cx));
    assert!(model.read_with(cx, |m, _| matches!(m.state(), SendState::Cancelled)));
    assert!(!asked(cx, &s));
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn cancelling_under_another_dialog_leaves_that_dialog_open(cx: &mut TestAppContext) {
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::*;
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    cx.update_window(s.win, |_, w, cx| {
        w.open_dialog(cx, |d, _, _| {
            d.child(div().id("other-dialog").test_support().size_4())
        })
    })
    .unwrap();
    frame(cx, s.win);
    model.update(cx, |m, cx| m.cancel(cx));
    cx.run_until_parked();
    assert!(present(cx, s.win, "other-dialog"));
}

#[gpui_kit::test]
fn the_timer_starts_only_after_the_prompt_is_answered(cx: &mut TestAppContext) {
    http_ready(cx);
    let slow = ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30));
    let server = mock_server(cx, slow);
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    model.read_with(cx, |m, _| {
        assert!(m.is_waiting());
        assert_eq!(m.elapsed(), None);
    });
    replace_input(cx, s.win, None, "prompt-var-0", "A1");
    click(cx, &s, "prompt-vars-continue");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| !m.is_waiting() && m.elapsed().is_some())
    });
    // A cancel before the run reaches the request ends it as Skipped, so wait until it is sent.
    wait_real(cx, |cx| served(cx, &server).len() == 1);
    model.update(cx, |m, cx| m.cancel(cx));
    assert_eq!(finished(cx, &model).outcome, Outcome::Cancelled);
}

#[gpui_kit::test]
fn cancelling_the_first_send_prompt_leaves_the_pane_never_shown(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = prompted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    assert!(!model.read_with(cx, |m, _| m.ever_shown()));
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    click(cx, &s, "prompt-vars-cancel");
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert!(
        !model.read_with(cx, |m, _| m.ever_shown()),
        "a cancelled first send opened the pane"
    );
    assert!(served(cx, &server).is_empty());
}
