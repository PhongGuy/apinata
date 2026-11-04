//! A secret a script prints, names a test with, or throws, or a failing assertion shows as its
//! actual value, is redacted in the response pane's Console and Tests tabs, its error block,
//! and the runner's row reasons.

use crate::support;

use std::sync::Arc;

use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_app::runner_model::{RowState, RunState};
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext, WindowOptions,
};
use support::*;
use wiremock::ResponseTemplate;

const SECRET: &str = "Pane-S3cret-58";

// `code` as one script item of `kind`, indented under `runtime.scripts`.
fn script(kind: &str, code: &str) -> String {
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    format!("    - type: {kind}\n      code: |-\n{body}")
}

// `a.yml` logs the secret and fails a test named with it; `b.yml` throws it before sending.
fn secret_app(cx: &mut TestAppContext, uri: &str) -> (tempfile::TempDir, Started) {
    let logs = script(
        "before-request",
        "console.log(bru.getEnvVar('token'))\nconsole.log('plain-41')",
    ) + &script(
        "tests",
        "test('saw ' + bru.getEnvVar('token'), () => expect(1).to.equal(2))",
    );
    let a = format!(
        "{}runtime:\n  scripts:\n{logs}",
        request("a", &format!("{uri}/ok"))
    );
    let throws = script("before-request", "throw new Error(bru.getEnvVar('token'))");
    let b = format!(
        "{}runtime:\n  scripts:\n{throws}",
        request("b", &format!("{uri}/ok"))
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
        ("b.yml", &b),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), SECRET).unwrap();
    let root = col.path().to_path_buf();
    let s = started_on(cx, &col, |p| allow_scripts(p, &root), Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    (col, s)
}

// Sends `rel` and opens a response pane on its result in a window of its own.
fn pane_for(
    cx: &mut TestAppContext,
    s: &Started,
    rel: &str,
) -> (
    Arc<apinata_core::RequestResult>,
    AnyWindowHandle,
    Entity<ResponsePane>,
) {
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip(rel), cx)));
    let result = finished(cx, &model);
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    (result, win, view)
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_console_and_tests_tabs_redact_a_secret(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let (_col, s) = secret_app(cx, &server.uri());
    let (result, win, view) = pane_for(cx, &s, "a.yml");
    assert_eq!(
        result.console[0].text, SECRET,
        "the script logged the secret"
    );
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Console, cx));
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("console-copy", cx)
    })
    .unwrap();
    cx.run_until_parked();
    let copied = cx.read_from_clipboard().and_then(|c| c.text()).unwrap();
    assert_eq!(
        copied,
        "log [pre-request] [redacted]\nlog [pre-request] plain-41"
    );
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Tests, cx));
    cx.run_until_parked();
    let line = label(cx, win, ("test", 0usize));
    assert_eq!(line, "saw [redacted]: expected 1 to equal 2");
}

#[gpui_kit::test]
fn the_error_block_redacts_a_thrown_secret(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let (_col, s) = secret_app(cx, &server.uri());
    let (result, _win, view) = pane_for(cx, &s, "b.yml");
    assert!(result.error.as_ref().unwrap().to_string().contains(SECRET));
    let message = view
        .read_with(cx, |p, cx| p.error_view(cx))
        .and_then(|e| e.message)
        .expect("an error message");
    assert!(message.contains("[redacted]"), "{message}");
    assert!(!message.contains(SECRET), "{message}");
}

#[gpui_kit::test]
fn the_runner_rows_redact_a_secret_in_their_reason(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let (_col, s) = secret_app(cx, &server.uri());
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("run-start", cx))
        .unwrap();
    cx.run_until_parked();
    let runner = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view");
    let model = runner.read_with(cx, |v, _| v.model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    let rows = model.read_with(cx, |m, _| m.rows().len());
    let reasons: Vec<String> = (0..rows)
        .map(|ix| label(cx, s.win, ("run-reason", ix)))
        .collect();
    let raw = model.read_with(cx, |m, _| {
        (m.rows().iter()).any(|r| {
            matches!(&r.state, RowState::Done(r) if r.error.as_ref().is_some_and(|e| e.to_string().contains(SECRET)))
        })
    });
    assert!(raw, "a row's raw error holds the secret");
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    for why in &reasons {
        assert!(why.contains("[redacted]"), "{why}");
        assert!(!why.contains(SECRET), "{why}");
    }
}

// `e.yml` asserts the echoed body field differs from the secret, so the assertion fails with
// the secret as its actual value.
fn echo_app(cx: &mut TestAppContext, uri: &str) -> (tempfile::TempDir, Started) {
    let e = format!(
        "{}runtime:\n  assertions:\n    - expression: res.body.echo\n      operator: neq\n      value: \"{{{{token}}}}\"\n",
        request("e", &format!("{uri}/echo"))
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("e.yml", &e),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), SECRET).unwrap();
    let root = col.path().to_path_buf();
    let s = started_on(cx, &col, |p| allow_scripts(p, &root), Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    (col, s)
}

fn echo() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({ "echo": SECRET }))
}

#[gpui_kit::test]
fn the_tests_tab_redacts_a_secret_an_assertion_shows_as_actual(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, echo());
    let (_col, s) = echo_app(cx, &server.uri());
    let (result, win, view) = pane_for(cx, &s, "e.yml");
    assert_eq!(result.assertions.len(), 1, "{result:?}");
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Tests, cx));
    cx.run_until_parked();
    let line = label(cx, win, ("assertion", 0usize));
    assert_eq!(line, "res.body.echo neq {{token}}: actual \"[redacted]\"");
}

#[gpui_kit::test]
fn the_runner_row_redacts_a_secret_an_assertion_shows_as_actual(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, echo());
    let (_col, s) = echo_app(cx, &server.uri());
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("run-start", cx))
        .unwrap();
    cx.run_until_parked();
    let runner = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view");
    let model = runner.read_with(cx, |v, _| v.model().clone());
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    let why = label(cx, s.win, ("run-reason", 0usize));
    assert_eq!(why, "res.body.echo neq {{token}}: actual \"[redacted]\"");
}
