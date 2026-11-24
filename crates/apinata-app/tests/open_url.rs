//! Open URL as new request: a URL from the response body or its Location header becomes a GET
//! draft in the request's folder, never sent until Send.

use crate::support;

use apinata_app::body_view::{BodyView, Shown};
use apinata_app::follow_url::{
    BASE_QUERY, NO_URL, OPEN_URL, PLACEHOLDER, follow_target, same_origin, url_at,
};
use apinata_app::response_pane::ResponseTab;
use apinata_app::workspace::Workspace;
use apinata_core::{AuthSpec, ItemPath};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use support::*;

#[test]
fn the_url_at_the_caret_is_the_whole_http_url_around_it() {
    let text = r#"{"next": "https://api.test.invalid/items?page=2", "n": 1}"#;
    let inside = text.find("items").unwrap();
    assert_eq!(
        url_at(text, inside),
        Some("https://api.test.invalid/items?page=2")
    );
    let start = text.find("https").unwrap();
    assert_eq!(
        url_at(text, start),
        Some("https://api.test.invalid/items?page=2")
    );
    assert_eq!(url_at(text, text.find("\"n\"").unwrap()), None);
    let prose = "see http://127.0.0.1/a/b). Then";
    assert_eq!(
        url_at(prose, prose.find("/a").unwrap()),
        Some("http://127.0.0.1/a/b")
    );
    let link = "<HTTPS://x.invalid/p>; rel=\"next\"";
    assert_eq!(url_at(link, 3), Some("HTTPS://x.invalid/p"));
}

#[test]
fn only_http_and_https_urls_are_found_at_the_caret() {
    for text in [
        "ftp://x.invalid/f",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "plain words",
    ] {
        assert_eq!(url_at(text, 2), None, "{text}");
    }
}

#[test]
fn a_target_resolves_against_the_request_url_and_refuses_other_schemes() {
    let base = "http://127.0.0.1:9/users/1?x=1";
    assert_eq!(
        follow_target("/items/7", base).as_deref(),
        Ok("http://127.0.0.1:9/items/7")
    );
    assert_eq!(
        follow_target("2", base).as_deref(),
        Ok("http://127.0.0.1:9/users/2")
    );
    assert_eq!(
        follow_target("https://x.invalid/a?b=1", base).as_deref(),
        Ok("https://x.invalid/a?b=1")
    );
    for refused in [
        "ftp://x.invalid/f",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/plain,hi",
    ] {
        assert_eq!(follow_target(refused, base), Err(NO_URL), "{refused}");
    }
}

#[test]
fn a_target_holding_a_placeholder_is_refused() {
    let base = "http://127.0.0.1:9/users/1";
    for raw in [
        "https://attacker.test.invalid/?t={{token}}",
        "http://127.0.0.1:9/{{path}}",
        "/items?t={{token}}",
    ] {
        assert_eq!(follow_target(raw, base), Err(PLACEHOLDER), "{raw}");
    }
}

#[test]
fn a_relative_target_never_takes_the_request_urls_query() {
    let base = "http://127.0.0.1:9/users/1?api_key=SECRET";
    for raw in ["", "#frag"] {
        assert_eq!(follow_target(raw, base), Err(BASE_QUERY), "{raw:?}");
    }
    assert_eq!(
        follow_target("?page=2", base).as_deref(),
        Ok("http://127.0.0.1:9/users/1?page=2")
    );
    assert_eq!(
        follow_target("/next", base).as_deref(),
        Ok("http://127.0.0.1:9/next")
    );
}

#[test]
fn a_target_never_carries_userinfo() {
    let base = "http://u:pw@127.0.0.1:9/users/1";
    assert_eq!(
        follow_target("/next", base).as_deref(),
        Ok("http://127.0.0.1:9/next")
    );
    assert_eq!(
        follow_target("https://u:pw@x.invalid/a?b=1", base).as_deref(),
        Ok("https://x.invalid/a?b=1")
    );
}

#[test]
fn same_origin_compares_scheme_host_and_port() {
    let a = "http://127.0.0.1:9/a";
    assert!(same_origin(a, "http://127.0.0.1:9/b?c"));
    assert!(!same_origin(a, "https://127.0.0.1:9/a"));
    assert!(!same_origin(a, "http://127.0.0.1:10/a"));
    assert!(!same_origin(a, "http://x.invalid:9/a"));
    assert!(same_origin("http://x.invalid/a", "http://X.invalid:80/b"));
    assert!(same_origin(
        "https://x.invalid:443/a",
        "https://x.invalid/b"
    ));
}

// A collection whose `users/u.yml` carries a literal header and bearer token and answers with
// a JSON body of links and a relative Location; the server counts what reaches it.
struct Sent {
    s: Started,
    server: wiremock::MockServer,
    _front: Option<wiremock::MockServer>,
    body: Entity<BodyView>,
    pane: Entity<apinata_app::response_pane::ResponsePane>,
    uri: String,
    _col: tempfile::TempDir,
}

fn sent(cx: &mut TestAppContext) -> Sent {
    sent_via(cx, false)
}

// With `redirected`, the request goes to a front server that redirects to the answering one,
// so the URL first sent to and the URL that answered have different origins.
fn sent_via(cx: &mut TestAppContext, redirected: bool) -> Sent {
    use apinata_app::request_editor::SendRequest;
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    let uri = server.uri();
    let json = format!(
        r#"{{"next": "{uri}/page/2", "away": "https://away.test.invalid/x", "ftp": "ftp://x.invalid/f", "leak": "https://leak.test.invalid/?t={{{{token}}}}"}}"#
    );
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(201)
                    .insert_header("Location", "/items/7")
                    .set_body_raw(json, "application/json"),
            )
            .mount(&server),
    );
    let front = redirected.then(|| {
        let front = handle.block_on(wiremock::MockServer::builder().start());
        handle.block_on(
            wiremock::Mock::given(wiremock::matchers::any())
                .respond_with(
                    wiremock::ResponseTemplate::new(302)
                        .insert_header("Location", format!("{uri}/users/1").as_str()),
                )
                .mount(&front),
        );
        front
    });
    let first = front.as_ref().map_or(uri.clone(), |f| f.uri());
    let u = format!(
        "{}  headers:\n    - name: X-Team\n      value: t1\n  auth:\n    type: bearer\n    token: lit-token\n",
        request("u", &format!("{first}/users/1"))
    );
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("users")),
        ("users/u.yml", &u),
    ]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    let tab = open_tab(cx, &s.ws, &s.root, "users/u.yml");
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).editor(tab).cloned())
        .unwrap();
    let pane = ed.read_with(cx, |e, _| e.response().clone());
    let body = pane.read_with(cx, |p, _| p.body().clone());
    cx.update_window(s.win, |_, w, cx| {
        ed.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    wait_real(cx, |cx| {
        body.read_with(cx, |b, _| b.shown()) == Shown::Editor
    });
    Sent {
        s,
        server,
        _front: front,
        body,
        pane,
        uri,
        _col: col,
    }
}

fn received(cx: &mut TestAppContext, server: &wiremock::MockServer) -> usize {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle
        .block_on(server.received_requests())
        .unwrap_or_default()
        .len()
}

// Puts the body editor's caret on the first `needle` and runs the command from there.
fn open_at(cx: &mut TestAppContext, t: &Sent, needle: &str) {
    let editor = t.body.read_with(cx, |b, _| b.editor().clone());
    let offset = editor.read_with(cx, |e, _| e.value().find(needle).unwrap());
    editor.update(cx, |e, cx| e.set_selected_range(offset..offset, cx));
    cx.update_window(t.s.win, |_, w, cx| {
        t.body.update(cx, |b, cx| b.open_url_at_caret(w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, t.s.win);
}

fn active_draft(cx: &mut TestAppContext, ws: &Entity<Workspace>) -> Option<ItemPath> {
    ws.read_with(cx, |w, _| w.tabs().active().and_then(|e| e.item()).cloned())
        .filter(ItemPath::is_draft)
}

#[gpui_kit::test]
fn a_same_origin_url_opens_an_unsent_get_draft_with_the_requests_headers_and_auth(
    cx: &mut TestAppContext,
) {
    let t = sent(cx);
    assert_eq!(received(cx, &t.server), 1);
    open_at(cx, &t, "page/2");
    let item = active_draft(cx, &t.s.ws).expect("a draft tab is active");
    assert_eq!(item.parent(), Some(ip("users")), "in the request's folder");
    let view = view_of(cx, &t.s.ws, &t.s.root, item.as_str());
    assert_eq!(view.method, "GET");
    assert_eq!(view.url, format!("{}/page/2", t.uri));
    let headers: Vec<_> = (view.headers.iter())
        .map(|h| (h.name.as_str(), h.value.as_str()))
        .collect();
    assert_eq!(headers, [("X-Team", "t1")]);
    assert!(matches!(&view.auth, AuthSpec::Bearer { token } if token == "lit-token"));
    assert_eq!(received(cx, &t.server), 1, "nothing is sent");
}

#[gpui_kit::test]
fn another_origins_url_carries_no_headers_and_no_auth(cx: &mut TestAppContext) {
    let t = sent(cx);
    open_at(cx, &t, "away.test");
    let item = active_draft(cx, &t.s.ws).expect("a draft tab is active");
    let view = view_of(cx, &t.s.ws, &t.s.root, item.as_str());
    assert_eq!(view.url, "https://away.test.invalid/x");
    assert!(view.headers.is_empty());
    assert!(
        matches!(view.auth, AuthSpec::None),
        "no folder or collection auth goes to another host"
    );
    assert_eq!(received(cx, &t.server), 1, "nothing is sent");
}

#[gpui_kit::test]
fn a_caret_on_another_scheme_opens_nothing(cx: &mut TestAppContext) {
    let t = sent(cx);
    open_at(cx, &t, "x.invalid/f");
    assert_eq!(active_draft(cx, &t.s.ws), None);
    open_at(cx, &t, "\"away\"");
    assert_eq!(active_draft(cx, &t.s.ws), None);
}

#[gpui_kit::test]
fn a_url_holding_a_placeholder_opens_nothing(cx: &mut TestAppContext) {
    let t = sent(cx);
    open_at(cx, &t, "leak.test");
    assert_eq!(active_draft(cx, &t.s.ws), None);
}

#[gpui_kit::test]
fn after_a_redirect_the_origin_is_the_first_urls_and_the_base_the_last(cx: &mut TestAppContext) {
    let t = sent_via(cx, true);
    // The link shares the origin that answered, not the one the request was sent to.
    open_at(cx, &t, "page/2");
    let item = active_draft(cx, &t.s.ws).expect("a draft tab is active");
    let view = view_of(cx, &t.s.ws, &t.s.root, item.as_str());
    assert_eq!(view.url, format!("{}/page/2", t.uri));
    assert!(view.headers.is_empty());
    assert!(matches!(view.auth, AuthSpec::None));
    // A relative Location resolves against the URL that answered.
    cx.update_window(t.s.win, |_, w, cx| {
        t.pane
            .update(cx, |p, cx| p.open_url_as_request("/items/7", w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, t.s.win);
    let item = active_draft(cx, &t.s.ws).expect("a draft tab is active");
    let view = view_of(cx, &t.s.ws, &t.s.root, item.as_str());
    assert_eq!(view.url, format!("{}/items/7", t.uri));
    assert_eq!(received(cx, &t.server), 1, "nothing is sent");
}

#[gpui_kit::test]
fn the_location_rows_menu_opens_the_resolved_url(cx: &mut TestAppContext) {
    let t = sent(cx);
    t.pane
        .update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    frame(cx, t.s.win);
    let at = cx
        .update_window(t.s.win, |_, w, _| {
            use gpui_kit::test::TestWindowExt as _;
            let ix = (0..32usize)
                .find(|&ix| {
                    w.try_find(("header-name", ix))
                        .is_some_and(|e| e.label() == Some("location"))
                })
                .expect("a Location row");
            w.find(("header", ix)).bounds().center()
        })
        .unwrap();
    right_click(cx, t.s.win, at);
    frame(cx, t.s.win);
    click_menu(cx, t.s.win, OPEN_URL);
    frame(cx, t.s.win);
    let item = active_draft(cx, &t.s.ws).expect("a draft tab is active");
    let view = view_of(cx, &t.s.ws, &t.s.root, item.as_str());
    assert_eq!(view.url, format!("{}/items/7", t.uri));
    assert_eq!(view.method, "GET");
    assert_eq!(received(cx, &t.server), 1, "nothing is sent");
}

// A right press and release at `at`, through the window's event pipeline.
fn right_click(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    at: gpui_kit::Point<gpui_kit::Pixels>,
) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let down = MouseDownEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn the_body_editors_menu_lists_the_command(cx: &mut TestAppContext) {
    let t = sent(cx);
    let at = cx
        .update_window(t.s.win, |_, w, _| {
            use gpui_kit::test::TestWindowExt as _;
            w.find("body-editor").bounds().center()
        })
        .unwrap();
    right_click(cx, t.s.win, at);
    frame(cx, t.s.win);
    let labels = cx.update_window(t.s.win, |_, w, _| menu_labels(w)).unwrap();
    assert!(
        labels.iter().any(|l| l.as_deref() == Some(OPEN_URL)),
        "{labels:?}"
    );
}
