//! The Timeline hides what the console hides for the same result, with an eye to reveal it.

use crate::support;

use std::sync::Arc;

use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_core::RequestResult;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext, WindowOptions,
};
use support::*;
use wiremock::ResponseTemplate;

const SECRET: &str = "s3cr3t-value-123";

// The pane window, the shown result and what keeps the send's server and files alive.
pub(crate) struct Fx {
    pub(crate) win: AnyWindowHandle,
    result: Arc<RequestResult>,
    model: Entity<apinata_app::send_model::SendModel>,
    _keep: (
        wiremock::MockServer,
        tempfile::TempDir,
        Started,
        Entity<ResponsePane>,
    ),
}

// A POST whose url, header and body use the secret environment variable `token`, next to a
// header of plain values. The server echoes the secret in a response header.
pub(crate) fn shown(cx: &mut TestAppContext) -> Fx {
    setup(cx, false, "")
}

// As `shown`, with literal credentials under credential header names that no secret list knows.
pub(crate) fn shown_named(cx: &mut TestAppContext) -> Fx {
    let extra = "    - name: X-API-Key\n      value: lit-api-key-1\n    - name: x-session-token\n      value: lit-session-2\n";
    setup(cx, false, extra)
}

// With `pac`, the send goes through a PAC route naming the secret, which here is the proxy's
// own host, and the mock server answers as the proxy.
fn setup(cx: &mut TestAppContext, pac: bool, extra: &str) -> Fx {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200)
            .insert_header("x-echo", SECRET)
            .insert_header("www-authenticate", "Negotiate dG9rZW4="),
    );
    let (secret, base) = if pac {
        (
            "127.0.0.1",
            format!("http://app.test:{}", server.address().port()),
        )
    } else {
        (SECRET, server.uri())
    };
    let pac_file = tempfile::tempdir().unwrap();
    let mut settings = test_settings();
    if pac {
        let path = pac_file.path().join("proxy.pac");
        let route = format!(
            "function FindProxyForURL(u, h) {{ return 'PROXY 127.0.0.1:{}'; }}",
            server.address().port()
        );
        std::fs::write(&path, route).unwrap();
        settings.proxy.mode = apinata_app::settings::ProxyMode::Pac;
        settings.proxy.pac_source = path.to_str().unwrap().into();
    }
    let a = format!(
        r#"info:
  name: a
  type: http
  seq: 1
http:
  method: POST
  url: "{}/ok?token={{{{token}}}}"
  headers:
    - name: x-auth
      value: "Bearer {{{{token}}}}"
    - name: x-plain
      value: hello
    - name: authorization
      value: Negotiate dG9rZW4=
    - name: proxy-authorization
      value: Basic dXNlcjpwdw==
{}  body:
    type: json
    data: '{{"k":"{{{{token}}}}"}}'
"#,
        base, extra
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), secret).unwrap();
    let s = started_on(
        cx,
        &col,
        |p| apinata_app::persist::save_json(&p.settings, &settings).unwrap(),
        Arc::new(secrets),
    );
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    // Keeps the PAC script's temp dir alive until the end of the test.
    let _keep_pac = pac_file;
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model.clone(), w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    frame(cx, win);
    Fx {
        win,
        result,
        model,
        _keep: (server, col, s, view),
    }
}

pub(crate) fn label(cx: &mut TestAppContext, fx: &Fx, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(fx.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, fx: &Fx, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(fx.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    frame(cx, fx.win);
}

// The row of the header `name` in the table whose cells are `{prefix}-name` and `{prefix}-value`.
pub(crate) fn row_of(cx: &mut TestAppContext, fx: &Fx, prefix: &'static str, name: &str) -> usize {
    let name_id: &'static str = Box::leak(format!("{prefix}-name").into_boxed_str());
    (0..40)
        .find(|&ix| present(cx, fx.win, (name_id, ix)) && label(cx, fx, (name_id, ix)) == name)
        .unwrap_or_else(|| panic!("no {name} row in {prefix}"))
}

fn value_id(prefix: &'static str) -> &'static str {
    Box::leak(format!("{prefix}-value").into_boxed_str())
}

const REQ: &str = "hop-request-header";
const RESP: &str = "hop-response-header";

#[gpui_kit::test]
fn a_secret_request_header_is_masked_until_the_eye_reveals_it(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let row = row_of(cx, &fx, REQ, "x-auth");
    let value = value_id(REQ);
    assert_eq!(label(cx, &fx, (value, row)), "Bearer [redacted]");
    assert!(present(cx, fx.win, ("hop-request-header-reveal", row)));
    click(cx, &fx, ("hop-request-header-reveal", row));
    assert_eq!(label(cx, &fx, (value, row)), format!("Bearer {SECRET}"));
    assert!(present(cx, fx.win, ("hop-request-header-hide", row)));
    click(cx, &fx, ("hop-request-header-hide", row));
    assert_eq!(label(cx, &fx, (value, row)), "Bearer [redacted]");
}

#[gpui_kit::test]
fn a_plain_request_header_shows_without_an_eye(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let row = row_of(cx, &fx, REQ, "x-plain");
    assert_eq!(label(cx, &fx, (value_id(REQ), row)), "hello");
    assert!(!present(cx, fx.win, ("hop-request-header-reveal", row)));
    assert!(!present(cx, fx.win, ("hop-request-header-hide", row)));
}

#[gpui_kit::test]
fn the_timeline_and_the_console_hide_the_same_header_values(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let hop = &fx.result.response.as_ref().unwrap().timeline()[0];
    for (ix, (name, value)) in hop.request_headers.iter().enumerate() {
        let console = fx.result.redact_header(name, value);
        let shown = label(cx, &fx, (value_id(REQ), ix));
        assert_eq!(shown, console, "{name}");
        assert!(!shown.contains(SECRET), "{name}");
    }
}

#[gpui_kit::test]
fn a_secret_in_the_url_is_masked_in_the_hop_line(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let line = label(cx, &fx, "hop-line");
    assert!(!line.contains(SECRET), "{line}");
    assert!(line.contains("[redacted]"), "{line}");
    click(cx, &fx, "hop-line-reveal");
    assert!(label(cx, &fx, "hop-line").contains(SECRET));
    click(cx, &fx, "hop-line-hide");
    assert!(!label(cx, &fx, "hop-line").contains(SECRET));
}

#[gpui_kit::test]
fn a_response_header_echoing_the_secret_is_masked(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let row = row_of(cx, &fx, RESP, "x-echo");
    assert_eq!(label(cx, &fx, (value_id(RESP), row)), "[redacted]");
    click(cx, &fx, ("hop-response-header-reveal", row));
    assert_eq!(label(cx, &fx, (value_id(RESP), row)), SECRET);
}

#[gpui_kit::test]
fn a_request_body_with_the_secret_is_masked(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let body = label(cx, &fx, "hop-request-body");
    assert!(!body.contains(SECRET), "{body}");
    assert!(body.contains("[redacted]"), "{body}");
    click(cx, &fx, "hop-request-body-reveal");
    assert!(label(cx, &fx, "hop-request-body").contains(SECRET));
    click(cx, &fx, "hop-request-body-hide");
    assert!(!label(cx, &fx, "hop-request-body").contains(SECRET));
}

#[gpui_kit::test]
fn a_handshake_credential_stays_masked_and_has_no_eye(cx: &mut TestAppContext) {
    let fx = shown(cx);
    // The console hides these two whole, and the Timeline reads the same.
    for (name, value) in [
        ("authorization", "Negotiate dG9rZW4="),
        ("proxy-authorization", "Basic dXNlcjpwdw=="),
    ] {
        let row = row_of(cx, &fx, REQ, name);
        let shown = label(cx, &fx, (value_id(REQ), row));
        assert_eq!(shown, fx.result.redact_header(name, value));
        assert_eq!(shown, "[redacted]");
        assert!(!present(cx, fx.win, ("hop-request-header-reveal", row)));
    }
    let row = row_of(cx, &fx, RESP, "www-authenticate");
    assert_eq!(
        label(cx, &fx, (value_id(RESP), row)),
        "Negotiate [redacted]"
    );
    assert!(!present(cx, fx.win, ("hop-response-header-reveal", row)));
}

// The eye sits inside its value's cell and the hop line's eye has room around it.
#[gpui_kit::test]
fn an_eye_follows_its_value(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let row = row_of(cx, &fx, REQ, "x-auth");
    cx.update_window(fx.win, |_, w, cx| {
        w.render_frame(cx);
        let cell = w.find((value_id(REQ), row)).bounds();
        let eye = w.find(("hop-request-header-reveal", row)).bounds();
        assert!(
            cell.left() <= eye.left() && eye.right() <= cell.right(),
            "{cell:?} {eye:?}"
        );
        let line = w.find("hop-line").bounds();
        let eye = w.find("hop-line-reveal").bounds();
        assert!(line.right() <= eye.left(), "{line:?} {eye:?}");
        let (hop, eye_ring) = (w.find(("hop", 0usize)).bounds(), eye);
        assert!(
            eye_ring.top() > hop.top() && eye_ring.right() < hop.right(),
            "{hop:?} {eye_ring:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_reveal_is_forgotten_when_another_result_is_shown(cx: &mut TestAppContext) {
    let fx = shown(cx);
    let row = row_of(cx, &fx, REQ, "x-auth");
    click(cx, &fx, ("hop-request-header-reveal", row));
    assert!(label(cx, &fx, (value_id(REQ), row)).contains(SECRET));
    let (root, model) = (fx._keep.2.root.clone(), fx.model.clone());
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let again = finished(cx, &model);
    assert!(!Arc::ptr_eq(&again, &fx.result));
    frame(cx, fx.win);
    let row = row_of(cx, &fx, REQ, "x-auth");
    assert_eq!(label(cx, &fx, (value_id(REQ), row)), "Bearer [redacted]");
}

#[gpui_kit::test]
fn a_pac_route_naming_a_secret_is_masked(cx: &mut TestAppContext) {
    let fx = setup(cx, true, "");
    let raw = fx.result.response.as_ref().unwrap().timeline()[0]
        .pac
        .clone()
        .expect("a PAC route");
    assert!(raw.contains("127.0.0.1"), "{raw}");
    let shown = label(cx, &fx, "hop-pac");
    assert_eq!(shown, format!("PAC: {}", fx.result.redact(&raw)));
    assert!(!shown.contains("127.0.0.1"), "{shown}");
}

// Clicks the eye of request header row `row`.
fn reveal_named(cx: &mut TestAppContext, fx: &Fx, row: usize) {
    click(cx, fx, ("hop-request-header-reveal", row));
}

#[gpui_kit::test]
fn a_credential_named_header_is_dots_until_its_eye_reveals_it(cx: &mut TestAppContext) {
    let fx = shown_named(cx);
    let value = value_id(REQ);
    for (name, literal) in [
        ("x-api-key", "lit-api-key-1"),
        ("x-session-token", "lit-session-2"),
    ] {
        let row = row_of(cx, &fx, REQ, name);
        assert_eq!(
            label(cx, &fx, (value, row)),
            apinata_app::response_pane::NAME_MASK
        );
        assert!(
            present(cx, fx.win, ("hop-request-header-reveal", row)),
            "{name}"
        );
        reveal_named(cx, &fx, row);
        assert_eq!(label(cx, &fx, (value, row)), literal);
        click(cx, &fx, ("hop-request-header-hide", row));
        assert_eq!(
            label(cx, &fx, (value, row)),
            apinata_app::response_pane::NAME_MASK
        );
    }
    // Other headers read as before.
    let row = row_of(cx, &fx, REQ, "x-plain");
    assert_eq!(label(cx, &fx, (value, row)), "hello");
}

#[gpui_kit::test]
fn a_credential_name_reveal_lasts_across_results(cx: &mut TestAppContext) {
    let fx = shown_named(cx);
    let row = row_of(cx, &fx, REQ, "x-api-key");
    reveal_named(cx, &fx, row);
    let (root, model) = (fx._keep.2.root.clone(), fx.model.clone());
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let again = finished(cx, &model);
    assert!(!Arc::ptr_eq(&again, &fx.result));
    frame(cx, fx.win);
    let row = row_of(cx, &fx, REQ, "x-api-key");
    assert_eq!(label(cx, &fx, (value_id(REQ), row)), "lit-api-key-1");
    // A per-result reveal is still forgotten.
    let auth = row_of(cx, &fx, REQ, "x-auth");
    assert_eq!(label(cx, &fx, (value_id(REQ), auth)), "Bearer [redacted]");
    // The session log keeps showing the literal as it always did.
    let hop = &again.response.as_ref().unwrap().timeline()[0];
    let (name, value) = (hop.request_headers.iter())
        .find(|(n, _)| n.eq_ignore_ascii_case("x-api-key"))
        .unwrap();
    assert_eq!(again.redact_header(name, value), "lit-api-key-1");
}

#[gpui_kit::test]
fn a_credential_name_reveal_stays_in_its_collection(cx: &mut TestAppContext) {
    let fx = shown_named(cx);
    let row = row_of(cx, &fx, REQ, "x-api-key");
    reveal_named(cx, &fx, row);
    assert_eq!(label(cx, &fx, (value_id(REQ), row)), "lit-api-key-1");
    let (root, model) = (fx._keep.2.root.clone(), fx.model.clone());
    let other = tempfile::tempdir().unwrap();
    model.update(cx, |m, cx| m.show_at(other.path(), fx.result.clone(), cx));
    frame(cx, fx.win);
    let row = row_of(cx, &fx, REQ, "x-api-key");
    assert_eq!(
        label(cx, &fx, (value_id(REQ), row)),
        apinata_app::response_pane::NAME_MASK
    );
    model.update(cx, |m, cx| m.show_at(&root, fx.result.clone(), cx));
    frame(cx, fx.win);
    let row = row_of(cx, &fx, REQ, "x-api-key");
    assert_eq!(label(cx, &fx, (value_id(REQ), row)), "lit-api-key-1");
}
