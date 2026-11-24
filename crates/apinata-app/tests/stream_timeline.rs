//! A stream's Timeline shows the hops before its final response while the stream is open.

use crate::support;

use std::sync::Arc;
use std::time::Duration;

use apinata_app::response_pane::{ResponsePane, ResponseTab};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, TestAppContext, WindowOptions};
use support::sse_server::{Script, Step, event};
use support::*;

const SECRET: &str = "s3cr3t-live-hop-9";

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_redirected_stream_shows_its_masked_redirect_hop_before_it_ends(cx: &mut TestAppContext) {
    http_ready(cx);
    // `/hop` answers 302 and closes; `/s` streams one event and stays open for a while.
    let redirect = "HTTP/1.1 302 Found\r\nlocation: /s\r\ncontent-length: 0\r\n\
                    connection: close\r\n\r\n";
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(sse_server::routed_sse_server(
        vec![(
            "/hop",
            Script::raw_head(redirect.to_owned(), vec![Step::Abort]),
        )],
        Script::sse(vec![
            event("one"),
            Step::Pause(Duration::from_secs(3)),
            Step::Finish,
        ]),
    ));
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}?key={{{{token}}}}\"\n  headers:\n    - name: authorization\n      value: Bearer {{{{token}}}}\n",
        server.url("/hop")
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), SECRET).unwrap();
    let s = started_on(
        cx,
        &col,
        |p| apinata_app::persist::save_json(&p.settings, &test_settings()).unwrap(),
        Arc::new(secrets),
    );
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    support::init(cx);
    let m = model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| m.live().is_some_and(|l| l.events().len() == 1))
    });
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    assert!(model.read_with(cx, |m, _| m.is_streaming()));

    assert!(
        present(cx, win, ("hop", 0usize)),
        "the redirect hop shows while the stream is open"
    );
    let line = label(cx, win, "hop-line");
    assert!(line.starts_with("GET "), "{line}");
    assert!(line.contains(" 302"), "{line}");
    assert!(!line.contains(SECRET), "{line}");
    let row = (0..20usize)
        .find(|&ix| {
            present(cx, win, ("hop-request-header-name", ix))
                && label(cx, win, ("hop-request-header-name", ix)) == "authorization"
        })
        .expect("the redirect hop shows its authorization header");
    assert_eq!(
        label(cx, win, ("hop-request-header-value", row)),
        "[redacted]"
    );
    assert!(!present(cx, win, ("hop-request-header-reveal", row)));

    model.update(cx, |m, cx| m.cancel(cx));
    finished(cx, &model);
}
