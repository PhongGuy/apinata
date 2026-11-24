//! What the runner says about a run that stopped early, which environments a run uses, and
//! where the retry options sit.

use crate::support;

use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_core::{ItemPath, Outcome, RunStop};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Entity, Pixels, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

const FAILS: &str = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";

/// Three requests to `<uri>/ok`, each asserting a 200.
fn three(uri: &str) -> tempfile::TempDir {
    let req = |name: &str, seq: u32| {
        format!(
            "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{uri}/ok\"\n{FAILS}"
        )
    };
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/stage.yml", "name: stage\n"),
        ("a.yml", &req("a", 1)),
        ("b.yml", &req("b", 2)),
        ("c.yml", &req("c", 3)),
    ])
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

fn label(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn find(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Bounds<Pixels> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

#[gpui_kit::test]
fn a_stopped_run_lists_every_iteration_and_says_why_it_stopped(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(500));
    let col = three(&server.uri());
    let s = started(cx, &col);
    let model = open(cx, &s);
    model.update(cx, |m, cx| {
        m.set_bail(true, cx);
        m.set_iteration_count(Some(2), cx);
    });
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("run-start", cx))
        .unwrap();
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    let (marks, outcomes) = model.read_with(cx, |m, _| {
        let rows = m.rows();
        let marks: Vec<Option<usize>> = rows.iter().map(|r| r.iteration).collect();
        let outcomes: Vec<Option<Outcome>> = (rows.iter())
            .map(|r| match &r.state {
                apinata_app::runner_model::RowState::Done(x) => Some(x.outcome),
                _ => None,
            })
            .collect();
        (marks, outcomes)
    });
    assert_eq!(
        marks,
        [Some(0), Some(0), Some(0), Some(1), Some(1), Some(1)]
    );
    assert_eq!(outcomes[0], Some(Outcome::Failed));
    assert!(
        outcomes[1..].iter().all(|o| *o == Some(Outcome::Skipped)),
        "{outcomes:?}"
    );
    let stop = model.read_with(cx, |m, _| match m.state() {
        RunState::Finished(summary) => summary.stop,
        _ => None,
    });
    assert_eq!(stop, Some(RunStop::FirstFailure));
    assert_eq!(
        label(cx, &s, "run-summary").as_deref(),
        Some("6 requests: 0 passed, 1 failed, 5 not sent (stopped after the first failure)")
    );
}

#[gpui_kit::test]
fn the_runner_names_the_environments_a_run_uses(cx: &mut TestAppContext) {
    let col = three("http://127.0.0.1:9");
    let s = started(cx, &col);
    open(cx, &s);
    // The collection's only environment is chosen when it opens.
    assert_eq!(
        label(cx, &s, "run-environment").as_deref(),
        Some("Environment: stage, Global: None")
    );
    let globals = write_tree(&[("shared.yml", "name: shared\n")]);
    let dir = globals.path().to_path_buf();
    s.ws.update(cx, |w, cx| w.set_global_dir(dir, cx));
    cx.run_until_parked();
    assert_eq!(
        s.ws.read_with(cx, |w, _| w.global_environments()),
        ["shared"]
    );
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, None, cx);
        w.select_global_environment(Some("shared".into()), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        label(cx, &s, "run-environment").as_deref(),
        Some("Environment: None, Global: shared")
    );
    let line = find(cx, &s, "run-environment");
    let run = find(cx, &s, "run-start");
    assert!(
        line.bottom() <= run.top(),
        "environment line {line:?} not above Run {run:?}"
    );
}

#[gpui_kit::test]
fn the_retry_options_share_one_row(cx: &mut TestAppContext) {
    let col = three("http://127.0.0.1:9");
    let s = started(cx, &col);
    open(cx, &s);
    let retries = find(cx, &s, "run-retries");
    for id in ["run-retry-delay", "run-retry-unsafe"] {
        let b = find(cx, &s, id);
        assert!(
            b.top() < retries.bottom() && b.bottom() > retries.top(),
            "{id} {b:?} not on the line of run-retries {retries:?}"
        );
    }
    let status = find(cx, &s, "run-fail-on-status");
    assert!(
        retries.top() >= status.bottom(),
        "run-retries {retries:?} shares a line with Fail on status {status:?}"
    );
}
