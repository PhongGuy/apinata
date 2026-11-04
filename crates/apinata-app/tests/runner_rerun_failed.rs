//! The runner's "Re-run failed" button: shown after a run with failures, it runs only those.

use crate::support;

use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const BUTTON: &str = "run-rerun-failed";

/// A request to `url` asserting a 200.
fn checked(name: &str, seq: u32, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n"
    )
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

fn open(cx: &mut TestAppContext, s: &Started) -> Entity<RunnerModel> {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    s.shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view")
        .read_with(cx, |v, _| v.model().clone())
}

fn press_and_finish(
    cx: &mut TestAppContext,
    s: &Started,
    model: &Entity<RunnerModel>,
    button: &'static str,
) {
    click(cx, s, button);
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
}

fn row_names(cx: &mut TestAppContext, model: &Entity<RunnerModel>) -> Vec<String> {
    model.read_with(cx, |m, _| m.rows().iter().map(|r| r.name.clone()).collect())
}

/// A server whose `/flaky` answers 500 once and 200 afterwards, and whose `/ok` always answers 200.
fn server(cx: &mut TestAppContext) -> MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(async {
        Mock::given(path("/flaky"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(path("/flaky"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    });
    server
}

fn hits(cx: &mut TestAppContext, server: &MockServer, route: &str) -> usize {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let seen = handle
        .block_on(server.received_requests())
        .unwrap_or_default();
    seen.iter().filter(|r| r.url.path() == route).count()
}

fn tree(uri: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &checked("a", 1, &format!("{uri}/ok"))),
        ("b.yml", &checked("b", 2, &format!("{uri}/flaky"))),
        ("c.yml", &checked("c", 3, &format!("{uri}/ok"))),
    ])
}

#[gpui_kit::test]
fn the_button_is_absent_before_a_run_and_after_an_all_pass_run(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = server(cx);
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = open(cx, &s);
    assert!(!present(cx, s.win, BUTTON));
    press_and_finish(cx, &s, &model, "run-start");
    assert!(!present(cx, s.win, BUTTON));
}

#[gpui_kit::test]
fn the_button_reruns_only_the_failed_requests_until_they_pass(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = server(cx);
    let col = tree(&server.uri());
    let s = started(cx, &col);
    let model = open(cx, &s);
    press_and_finish(cx, &s, &model, "run-start");
    assert_eq!(row_names(cx, &model), ["a", "b", "c"]);
    assert!(present(cx, s.win, BUTTON));

    press_and_finish(cx, &s, &model, BUTTON);
    assert_eq!(row_names(cx, &model), ["b"]);
    assert_eq!(hits(cx, &server, "/ok"), 2);
    assert_eq!(hits(cx, &server, "/flaky"), 2);
    // The re-run passed, so nothing is left to re-run.
    assert!(!present(cx, s.win, BUTTON));
}

#[gpui_kit::test]
fn a_normal_run_after_a_rerun_is_a_full_run(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = server(cx);
    let col = tree(&server.uri());
    let s = started(cx, &col);
    let model = open(cx, &s);
    press_and_finish(cx, &s, &model, "run-start");
    press_and_finish(cx, &s, &model, BUTTON);
    press_and_finish(cx, &s, &model, "run-start");
    assert_eq!(row_names(cx, &model), ["a", "b", "c"]);
}

#[gpui_kit::test]
fn rerun_failed_does_nothing_when_nothing_failed(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = server(cx);
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let model = open(cx, &s);
    assert!(!model.update(cx, |m, cx| m.rerun_failed(cx)));
    press_and_finish(cx, &s, &model, "run-start");
    assert!(!model.update(cx, |m, cx| m.rerun_failed(cx)));
    assert_eq!(row_names(cx, &model), ["a"]);
}
