//! A GraphQL response whose body carries a non-empty `errors` array shows a short warning badge
//! beside the status, with the summary on hover, and a click filters the body to the errors; a
//! REST response with the same body shows none.

use crate::support;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;
use wiremock::matchers::path;

const SUMMARY: &str = "response-graphql-errors";

fn graphql(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"{url}\"\n  body:\n    query: \"{{ me }}\"\n"
    )
}

// A started app whose `a.yml` (GraphQL, or HTTP when `rest`) gets `body` back, sent once.
fn sent(cx: &mut TestAppContext, rest: bool, body: &str) -> (Started, wiremock::MockServer) {
    let (s, server, _) = sent_tab(cx, rest, body);
    (s, server)
}

// As `sent`, with the tab's id too.
fn sent_tab(
    cx: &mut TestAppContext,
    rest: bool,
    body: &str,
) -> (Started, wiremock::MockServer, apinata_app::tabs::TabId) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        wiremock::Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .mount(&server),
    );
    let url = format!("{}/ok", server.uri());
    let a = if rest {
        request("a", &url)
    } else {
        graphql("a", &url)
    };
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    std::mem::forget(col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-status"));
    for _ in 0..3 {
        frame(cx, s.win);
    }
    (s, server, tab)
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// The badge's hover text, after the tooltip's delay.
fn tooltip(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(SUMMARY, cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    label(cx, win, "response-graphql-errors-tooltip")
}

#[gpui_kit::test]
fn a_graphql_error_shows_its_summary_beside_the_status(cx: &mut TestAppContext) {
    let body = r#"{"errors":[{"message":"Cannot query field \"x\" on type \"Query\"","path":["user","x"]},{"message":"b"}],"data":null}"#;
    let (s, _server) = sent(cx, false, body);
    assert_eq!(
        label(cx, s.win, SUMMARY).as_deref(),
        Some("2 GraphQL errors")
    );
    let (status, summary) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (w.find("response-status").bounds(), w.find(SUMMARY).bounds())
        })
        .unwrap();
    assert!(
        (summary.center().y - status.center().y).abs() < gpui_kit::px(2.),
        "{summary:?} sits on the status row {status:?}"
    );
    assert_eq!(
        tooltip(cx, s.win).as_deref(),
        Some(r#"2 GraphQL errors: Cannot query field "x" on type "Query" (at user.x)"#)
    );
}

#[gpui_kit::test]
fn partial_data_says_so(cx: &mut TestAppContext) {
    let (s, _server) = sent(
        cx,
        false,
        r#"{"data":{"me":null},"errors":[{"message":"m"}]}"#,
    );
    assert_eq!(
        label(cx, s.win, SUMMARY).as_deref(),
        Some("1 GraphQL error")
    );
    assert_eq!(
        tooltip(cx, s.win).as_deref(),
        Some("1 GraphQL error, partial data: m")
    );
}

#[gpui_kit::test]
fn a_graphql_response_without_errors_shows_no_summary(cx: &mut TestAppContext) {
    let (s, _server) = sent(cx, false, r#"{"data":{"me":"x"}}"#);
    assert!(!present(cx, s.win, SUMMARY));
}

#[gpui_kit::test]
fn a_rest_response_with_errors_shows_no_summary(cx: &mut TestAppContext) {
    let (s, _server) = sent(cx, true, r#"{"errors":[{"message":"m"}]}"#);
    assert!(!present(cx, s.win, SUMMARY));
}

// A long message is cut to the summary's width cap; the tooltip carries all of it.
#[gpui_kit::test]
fn a_long_message_is_cut_and_the_tooltip_has_it_whole(cx: &mut TestAppContext) {
    let message = "a very long error message ".repeat(20);
    let message = message.trim_end();
    let body = format!(r#"{{"errors":[{{"message":"{message}"}}]}}"#);
    let (s, _server) = sent(cx, false, &body);
    let (width, cap) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let cap = apinata_app::style::RESPONSE_ADDRESS_MAX.to_pixels(w.rem_size());
            (w.find(SUMMARY).bounds().size.width, cap)
        })
        .unwrap();
    assert!(width <= cap + gpui_kit::px(0.5), "{width:?} over {cap:?}");
    assert_eq!(
        tooltip(cx, s.win),
        Some(format!("1 GraphQL error: {message}"))
    );
}

#[gpui_kit::test]
fn clicking_the_badge_opens_the_body_filtered_to_the_errors(cx: &mut TestAppContext) {
    let (s, _server, tab) = sent_tab(cx, false, r#"{"errors":[{"message":"m"}],"data":null}"#);
    let pane = editor_of(cx, &s, tab).read_with(cx, |e, _| e.response().clone());
    pane.update(cx, |p, cx| {
        p.set_tab(apinata_app::response_pane::ResponseTab::Headers, cx)
    });
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(SUMMARY, cx))
        .unwrap();
    frame(cx, s.win);
    let (tab, filter) = pane.read_with(cx, |p, cx| {
        let filter = p.body().read(cx).applied_filter().map(str::to_owned);
        (p.tab(), filter)
    });
    assert_eq!(tab, apinata_app::response_pane::ResponseTab::Body);
    assert_eq!(filter.as_deref(), Some("$.errors"));
}
