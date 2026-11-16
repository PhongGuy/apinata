//! A response sent with a JSON body that does not parse carries a muted note naming where the
//! body broke; a valid body, a text body with the same bytes and the next valid send show none.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;
use wiremock::matchers::path;

const NOTE: &str = "response-sent-json";
const INVALID: &str = "{\"a\": 1,}";

// A started app whose `a.yml` posts `data` as a `kind` body to a local server answering 200.
fn posting(cx: &mut TestAppContext, kind: &str, data: &str) -> (Sent, wiremock::MockServer) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        wiremock::Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let body = format!("  body:\n    type: {kind}\n    data: '{data}'\n");
    let col = collection_for(&server.uri(), &body);
    let s = started(cx, &col);
    std::mem::forget(col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    (Sent { app: s, tab }, server)
}

struct Sent {
    app: Started,
    tab: apinata_app::tabs::TabId,
}

fn editor(
    cx: &mut TestAppContext,
    s: &Sent,
) -> gpui_kit::Entity<apinata_app::request_editor::RequestEditor> {
    editor_of(cx, &s.app, s.tab)
}

// Clicks Send and waits until that send has finished.
fn send(cx: &mut TestAppContext, s: &Sent) {
    let model = editor(cx, s).read_with(cx, |e, _| e.send_model().clone());
    cx.update_window(s.app.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| !model.read_with(cx, |m, _| m.is_sending()));
    frame(cx, s.app.win);
    assert!(
        present(cx, s.app.win, "response-status"),
        "the send got a response"
    );
}

fn type_body(cx: &mut TestAppContext, s: &Sent, text: &str) {
    let code = editor(cx, s).read_with(cx, |e, cx| e.body().read(cx).code().clone());
    cx.update_window(s.app.win, |_, w, cx| {
        code.update(cx, |c, cx| c.replace_all(text.to_string(), w, cx))
    })
    .unwrap();
    frame(cx, s.app.win);
}

fn note(cx: &mut TestAppContext, s: &Sent) -> Option<String> {
    cx.update_window(s.app.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(NOTE).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn an_invalid_json_body_sent_shows_where_it_broke(cx: &mut TestAppContext) {
    let (s, _server) = posting(cx, "json", INVALID);
    send(cx, &s);
    assert_eq!(
        note(cx, &s).as_deref(),
        Some("The body sent was not valid JSON: line 1, column 9.")
    );
}

#[gpui_kit::test]
fn a_valid_json_body_sent_shows_no_note(cx: &mut TestAppContext) {
    let (s, _server) = posting(cx, "json", "{\"a\": 1}");
    send(cx, &s);
    assert_eq!(note(cx, &s), None);
}

#[gpui_kit::test]
fn a_text_body_with_the_same_bytes_shows_no_note(cx: &mut TestAppContext) {
    let (s, _server) = posting(cx, "text", INVALID);
    send(cx, &s);
    assert_eq!(note(cx, &s), None);
}

// The note follows the body each send carried, so an edit made in the editor counts.
#[gpui_kit::test]
fn the_note_follows_the_body_each_send_carried(cx: &mut TestAppContext) {
    let (s, _server) = posting(cx, "json", "{\"a\": 1}");
    type_body(cx, &s, "{\n\"b\": }");
    send(cx, &s);
    assert_eq!(
        note(cx, &s).as_deref(),
        Some("The body sent was not valid JSON: line 2, column 6.")
    );
    type_body(cx, &s, "{\n\"b\": 1}");
    send(cx, &s);
    assert_eq!(note(cx, &s), None, "the next valid send clears the note");
}
