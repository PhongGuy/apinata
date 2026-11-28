//! The runner's parallel degree: kept and saved like the other options, refused before anything
//! is sent when the plan or the delay rules it out, and the cookie warning on the rows.

use crate::support;

use apinata_app::runner_model::{RowState, RunState, RunnerModel};
use apinata_app::runner_view::row_warning;
use apinata_app::workspace::Workspace;
use apinata_core::{Outcome, RequestResult};
use gpui_kit::{AppContext, Entity, TestAppContext};
use std::path::Path;
use std::sync::Arc;
use support::*;
use wiremock::ResponseTemplate;

const CAPTURE: &str = "runtime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.status\n        method: jsonq\n      variable:\n        name: code\n        scope: runtime\n";

fn get(name: &str, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}/{name}\"\n{extra}"
    )
}

fn runner(cx: &mut TestAppContext, ws: &Entity<Workspace>, root: &Path) -> Entity<RunnerModel> {
    let (ws, root) = (ws.clone(), root.to_path_buf());
    cx.new(|_| RunnerModel::new(ws, root, ip("")))
}

// Starts the run and waits for it to settle, returning why it failed, if it did.
fn run(cx: &mut TestAppContext, m: &Entity<RunnerModel>) -> Option<String> {
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    m.read_with(cx, |m, _| match m.state() {
        RunState::Failed(why) => Some(why.clone()),
        _ => None,
    })
}

fn results(cx: &mut TestAppContext, m: &Entity<RunnerModel>) -> Vec<Arc<RequestResult>> {
    m.read_with(cx, |m, _| {
        (m.rows().iter())
            .filter_map(|r| match &r.state {
                RowState::Done(x) => Some(Arc::clone(x)),
                _ => None,
            })
            .collect()
    })
}

#[gpui_kit::test]
fn the_degree_is_clamped_to_1_to_64(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root);
    assert_eq!(m.read_with(cx, |m, _| m.parallel()), 1);
    m.update(cx, |m, cx| m.set_parallel(0, cx));
    assert_eq!(m.read_with(cx, |m, _| m.parallel()), 1);
    m.update(cx, |m, cx| m.set_parallel(500, cx));
    assert_eq!(m.read_with(cx, |m, _| m.parallel()), 64);
    m.update(cx, |m, cx| m.set_parallel(4, cx));
    assert_eq!(m.read_with(cx, |m, _| m.parallel()), 4);
}

#[gpui_kit::test]
fn a_plan_that_cannot_run_in_parallel_fails_before_anything_is_sent(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let u = server.uri();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &get("one", &u, "")),
        ("two.yml", &get("two", &u, CAPTURE)),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root);
    m.update(cx, |m, cx| m.set_parallel(2, cx));
    let failed = run(cx, &m).expect("refused");
    assert!(failed.contains("two.yml"), "{failed}");
    assert!(failed.contains("cannot run in parallel"), "{failed}");
    assert!(m.read_with(cx, |m, _| m.rows().is_empty()));
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn a_parallel_run_with_a_delay_fails_before_anything_is_sent(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &get("one", &server.uri(), "")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root);
    m.update(cx, |m, cx| {
        m.set_parallel(2, cx);
        m.set_delay_ms(10, cx);
    });
    let failed = run(cx, &m).expect("refused");
    assert!(failed.contains("delay"), "{failed}");
    assert!(served(cx, &server).is_empty());
}

#[gpui_kit::test]
fn a_parallel_run_sends_all_in_plan_order_and_warns_of_unseen_cookies(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).append_header("set-cookie", "sid=one; Path=/"),
    );
    let u = server.uri();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &get("one", &u, "")),
        ("two.yml", &get("two", &u, "")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root);
    m.update(cx, |m, cx| m.set_parallel(2, cx));
    assert_eq!(run(cx, &m), None);
    let done = results(cx, &m);
    let paths: Vec<&str> = done.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["one.yml", "two.yml"]);
    assert!(done.iter().all(|r| r.outcome == Outcome::Passed));
    let mut hit = served(cx, &server);
    hit.sort();
    assert_eq!(hit, ["/one", "/two"]);
    let warning = row_warning(&done[0]).expect("the first row warns");
    assert!(warning.contains("running alongside"), "{warning}");
    assert_eq!(
        row_warning(&done[1]),
        None,
        "the last request warns of nothing"
    );
}

#[gpui_kit::test]
fn the_degree_is_saved_with_the_runner_options(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (w, r) = (ws.clone(), root.clone());
    let first = cx.new(|_| RunnerModel::new(w, r, ip("api")));
    first.update(cx, |m, cx| m.set_parallel(8, cx));
    let second = cx.new(|_| RunnerModel::new(ws.clone(), root.clone(), ip("api")));
    second.update(cx, |m, cx| m.restore_options(cx));
    assert_eq!(second.read_with(cx, |m, _| m.parallel()), 8);
}

#[gpui_kit::test]
fn two_rows_waiting_out_a_retry_at_once_both_say_so(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(503));
    let u = server.uri();
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &get("one", &u, "")),
        ("two.yml", &get("two", &u, "")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let m = runner(cx, &ws, &root);
    m.update(cx, |m, cx| {
        m.set_parallel(2, cx);
        m.set_retries(1, cx);
        m.set_retry_delay_ms(60_000, cx);
    });
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| {
        m.read_with(cx, |m, _| {
            m.retrying(0).is_some() && m.retrying(1).is_some()
        })
    });
    assert_eq!(m.read_with(cx, |m, _| m.retrying(0)), Some((2, 2)));
    m.update(cx, |m, cx| m.cancel(cx));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    assert_eq!(m.read_with(cx, |m, _| m.retrying(0)), None);
}
