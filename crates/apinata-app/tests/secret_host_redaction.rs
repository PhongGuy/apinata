//! A secret used as the request host is redacted in the friendly error sentence of the
//! response pane's error block and of the runner's row reasons.

use crate::support;

use std::sync::Arc;

use apinata_app::response_pane::ResponsePane;
use apinata_app::runner_model::RunState;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use support::*;

const SECRET_HOST: &str = "127.0.0.1";

// `a.yml` dials a closed port on the secret host, so the run fails with a refused connection.
fn secret_host_app(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let dev = "name: dev\nvariables:\n  - name: host\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &request("a", "http://{{host}}:1/")),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "host"), SECRET_HOST).unwrap();
    let s = started_on(cx, &col, |_| {}, Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    (col, s)
}

#[gpui_kit::test]
fn the_error_block_sentence_redacts_a_secret_host(cx: &mut TestAppContext) {
    http_ready(cx);
    let (_col, s) = secret_host_app(cx);
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    support::init(cx);
    let (_win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    let error = view
        .read_with(cx, |p, cx| p.error_view(cx))
        .expect("an error block");
    assert_eq!(error.title, "Connection refused");
    let sentence = error.sentence.expect("a sentence");
    assert!(sentence.contains("[redacted]"), "{sentence}");
    assert!(!sentence.contains(SECRET_HOST), "{sentence}");
}

#[gpui_kit::test]
fn a_runner_row_redacts_a_secret_host(cx: &mut TestAppContext) {
    http_ready(cx);
    let (_col, s) = secret_host_app(cx);
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
    let why = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("run-reason", 0usize))
                .label()
                .unwrap_or_default()
                .to_owned()
        })
        .unwrap();
    assert!(why.starts_with("Connection refused: "), "{why}");
    assert!(why.contains("[redacted]"), "{why}");
    assert!(!why.contains(SECRET_HOST), "{why}");
}
