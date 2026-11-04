//! The response Headers, Cookies and Body tabs hide what the console hides, with an eye to show it.

use crate::support;

use std::sync::Arc;
use std::time::{Duration, Instant};

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
    view: Entity<ResponsePane>,
    _keep: (wiremock::MockServer, tempfile::TempDir, Started),
}

// A send whose response echoes the secret in a header, a cookie and the body `body`.
fn setup(cx: &mut TestAppContext, tab: ResponseTab, body: Vec<u8>, ct: &str) -> Fx {
    let response = ResponseTemplate::new(200)
        .insert_header("x-echo", SECRET)
        .insert_header("x-plain", "hello")
        .insert_header("set-cookie", format!("sess={SECRET}; Path=/").as_str())
        .append_header("set-cookie", "theme=dark; Path=/")
        .set_body_raw(body, ct);
    setup_answering(cx, tab, response)
}

// The same send, answered by `response`.
fn setup_answering(cx: &mut TestAppContext, tab: ResponseTab, response: ResponseTemplate) -> Fx {
    http_ready(cx);
    let server = mock_server(cx, response);
    let a = format!(
        r#"info:
  name: a
  type: http
  seq: 1
http:
  method: GET
  url: "{}/ok?token={{{{token}}}}"
"#,
        server.uri()
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
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model.clone(), w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view.update(cx, |p, cx| jar_tab(p, tab, cx));
    frame(cx, win);
    Fx {
        win,
        result,
        model,
        view,
        _keep: (server, col, s),
    }
}

fn json_body() -> Vec<u8> {
    format!(r#"{{"echo":"{SECRET}","n":1}}"#).into_bytes()
}

pub(crate) fn shown(cx: &mut TestAppContext, tab: ResponseTab) -> Fx {
    setup(cx, tab, json_body(), "application/json")
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

fn clipboard(cx: &mut TestAppContext) -> String {
    cx.read_from_clipboard().and_then(|c| c.text()).unwrap()
}

// The row of the entry whose name cell `{name_id}` reads `name`.
pub(crate) fn row_of(cx: &mut TestAppContext, fx: &Fx, name_id: &'static str, name: &str) -> usize {
    (0..40)
        .find(|&ix| present(cx, fx.win, (name_id, ix)) && label(cx, fx, (name_id, ix)) == name)
        .unwrap_or_else(|| panic!("no {name} row in {name_id}"))
}

#[gpui_kit::test]
fn a_secret_response_header_is_masked_until_the_eye_reveals_it(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Headers);
    let row = row_of(cx, &fx, "header-name", "x-echo");
    assert_eq!(label(cx, &fx, ("header-value", row)), "[redacted]");
    assert!(present(cx, fx.win, ("header-reveal", row)));
    click(cx, &fx, ("header-reveal", row));
    assert_eq!(label(cx, &fx, ("header-value", row)), SECRET);
    assert!(present(cx, fx.win, ("header-hide", row)));
    click(cx, &fx, ("header-hide", row));
    assert_eq!(label(cx, &fx, ("header-value", row)), "[redacted]");
}

#[gpui_kit::test]
fn a_plain_response_header_has_no_eye(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Headers);
    let row = row_of(cx, &fx, "header-name", "x-plain");
    assert_eq!(label(cx, &fx, ("header-value", row)), "hello");
    assert!(!present(cx, fx.win, ("header-reveal", row)));
}

#[gpui_kit::test]
fn the_headers_tab_and_the_console_hide_the_same_values(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Headers);
    let headers = fx.result.response.as_ref().unwrap().headers().to_vec();
    for (ix, (name, value)) in headers.iter().enumerate() {
        let shown = label(cx, &fx, ("header-value", ix));
        assert_eq!(shown, fx.result.redact_header(name, value), "{name}");
    }
}

#[gpui_kit::test]
fn header_copies_follow_the_eye(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Headers);
    let row = row_of(cx, &fx, "header-name", "x-echo");
    click(cx, &fx, ("header-copy", row));
    assert_eq!(clipboard(cx), "[redacted]");
    click(cx, &fx, "headers-copy");
    let all = clipboard(cx);
    assert!(!all.contains(SECRET), "{all}");
    assert!(all.contains("x-echo: [redacted]"), "{all}");
    assert!(all.contains("x-plain: hello"), "{all}");
    click(cx, &fx, ("header-reveal", row));
    click(cx, &fx, ("header-copy", row));
    assert_eq!(clipboard(cx), SECRET);
    click(cx, &fx, "headers-copy");
    assert!(clipboard(cx).contains(&format!("x-echo: {SECRET}")));
}

const URL_PASSWORD: &str = "pa55-xyz";

// A response naming urls whose userinfo holds `URL_PASSWORD`; nothing follows them.
fn url_headers_shown(cx: &mut TestAppContext) -> Fx {
    let url = format!("http://joe:{URL_PASSWORD}@127.0.0.1:9/ok");
    let response = ResponseTemplate::new(200)
        .insert_header("location", url.as_str())
        .insert_header("content-location", url.as_str())
        .insert_header("refresh", format!("0; url={url}").as_str());
    setup_answering(cx, ResponseTab::Headers, response)
}

#[gpui_kit::test]
fn a_url_header_password_is_masked_until_the_eye_reveals_it(cx: &mut TestAppContext) {
    let fx = url_headers_shown(cx);
    for name in ["location", "content-location", "refresh"] {
        let row = row_of(cx, &fx, "header-name", name);
        let shown = label(cx, &fx, ("header-value", row));
        let raw = fx.result.response.as_ref().unwrap().headers()[row]
            .1
            .clone();
        assert!(!shown.contains(URL_PASSWORD), "{name}: {shown}");
        assert_eq!(
            shown,
            apinata_core::mask_response_header(name, &raw),
            "{name}"
        );
        assert!(present(cx, fx.win, ("header-reveal", row)), "{name}");
    }
    let row = row_of(cx, &fx, "header-name", "location");
    assert_eq!(
        label(cx, &fx, ("header-value", row)),
        "http://127.0.0.1:9/ok"
    );
    click(cx, &fx, ("header-reveal", row));
    let raw = format!("http://joe:{URL_PASSWORD}@127.0.0.1:9/ok");
    assert_eq!(label(cx, &fx, ("header-value", row)), raw);
    click(cx, &fx, ("header-hide", row));
    assert_eq!(
        label(cx, &fx, ("header-value", row)),
        "http://127.0.0.1:9/ok"
    );
}

#[gpui_kit::test]
fn url_header_copies_hold_no_password_while_hidden(cx: &mut TestAppContext) {
    let fx = url_headers_shown(cx);
    for name in ["location", "content-location", "refresh"] {
        let row = row_of(cx, &fx, "header-name", name);
        click(cx, &fx, ("header-copy", row));
        let copied = clipboard(cx);
        assert!(!copied.contains(URL_PASSWORD), "{name}: {copied}");
    }
    click(cx, &fx, "headers-copy");
    let all = clipboard(cx);
    assert!(!all.contains(URL_PASSWORD), "{all}");
    assert!(all.contains("location: http://127.0.0.1:9/ok"), "{all}");
}

#[gpui_kit::test]
fn every_cookie_value_is_masked_until_its_eye_reveals_it(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Cookies);
    let sess = row_of(cx, &fx, "cookie-name", "sess");
    let theme = row_of(cx, &fx, "cookie-name", "theme");
    assert_eq!(label(cx, &fx, ("cookie-value", sess)), "[redacted]");
    assert_eq!(label(cx, &fx, ("cookie-value", theme)), "[redacted]");
    assert!(present(cx, fx.win, ("cookie-reveal", theme)));
    click(cx, &fx, ("cookie-reveal", sess));
    assert_eq!(label(cx, &fx, ("cookie-value", sess)), SECRET);
    assert_eq!(label(cx, &fx, ("cookie-value", theme)), "[redacted]");
    click(cx, &fx, ("cookie-hide", sess));
    assert_eq!(label(cx, &fx, ("cookie-value", sess)), "[redacted]");
    click(cx, &fx, ("cookie-reveal", theme));
    assert_eq!(label(cx, &fx, ("cookie-value", theme)), "dark");
}

// The console hides a Set-Cookie header whole, so the Headers tab and the Cookies tab agree.
#[gpui_kit::test]
fn the_cookies_and_headers_tabs_agree_about_a_set_cookie(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Headers);
    let row = row_of(cx, &fx, "header-name", "set-cookie");
    assert_eq!(label(cx, &fx, ("header-value", row)), "[redacted]");
    fx.view
        .update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    frame(cx, fx.win);
    for name in ["sess", "theme"] {
        let c = row_of(cx, &fx, "cookie-name", name);
        assert_eq!(label(cx, &fx, ("cookie-value", c)), "[redacted]", "{name}");
    }
}

#[gpui_kit::test]
fn cookie_copies_follow_the_eye(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Cookies);
    let sess = row_of(cx, &fx, "cookie-name", "sess");
    click(cx, &fx, ("cookie-copy", sess));
    assert_eq!(clipboard(cx), "[redacted]");
    click(cx, &fx, "cookies-copy");
    let all = clipboard(cx);
    assert!(!all.contains(SECRET), "{all}");
    assert!(all.contains("sess=[redacted]"), "{all}");
    click(cx, &fx, ("cookie-reveal", sess));
    click(cx, &fx, ("cookie-copy", sess));
    assert_eq!(clipboard(cx), SECRET);
    click(cx, &fx, "cookies-copy");
    assert!(clipboard(cx).contains(&format!("sess={SECRET}")));
}

// What the body view copies, which is what its editor or rows show.
fn body_copy(cx: &mut TestAppContext, fx: &Fx) -> String {
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    cx.update_window(fx.win, |_, w, cx| body.update(cx, |b, cx| b.copy(w, cx)))
        .unwrap();
    cx.run_until_parked();
    clipboard(cx)
}

fn editor_text(cx: &mut TestAppContext, fx: &Fx) -> String {
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    body.read_with(cx, |b, cx| b.editor().read(cx).value().to_string())
}

fn mask_runs(cx: &mut TestAppContext, fx: &Fx) -> usize {
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    body.read_with(cx, |b, _| b.mask_runs())
}

#[gpui_kit::test]
fn the_body_is_masked_until_its_eye_reveals_it(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    frame(cx, fx.win);
    assert!(!editor_text(cx, &fx).contains(SECRET));
    assert!(editor_text(cx, &fx).contains("[redacted]"));
    assert!(!body_copy(cx, &fx).contains(SECRET));
    assert!(present(cx, fx.win, "body-reveal"));
    click(cx, &fx, "body-reveal");
    cx.run_until_parked();
    frame(cx, fx.win);
    assert!(editor_text(cx, &fx).contains(SECRET));
    assert!(body_copy(cx, &fx).contains(SECRET));
    assert!(present(cx, fx.win, "body-hide"));
    click(cx, &fx, "body-hide");
    cx.run_until_parked();
    assert!(!editor_text(cx, &fx).contains(SECRET));
}

#[gpui_kit::test]
fn the_pretty_and_raw_views_format_the_masked_text(cx: &mut TestAppContext) {
    use apinata_app::body_view::BodyMode;
    let fx = shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    assert_eq!(
        editor_text(cx, &fx),
        "{\n  \"echo\": \"[redacted]\",\n  \"n\": 1\n}"
    );
    cx.update_window(fx.win, |_, w, cx| {
        body.update(cx, |b, cx| b.set_mode(BodyMode::Raw, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(editor_text(cx, &fx), r#"{"echo":"[redacted]","n":1}"#);
}

#[gpui_kit::test]
fn the_body_search_runs_on_the_shown_text(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    let find = |cx: &mut TestAppContext, q: &str| {
        cx.update_window(fx.win, |_, w, cx| {
            body.update(cx, |b, cx| {
                b.open_find(w, cx);
                b.set_find_query(q, w, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, fx.win);
        body.read_with(cx, |b, cx| b.find_label(cx))
    };
    assert_eq!(find(cx, "s3cr3t"), "No matches");
    assert_eq!(find(cx, "redacted"), "1 of 1");
    click(cx, &fx, "body-reveal");
    cx.run_until_parked();
    assert_eq!(find(cx, "s3cr3t"), "1 of 1");
}

#[gpui_kit::test]
fn a_binary_body_is_not_masked(cx: &mut TestAppContext) {
    let mut bytes = vec![0xff, 0xfe, 0x00, 0x80];
    bytes.extend_from_slice(SECRET.as_bytes());
    let fx = setup(cx, ResponseTab::Body, bytes, "application/octet-stream");
    cx.run_until_parked();
    frame(cx, fx.win);
    assert!(!present(cx, fx.win, "body-reveal"));
    assert!(!present(cx, fx.win, "body-hide"));
    assert_eq!(mask_runs(cx, &fx), 0);
}

#[gpui_kit::test]
fn a_body_without_a_secret_has_no_eye(cx: &mut TestAppContext) {
    let fx = setup(
        cx,
        ResponseTab::Body,
        b"{\"n\":1}".to_vec(),
        "application/json",
    );
    cx.run_until_parked();
    frame(cx, fx.win);
    assert!(!present(cx, fx.win, "body-reveal"));
}

#[gpui_kit::test]
fn the_body_is_masked_once_per_result(cx: &mut TestAppContext) {
    use apinata_app::body_view::BodyMode;
    let fx = shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    assert_eq!(mask_runs(cx, &fx), 1);
    // Renders, the eye, the mode switch and a tab round trip reuse the cached text.
    for _ in 0..5 {
        frame(cx, fx.win);
    }
    click(cx, &fx, "body-reveal");
    cx.run_until_parked();
    click(cx, &fx, "body-hide");
    cx.run_until_parked();
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    cx.update_window(fx.win, |_, w, cx| {
        body.update(cx, |b, cx| b.set_mode(BodyMode::Raw, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    fx.view
        .update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    frame(cx, fx.win);
    fx.view.update(cx, |p, cx| p.set_tab(ResponseTab::Body, cx));
    frame(cx, fx.win);
    assert_eq!(mask_runs(cx, &fx), 1);
    // A new result is masked afresh.
    let (root, model) = (fx._keep.2.root.clone(), fx.model.clone());
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let again = finished(cx, &model);
    assert!(!Arc::ptr_eq(&again, &fx.result));
    cx.run_until_parked();
    frame(cx, fx.win);
    assert_eq!(mask_runs(cx, &fx), 1);
}

#[gpui_kit::test]
fn reveals_are_forgotten_when_another_result_is_shown(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Headers);
    let row = row_of(cx, &fx, "header-name", "x-echo");
    click(cx, &fx, ("header-reveal", row));
    assert_eq!(label(cx, &fx, ("header-value", row)), SECRET);
    fx.view.update(cx, |p, cx| p.set_tab(ResponseTab::Body, cx));
    cx.run_until_parked();
    click(cx, &fx, "body-reveal");
    cx.run_until_parked();
    let (root, model) = (fx._keep.2.root.clone(), fx.model.clone());
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    finished(cx, &model);
    cx.run_until_parked();
    frame(cx, fx.win);
    assert!(!editor_text(cx, &fx).contains(SECRET));
    assert!(present(cx, fx.win, "body-reveal"));
    fx.view
        .update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    frame(cx, fx.win);
    let row = row_of(cx, &fx, "header-name", "x-echo");
    assert_eq!(label(cx, &fx, ("header-value", row)), "[redacted]");
}

#[gpui_kit::test]
fn a_large_body_is_masked_off_the_render_path(cx: &mut TestAppContext) {
    // Several megabytes of lines, past the editor limit, with the secret on every tenth.
    let body: String = (0..120_000)
        .map(|i| {
            if i % 10 == 0 {
                format!("line {i} token={SECRET}\n")
            } else {
                format!("line {i} plain text\n")
            }
        })
        .collect();
    assert!(body.len() > 2_000_000);
    let fx = setup(cx, ResponseTab::Body, body.into_bytes(), "text/plain");
    let started = Instant::now();
    cx.run_until_parked();
    assert_eq!(mask_runs(cx, &fx), 1);
    let copied = body_copy(cx, &fx);
    assert!(!copied.contains(SECRET));
    assert!(copied.contains("line 0 token=[redacted]"));
    // Frames after the first never redact again, so each stays quick.
    let frames = Instant::now();
    for _ in 0..10 {
        frame(cx, fx.win);
    }
    assert!(
        frames.elapsed() < Duration::from_secs(10),
        "{:?}",
        frames.elapsed()
    );
    assert_eq!(mask_runs(cx, &fx), 1);
    eprintln!("large body masked in {:?}", started.elapsed());
}

// A JSON key can be a secret, and the generated schema names its keys.
#[gpui_kit::test]
fn the_schema_test_is_built_from_the_masked_body(cx: &mut TestAppContext) {
    use apinata_app::body_view::SchemaTest;
    let body = format!(r#"{{"{SECRET}":1,"n":2}}"#).into_bytes();
    let fx = setup(cx, ResponseTab::Body, body, "application/json");
    cx.run_until_parked();
    frame(cx, fx.win);
    let got: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let sink = got.clone();
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    let sub = cx.update(|cx| {
        cx.subscribe(&body, move |_, e: &SchemaTest, _| {
            sink.borrow_mut().push(e.0.clone())
        })
    });
    std::mem::forget(sub);
    click(cx, &fx, "body-schema-test");
    cx.run_until_parked();
    assert!(!got.borrow()[0].contains(SECRET), "{:?}", got.borrow());
    assert!(got.borrow()[0].contains("[redacted]"));
    click(cx, &fx, "body-reveal");
    cx.run_until_parked();
    click(cx, &fx, "body-schema-test");
    cx.run_until_parked();
    assert!(got.borrow()[1].contains(SECRET));
}

#[gpui_kit::test]
fn the_json_filter_runs_on_the_masked_body(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    cx.update_window(fx.win, |_, w, cx| {
        body.update(cx, |b, cx| b.set_filter("$.echo", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!editor_text(cx, &fx).contains(SECRET));
    assert!(editor_text(cx, &fx).contains("[redacted]"));
    assert!(!body_copy(cx, &fx).contains(SECRET));
    click(cx, &fx, "body-reveal");
    cx.run_until_parked();
    assert!(editor_text(cx, &fx).contains(SECRET));
}

fn type_into(cx: &mut TestAppContext, fx: &Fx, text: &str) {
    use gpui_kit::Focusable as _;
    let body = fx.view.read_with(cx, |p, _| p.body().clone());
    let ed = body.read_with(cx, |b, _| b.editor().clone());
    cx.update_window(fx.win, |_, w, cx| {
        ed.read(cx).focus_handle(cx).focus(w, cx);
        w.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn body_of(cx: &mut TestAppContext, fx: &Fx) -> Entity<apinata_app::body_view::BodyView> {
    fx.view.read_with(cx, |p, _| p.body().clone())
}

#[gpui_kit::test]
fn a_masked_body_takes_no_edit_and_a_revealed_one_saves_the_real_text(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    frame(cx, fx.win);
    let body = body_of(cx, &fx);
    assert!(!present(cx, fx.win, "body-edit"));
    cx.update_window(fx.win, |_, w, cx| {
        body.update(cx, |b, cx| b.set_editing(true, w, cx))
    })
    .unwrap();
    assert!(!body.read_with(cx, |b, _| b.editing()));
    let masked = editor_text(cx, &fx);
    type_into(cx, &fx, "x");
    assert_eq!(editor_text(cx, &fx), masked);
    click(cx, &fx, "body-reveal");
    cx.run_until_parked();
    frame(cx, fx.win);
    click(cx, &fx, "body-edit");
    type_into(cx, &fx, "x");
    let edited = editor_text(cx, &fx);
    assert!(edited.contains(SECRET) && edited.contains('x'), "{edited}");
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("body.json");
    click(cx, &fx, "body-save");
    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), edited);
    // Hiding again drops the edit mode, so a masked body is never edited.
    click(cx, &fx, "body-hide");
    answer_confirm(cx, "confirm-discard");
    assert!(!body.read_with(cx, |b, _| b.editing()));
    assert!(!editor_text(cx, &fx).contains(SECRET));
}

// The entries of the node menu for the node whose shown text starts with `needle`.
fn entries_at(
    cx: &mut TestAppContext,
    fx: &Fx,
    needle: &str,
) -> Vec<apinata_app::body_node::NodeCopy> {
    let body = body_of(cx, fx);
    body.update(cx, |b, _| b.set_assert_offered(true));
    let text = editor_text(cx, fx);
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle} in {text}"));
    body.read_with(cx, |b, cx| b.node_copies(at, cx))
}

fn entry<'a>(
    entries: &'a [apinata_app::body_node::NodeCopy],
    label: &str,
) -> &'a apinata_app::body_node::NodeCopy {
    entries.iter().find(|e| e.label == label).unwrap()
}

#[gpui_kit::test]
fn node_actions_on_a_masked_value_keep_the_secret_out_of_files(cx: &mut TestAppContext) {
    use apinata_app::body_node::{ASSERT_VALUE, SET_VARIABLE};
    let fx = shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    frame(cx, fx.win);
    let secret = entries_at(cx, &fx, "[redacted]");
    // Copy value follows the reveal state, like Copy body.
    assert_eq!(entry(&secret, "Copy value").text, "[redacted]");
    // Set as variable takes the real value and says it was secret.
    let set = entry(&secret, SET_VARIABLE);
    assert_eq!(set.text, SECRET);
    assert!(set.sensitive);
    // Assert would write the secret into a git-tracked file, so it is disabled with a reason.
    assert!(entry(&secret, ASSERT_VALUE).locked.is_some());
    // A plain node behaves as before.
    let plain = entries_at(cx, &fx, "1");
    assert_eq!(entry(&plain, SET_VARIABLE).text, "1");
    assert!(!entry(&plain, SET_VARIABLE).sensitive);
    assert!(entry(&plain, ASSERT_VALUE).locked.is_none());
    click(cx, &fx, "body-reveal");
    cx.run_until_parked();
    frame(cx, fx.win);
    let revealed = entries_at(cx, &fx, SECRET);
    assert_eq!(entry(&revealed, "Copy value").text, SECRET);
    assert!(!entry(&revealed, SET_VARIABLE).sensitive);
    assert!(entry(&revealed, ASSERT_VALUE).locked.is_none());
}

#[gpui_kit::test]
fn a_secret_value_is_set_as_a_sensitive_runtime_variable(cx: &mut TestAppContext) {
    use apinata_app::body_node::SET_VARIABLE;
    let fx = shown(cx, ResponseTab::Body);
    cx.run_until_parked();
    frame(cx, fx.win);
    let entries = entries_at(cx, &fx, "[redacted]");
    let body = body_of(cx, &fx);
    let set = entry(&entries, SET_VARIABLE).clone();
    cx.update_window(fx.win, |_, w, cx| {
        body.update(cx, |b, cx| b.activate_node(&set, w, cx))
    })
    .unwrap();
    frame(cx, fx.win);
    assert!(present(cx, fx.win, "set-variable-dialog"));
    // The dialog never shows the secret, and only the runtime store is offered.
    assert_eq!(label(cx, &fx, "set-variable-value"), "[redacted]");
    click(cx, &fx, "set-variable-set");
    cx.run_until_parked();
    let (ws, root) = (fx._keep.2.ws.clone(), fx._keep.2.root.clone());
    let vars = ws.read_with(cx, |w, cx| w.runtime_vars(&root, cx)).unwrap();
    let got = vars.entries();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].name(), "echo");
    assert_eq!(got[0].value(), Some(SECRET));
    assert!(vars.is_sensitive("echo"));
}

// A body view over a result with no secret, shown but not yet prepared.
#[gpui_kit::test]
fn the_schema_test_button_says_why_while_the_body_is_prepared(cx: &mut TestAppContext) {
    use apinata_app::body_view::BodyView;
    use apinata_core::{Outcome, Response};
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let response = Response::new(200)
        .with_header("content-type", "application/json")
        .with_text_body("{\"a\":1}");
    let r = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    ));
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    let pending = view.read_with(cx, |v, _| v.schema_test_lock());
    assert_eq!(pending, Some("Preparing the body"));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.schema_test_lock()), None);
}

// A body view of its own in a window, for tests that move between results.
fn own_body(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<apinata_app::body_view::BodyView>) {
    use apinata_app::body_view::BodyView;
    support::init(cx);
    cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
            cx.new(|cx| BodyView::new(w, cx))
        })
    })
    .unwrap()
}

fn show_in(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    body: &Entity<apinata_app::body_view::BodyView>,
    r: Arc<RequestResult>,
) {
    cx.update_window(win, |_, w, cx| {
        body.update(cx, |b, cx| b.show(Some(r), w, cx))
    })
    .unwrap();
}

fn type_in(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    body: &Entity<apinata_app::body_view::BodyView>,
) {
    use gpui_kit::Focusable as _;
    let ed = body.read_with(cx, |b, _| b.editor().clone());
    cx.update_window(win, |_, w, cx| {
        ed.read(cx).focus_handle(cx).focus(w, cx);
        w.input("x", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn plain_result() -> Arc<RequestResult> {
    use apinata_core::{Outcome, Response};
    let response = Response::new(200)
        .with_header("content-type", "application/json")
        .with_text_body("{\"a\":1}");
    Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    ))
}

#[gpui_kit::test]
fn editing_one_result_does_not_leave_a_masked_one_writable(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Body);
    let (win, body) = own_body(cx);
    show_in(cx, win, &body, plain_result());
    cx.run_until_parked();
    body.update(cx, |b, _| b.set_editable(true));
    cx.update_window(win, |_, w, cx| {
        body.update(cx, |b, cx| b.set_editing(true, w, cx))
    })
    .unwrap();
    assert!(body.read_with(cx, |b, _| b.editing()));
    show_in(cx, win, &body, fx.result.clone());
    cx.run_until_parked();
    frame(cx, win);
    assert!(body.read_with(cx, |b, _| b.masked()));
    assert!(!body.read_with(cx, |b, _| b.editing()));
    let before = body.read_with(cx, |b, cx| b.editor().read(cx).value().to_string());
    type_in(cx, win, &body);
    assert_eq!(
        body.read_with(cx, |b, cx| b.editor().read(cx).value().to_string()),
        before
    );
}

#[gpui_kit::test]
fn a_mask_that_lands_after_edit_was_chosen_makes_the_body_read_only(cx: &mut TestAppContext) {
    let fx = shown(cx, ResponseTab::Body);
    let (win, body) = own_body(cx);
    body.update(cx, |b, _| b.set_editable(true));
    show_in(cx, win, &body, fx.result.clone());
    // Edit is chosen before the redacted copy exists.
    cx.update_window(win, |_, w, cx| {
        body.update(cx, |b, cx| b.set_editing(true, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    assert!(body.read_with(cx, |b, _| b.masked()));
    assert!(!body.read_with(cx, |b, _| b.editing()));
    let before = body.read_with(cx, |b, cx| b.editor().read(cx).value().to_string());
    type_in(cx, win, &body);
    assert_eq!(
        body.read_with(cx, |b, cx| b.editor().read(cx).value().to_string()),
        before
    );
}
