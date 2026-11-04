//! A collection run asks once for the union of its requests' prompts, after tag filtering.

use crate::support;

use apinata_app::runner_model::{RunState, RunnerModel};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

fn req(uri: &str, name: &str, seq: u32, path: &str, tags: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\n{tags}http:\n  method: GET\n  url: \"{uri}/{path}\"\n"
    )
}

fn suite(uri: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req(uri, "a", 1, "{{?code}}", "")),
        ("b.yml", &req(uri, "b", 2, "b/{{?code}}", "")),
        (
            "c.yml",
            &req(uri, "c", 3, "{{?other}}", "  tags:\n    - slow\n"),
        ),
    ])
}

fn runner(cx: &mut TestAppContext, s: &Started) -> Entity<RunnerModel> {
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let m = cx.new(|_| RunnerModel::new(ws, root, ip("")));
    m.update(cx, |m, cx| m.set_exclude_tags("slow".into(), cx));
    m
}

fn asked(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.run_until_parked();
    frame(cx, s.win);
    present(cx, s.win, "prompt-vars-dialog")
}

#[gpui_kit::test]
fn one_question_answers_every_request(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri());
    let s = started(cx, &col);
    let m = runner(cx, &s);
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| asked(cx, &s));
    assert!(
        !present(cx, s.win, "prompt-var-1"),
        "the excluded request's label is not asked"
    );
    replace_input(cx, s.win, None, "prompt-var-0", "Z9");
    cx.update_window(s.win, |_, w, cx| w.click("prompt-vars-continue", cx))
        .unwrap();
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    assert!(matches!(
        m.read_with(cx, |m, _| m.state().clone()),
        RunState::Finished(_)
    ));
    assert!(!asked(cx, &s), "asked once");
    assert_eq!(served(cx, &server), ["/Z9", "/b/Z9"]);
}

#[gpui_kit::test]
fn cancel_does_not_start_the_run(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri());
    let s = started(cx, &col);
    let m = runner(cx, &s);
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| asked(cx, &s));
    cx.update_window(s.win, |_, w, cx| w.click("prompt-vars-cancel", cx))
        .unwrap();
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    assert!(matches!(
        m.read_with(cx, |m, _| m.state().clone()),
        RunState::Cancelled
    ));
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn cancelling_the_run_closes_the_open_prompt(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = suite(&server.uri());
    let s = started(cx, &col);
    let m = runner(cx, &s);
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| asked(cx, &s));
    m.update(cx, |m, cx| m.cancel(cx));
    assert!(matches!(
        m.read_with(cx, |m, _| m.state().clone()),
        RunState::Cancelled
    ));
    assert!(!asked(cx, &s), "the dropped run closes its prompt");
    assert!(served(cx, &server).is_empty());
}
