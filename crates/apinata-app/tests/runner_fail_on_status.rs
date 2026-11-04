//! The runner's "Fail on status" option: it fails a row whose reply status is in range, names
//! the status as the reason, and comes back after a restart.

use crate::support;

use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_app::session::{SavedRunnerOptions, Session};
use apinata_core::{ItemPath, Outcome, StatusRule};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::matchers::any;
use wiremock::{Mock, ResponseTemplate};

fn model_of(cx: &mut TestAppContext, s: &Started) -> Entity<RunnerModel> {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    let view = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view");
    view.read_with(cx, |v, _| v.model().clone())
}

fn label(cx: &mut TestAppContext, s: &Started, id: (&'static str, usize)) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn run_against(
    cx: &mut TestAppContext,
    status: u16,
    rule: Option<StatusRule>,
) -> (Started, Entity<RunnerModel>) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        Mock::given(any())
            .respond_with(ResponseTemplate::new(status))
            .mount(&server),
    );
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = model_of(cx, &s);
    model.update(cx, |m, cx| m.set_fail_on_status(rule, cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("run-start", cx))
        .unwrap();
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    // Keep the server and collection alive until the run is read.
    std::mem::forget((server, col));
    (s, model)
}

#[gpui_kit::test]
fn a_404_fails_the_row_with_the_status_as_its_reason(cx: &mut TestAppContext) {
    let (s, model) = run_against(cx, 404, Some(StatusRule::Error));
    model.read_with(cx, |m, _| match &m.rows()[0].state {
        apinata_app::runner_model::RowState::Done(r) => {
            assert_eq!(r.outcome, Outcome::Failed)
        }
        other => panic!("{other:?}"),
    });
    assert_eq!(
        label(cx, &s, ("run-reason", 0)).as_deref(),
        Some("Status 404")
    );
    assert_eq!(label(cx, &s, ("run-outcome", 0)).as_deref(), Some("Failed"));
}

#[gpui_kit::test]
fn with_the_option_off_a_404_still_passes(cx: &mut TestAppContext) {
    let (s, _) = run_against(cx, 404, None);
    assert_eq!(label(cx, &s, ("run-reason", 0)), None);
}

#[gpui_kit::test]
fn the_class_picks_what_fails(cx: &mut TestAppContext) {
    let (s, _) = run_against(cx, 404, Some(StatusRule::Server));
    assert_eq!(label(cx, &s, ("run-reason", 0)), None);
}

#[gpui_kit::test]
fn the_runner_has_a_fail_on_status_select(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:9", "");
    let s = started(cx, &col);
    let _ = model_of(cx, &s);
    frame(cx, s.win);
    let found = cx
        .update_window(s.win, |_, w, _| w.try_find("run-fail-on-status").is_some())
        .unwrap();
    assert!(found);
}

#[gpui_kit::test]
fn the_option_is_saved_and_an_old_session_without_it_loads() {
    let saved = SavedRunnerOptions {
        fail_on_status: Some(StatusRule::Client),
        ..Default::default()
    };
    let json = serde_json::to_string(&saved).unwrap();
    assert!(json.contains("\"fail_on_status\":\"4xx\""), "{json}");
    let back: SavedRunnerOptions = serde_json::from_str(&json).unwrap();
    assert_eq!(back, saved);
    let old: SavedRunnerOptions = serde_json::from_str("{\"bail\":true,\"retries\":2}").unwrap();
    assert_eq!(old.fail_on_status, None);
    let _ = Session::default();
}
