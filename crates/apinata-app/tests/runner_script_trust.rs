//! The collection runner and a single send share one script trust question and one stored
//! answer per collection: whichever asks first, the other honours the answer.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::runner_model::{RowState, RunnerModel};
use apinata_app::settings::RunScripts;
use apinata_core::{ItemPath, ScriptDecision, ScriptTrust};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;

const SETS_HEADER: &str = "req.setHeader('x-ran', 'yes')";

// A request whose pre-request script marks what it sends, so the server shows whether it ran.
fn scripted(uri: &str) -> tempfile::TempDir {
    collection_for(
        uri,
        &format!(
            "runtime:\n  scripts:\n    - type: before-request\n      code: \"{SETS_HEADER}\"\n"
        ),
    )
}

fn decision(s: &Started) -> Option<ScriptDecision> {
    ScriptTrust::load(s.files.path().join("script-trust.json"))
        .unwrap()
        .decision(&s.root)
}

fn asked(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.run_until_parked();
    frame(cx, s.win);
    present(cx, s.win, "script-trust-run")
}

fn answer(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

fn start_run(cx: &mut TestAppContext, s: &Started) -> Entity<RunnerModel> {
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let runner = cx.new(|_| RunnerModel::new(ws, root, ItemPath::root()));
    assert!(runner.update(cx, |m, cx| m.start(cx)));
    runner
}

// Waits for the run to end and returns its one request's warnings.
fn run_warnings(cx: &mut TestAppContext, runner: &Entity<RunnerModel>) -> Vec<String> {
    wait_real(cx, |cx| runner.read_with(cx, |m, _| !m.is_running()));
    runner.read_with(cx, |m, _| match &m.rows()[0].state {
        RowState::Done(r) => r.warnings.iter().map(|w| w.to_string()).collect(),
        _ => panic!("the run has no result"),
    })
}

fn send_warnings(cx: &mut TestAppContext, s: &Started) -> (bool, Vec<String>) {
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    let asked = asked(cx, s);
    if asked {
        cx.simulate_keystrokes(s.win, "escape");
    }
    let result = finished(cx, &model);
    let warnings = result.warnings.iter().map(|w| w.to_string()).collect();
    (asked, warnings)
}

// The `x-ran` header of each request the server received, in arrival order.
fn ran(cx: &mut TestAppContext, server: &wiremock::MockServer) -> Vec<Option<String>> {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let got = handle.block_on(server.received_requests()).unwrap();
    got.iter()
        .map(|r| {
            r.headers
                .get("x-ran")
                .map(|v| v.to_str().unwrap().to_owned())
        })
        .collect()
}

fn skipped(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains("script"))
}

#[gpui_kit::test]
fn a_set_variable_action_is_no_script_so_neither_path_asks(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({"id": 1, "title": "t"})),
    );
    // A selector and assertions on `res` chains are evaluated without JavaScript.
    let col = collection_for(
        &server.uri(),
        "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n    - expression: res.body.id\n      operator: eq\n      value: \"1\"\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.title\n        method: jsonq\n      variable:\n        name: postTitle\n        scope: runtime\n",
    );
    let s = started(cx, &col);
    let runner = start_run(cx, &s);
    assert!(!asked(cx, &s));
    assert!(!skipped(&run_warnings(cx, &runner)));
    let (asked, warnings) = send_warnings(cx, &s);
    assert!(!asked, "a send with nothing to approve does not ask");
    assert!(!skipped(&warnings), "{warnings:?}");
    assert_eq!(decision(&s), None);
}

#[gpui_kit::test]
fn a_run_asks_before_sending_and_run_scripts_carries_to_a_single_send(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = scripted(&server.uri());
    let s = started(cx, &col);
    let runner = start_run(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    assert!(
        ran(cx, &server).is_empty(),
        "nothing is sent before the answer"
    );
    answer(cx, &s, "script-trust-run");
    assert!(!skipped(&run_warnings(cx, &runner)));
    assert_eq!(decision(&s), Some(ScriptDecision::Allowed));
    let (asked, warnings) = send_warnings(cx, &s);
    assert!(!asked, "the runner's answer holds for a single send");
    assert!(!skipped(&warnings), "{warnings:?}");
    assert_eq!(ran(cx, &server), [Some("yes".into()), Some("yes".into())]);
}

#[gpui_kit::test]
fn run_scripts_in_a_single_send_carries_to_the_runner(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = scripted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    answer(cx, &s, "script-trust-run");
    finished(cx, &model);
    let runner = start_run(cx, &s);
    assert!(!asked(cx, &s), "the send's answer holds for the runner");
    assert!(!skipped(&run_warnings(cx, &runner)));
    assert_eq!(ran(cx, &server), [Some("yes".into()), Some("yes".into())]);
}

#[gpui_kit::test]
fn dont_run_in_the_runner_runs_without_scripts_and_stores_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = scripted(&server.uri());
    let s = started(cx, &col);
    let runner = start_run(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    answer(cx, &s, "script-trust-skip");
    assert!(skipped(&run_warnings(cx, &runner)));
    assert_eq!(ran(cx, &server), [None]);
    assert_eq!(decision(&s), None);
    // Like a single send's Don't run, it covers this run alone.
    let (asked, _) = send_warnings(cx, &s);
    assert!(asked, "Don't run is not remembered");
}

#[gpui_kit::test]
fn never_in_the_runner_carries_to_a_single_send(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = scripted(&server.uri());
    let s = started(cx, &col);
    let runner = start_run(cx, &s);
    wait_real(cx, |cx| asked(cx, &s));
    answer(cx, &s, "script-trust-never");
    assert!(skipped(&run_warnings(cx, &runner)));
    assert_eq!(decision(&s), Some(ScriptDecision::Blocked));
    let (asked, warnings) = send_warnings(cx, &s);
    assert!(!asked);
    assert!(skipped(&warnings), "{warnings:?}");
    assert_eq!(ran(cx, &server), [None, None]);
}

#[gpui_kit::test]
fn never_in_a_single_send_carries_to_the_runner(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = scripted(&server.uri());
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx));
    wait_real(cx, |cx| asked(cx, &s));
    answer(cx, &s, "script-trust-never");
    finished(cx, &model);
    let runner = start_run(cx, &s);
    assert!(!asked(cx, &s));
    assert!(skipped(&run_warnings(cx, &runner)));
    assert_eq!(ran(cx, &server), [None, None]);
}

#[gpui_kit::test]
fn the_never_preference_skips_runner_scripts_without_asking(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = scripted(&server.uri());
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| {
            p.update(cx, |s| s.run_scripts = RunScripts::Never)
        });
    });
    let runner = start_run(cx, &s);
    assert!(!asked(cx, &s));
    assert!(skipped(&run_warnings(cx, &runner)));
    assert_eq!(ran(cx, &server), [None]);
    assert_eq!(decision(&s), None);
}
