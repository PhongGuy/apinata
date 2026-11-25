//! The runner's cookie and variable options: a run on its own jar, cookies kept back into
//! the shared jar, and a run on a forked variable context that "Re-run failed" reuses.

use crate::support;

use apinata_app::runner_model::{RowState, RunCookies, RunState, RunnerModel};
use apinata_core::{CookieInfo, ItemPath};
use gpui_kit::{Entity, TestAppContext};
use support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn runner(cx: &mut TestAppContext, s: &Started) -> Entity<RunnerModel> {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    let view = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view");
    view.read_with(cx, |v, _| v.model().clone())
}

fn run_to_end(cx: &mut TestAppContext, model: &Entity<RunnerModel>, rerun: bool) {
    let started = model.update(cx, |m, cx| {
        if rerun {
            m.rerun_failed(cx)
        } else {
            m.start(cx)
        }
    });
    assert!(started, "the run starts");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
}

/// A server whose `/ok` sets the cookie `run=1`.
fn cookie_server(cx: &mut TestAppContext) -> MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200).insert_header("set-cookie", "run=1; Path=/"))
            .mount(&server),
    );
    server
}

/// The `Cookie` header of every request the server received, in order.
fn cookies_sent(cx: &mut TestAppContext, server: &MockServer) -> Vec<Option<String>> {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let got = handle
        .block_on(server.received_requests())
        .unwrap_or_default();
    got.iter()
        .map(|r| {
            r.headers
                .get("cookie")
                .map(|v| v.to_str().unwrap().to_owned())
        })
        .collect()
}

/// The shared jar's cookie names, after seeding it with `sid=shared` for 127.0.0.1.
fn seed_shared_jar(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let exec = apinata_app::executor::shared(cx).expect("an executor");
        exec.cookies()
            .insert(CookieInfo::new("127.0.0.1", "sid", "shared"))
            .unwrap();
    });
}

fn shared_names(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        let exec = apinata_app::executor::shared(cx).expect("an executor");
        exec.cookies().list().into_iter().map(|c| c.name).collect()
    })
}

fn cookie_run(
    cx: &mut TestAppContext,
    setup: impl FnOnce(&mut RunnerModel, &mut gpui_kit::Context<RunnerModel>),
) -> (Vec<Option<String>>, Vec<String>) {
    http_ready(cx);
    let server = cookie_server(cx);
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    seed_shared_jar(cx);
    let model = runner(cx, &s);
    model.update(cx, setup);
    run_to_end(cx, &model, false);
    (cookies_sent(cx, &server), shared_names(cx))
}

#[gpui_kit::test]
fn by_default_a_run_reads_and_fills_the_shared_jar(cx: &mut TestAppContext) {
    let (sent, shared) = cookie_run(cx, |m, _| {
        assert_eq!(
            (m.run_cookies(), m.keep_cookies(), m.keep_variables()),
            (RunCookies::Shared, false, true)
        );
    });
    assert_eq!(sent, [Some("sid=shared".to_owned())]);
    assert_eq!(shared, ["run", "sid"]);
}

#[gpui_kit::test]
fn an_empty_jar_run_sends_no_shared_cookie_and_leaves_the_jar_alone(cx: &mut TestAppContext) {
    let (sent, shared) = cookie_run(cx, |m, cx| m.set_run_cookies(RunCookies::Empty, cx));
    assert_eq!(sent, [None]);
    assert_eq!(shared, ["sid"]);
}

#[gpui_kit::test]
fn a_copied_jar_run_sends_the_shared_cookie_but_keeps_its_own(cx: &mut TestAppContext) {
    let (sent, shared) = cookie_run(cx, |m, cx| m.set_run_cookies(RunCookies::Copy, cx));
    assert_eq!(sent, [Some("sid=shared".to_owned())]);
    assert_eq!(shared, ["sid"]);
}

#[gpui_kit::test]
fn keep_puts_the_runs_cookies_back_into_the_shared_jar(cx: &mut TestAppContext) {
    let (sent, shared) = cookie_run(cx, |m, cx| {
        m.set_run_cookies(RunCookies::Empty, cx);
        m.set_keep_cookies(true, cx);
    });
    assert_eq!(sent, [None]);
    assert_eq!(shared, ["run", "sid"]);
}

const ACTION: &str = "runtime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"res.body.id\"\n        method: jsonq\n      variable:\n        name: \"id\"\n        scope: runtime\n";

/// `a` captures `id` from `/set`; `b` asks `/check/{{id}}`, which fails once and then passes.
fn variable_server(cx: &mut TestAppContext) -> (MockServer, tempfile::TempDir) {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(path("/set"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("{\"id\":\"forked\"}", "application/json"),
            )
            .mount(&server),
    );
    handle.block_on(
        Mock::given(path("/check/forked"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server),
    );
    handle.block_on(
        Mock::given(path("/check/forked"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let uri = server.uri();
    let a = format!("{}{ACTION}", request("a", &format!("{uri}/set")));
    let b = format!(
        "info:\n  name: b\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{uri}/check/{{{{id}}}}\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n"
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)]);
    (server, col)
}

fn shared_id(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    let vars =
        s.ws.read_with(cx, |w, cx| w.runtime_vars(&s.root, cx))
            .expect("the collection is open");
    (vars.entries().iter())
        .find(|v| v.name() == "id")
        .and_then(|v| v.value().map(str::to_owned))
}

#[gpui_kit::test]
fn by_default_a_runs_variables_reach_the_collection(cx: &mut TestAppContext) {
    http_ready(cx);
    let (_server, col) = variable_server(cx);
    let s = started(cx, &col);
    let model = runner(cx, &s);
    run_to_end(cx, &model, false);
    assert_eq!(shared_id(cx, &s).as_deref(), Some("forked"));
}

#[gpui_kit::test]
fn a_forked_run_leaves_the_collections_variables_alone_and_its_rerun_reuses_the_fork(
    cx: &mut TestAppContext,
) {
    http_ready(cx);
    let (server, col) = variable_server(cx);
    let s = started(cx, &col);
    let model = runner(cx, &s);
    model.update(cx, |m, cx| m.set_keep_variables(false, cx));
    run_to_end(cx, &model, false);
    assert_eq!(shared_id(cx, &s), None, "the value stayed in the fork");
    assert_eq!(model.read_with(cx, |m, _| m.failed_items()), [ip("b.yml")]);
    // Only the fork knows `id`, so `b` resolves and now passes only when the re-run uses it.
    run_to_end(cx, &model, true);
    assert!(model.read_with(cx, |m, _| m.failed_items()).is_empty());
    assert_eq!(
        served(cx, &server),
        ["/set", "/check/forked", "/check/forked"]
    );
    assert_eq!(shared_id(cx, &s), None);
}

/// `a` gets the cookie `run=1` from `/ok`; `b` asks `/need`, which fails once and then passes.
fn rerun_server(cx: &mut TestAppContext) -> (MockServer, tempfile::TempDir) {
    let server = cookie_server(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(
        Mock::given(path("/need"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server),
    );
    handle.block_on(
        Mock::given(path("/need"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let uri = server.uri();
    let b = format!(
        "info:\n  name: b\n  type: http\n  seq: 2\nhttp:\n  method: GET\n  url: \"{uri}/need\"\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n"
    );
    let a = request("a", &format!("{uri}/ok"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a), ("b.yml", &b)]);
    (server, col)
}

fn empty_jar_rerun(
    cx: &mut TestAppContext,
    between: impl FnOnce(&mut TestAppContext),
) -> Vec<Option<String>> {
    http_ready(cx);
    let (server, col) = rerun_server(cx);
    let s = started(cx, &col);
    seed_shared_jar(cx);
    let model = runner(cx, &s);
    model.update(cx, |m, cx| m.set_run_cookies(RunCookies::Empty, cx));
    run_to_end(cx, &model, false);
    between(cx);
    run_to_end(cx, &model, true);
    assert!(model.read_with(cx, |m, _| m.failed_items()).is_empty());
    assert_eq!(shared_names(cx), ["sid"]);
    cookies_sent(cx, &server)
}

#[gpui_kit::test]
fn a_rerun_sends_with_the_runs_own_jar(cx: &mut TestAppContext) {
    let sent = empty_jar_rerun(cx, |_| {});
    assert_eq!(
        sent,
        [None, Some("run=1".to_owned()), Some("run=1".to_owned())]
    );
}

#[gpui_kit::test]
fn a_rerun_follows_the_cookie_switches_set_since_the_run(cx: &mut TestAppContext) {
    let sent = empty_jar_rerun(cx, |cx| {
        cx.update(|cx| {
            apinata_app::executor::shared(cx)
                .unwrap()
                .set_send_cookies(false)
        });
    });
    assert_eq!(sent, [None, Some("run=1".to_owned()), None]);
}

#[gpui_kit::test]
fn a_cancelled_run_keeps_no_cookies(cx: &mut TestAppContext) {
    http_ready(cx);
    let (_server, col) = rerun_server(cx);
    let s = started(cx, &col);
    seed_shared_jar(cx);
    let model = runner(cx, &s);
    model.update(cx, |m, cx| {
        m.set_run_cookies(RunCookies::Empty, cx);
        m.set_keep_cookies(true, cx);
        m.set_delay_ms(60_000, cx);
        assert!(m.start(cx));
    });
    // The first request has stored its cookie; the long delay holds the run before the second.
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| {
            matches!(m.rows().first().map(|r| &r.state), Some(RowState::Done(_)))
        })
    });
    model.update(cx, |m, cx| m.cancel(cx));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    assert_eq!(shared_names(cx), ["sid"]);
}
