use crate::support;

use apinata_app::response_pane::{ResponsePane, ResponseTab, assertion_line};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

fn pane(cx: &mut TestAppContext, s: &Sent) -> (AnyWindowHandle, Entity<ResponsePane>) {
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    (win, view)
}

#[gpui_kit::test]
fn the_status_line_shows_status_time_and_size(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(201).set_body_raw("{\"a\":1}", "application/json"),
        "",
    );
    let (win, view) = pane(cx, &s);
    let sum = view
        .read_with(cx, |p, cx| p.summary(cx))
        .expect("a summary");
    assert_eq!(
        (
            sum.status,
            sum.reason.as_str(),
            sum.size.as_str(),
            sum.truncated
        ),
        (201, "Created", "7 B", false)
    );
    assert!(sum.time.ends_with(" ms") || sum.time.ends_with(" s"));
    assert!(present(cx, win, "response-status"));
    assert!(present(cx, win, "response-body"));
}

#[gpui_kit::test]
fn tabs_switch_by_click(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200).insert_header("x-a", "1"), "");
    let (win, view) = pane(cx, &s);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(("response-tab", 1usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |p, _| p.tab()), ResponseTab::Headers);
    assert!(present(cx, win, "response-headers"));
    let shown = s.result.response.as_ref().unwrap().headers().len();
    assert!((0..shown).all(|ix| present(cx, win, ("header", ix))));
}

#[gpui_kit::test]
fn the_timeline_has_one_entry_per_hop(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::start());
    let redirect = ResponseTemplate::new(302).insert_header("location", "/final");
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/ok"))
            .respond_with(redirect)
            .mount(&server),
    );
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/final"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &model);
    let s = Sent {
        server,
        col,
        state,
        ws,
        root,
        model,
        result,
    };
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    let uri = s.server.uri();
    assert_eq!(
        view.read_with(cx, |p, cx| p.hop_titles(cx)),
        [format!("GET {uri}/ok 302"), format!("GET {uri}/final 200")]
    );
    assert!(present(cx, win, ("hop", 1usize)));
}

#[gpui_kit::test]
fn assertions_show_pass_fail_and_skip(cx: &mut TestAppContext) {
    let extra = concat!(
        "runtime:\n  assertions:\n",
        "    - expression: res.status\n      operator: eq\n      value: \"201\"\n",
        "    - expression: res.status\n      operator: eq\n      value: \"500\"\n",
        "    - expression: res.url\n      operator: isDefined\n",
    );
    let s = send_once(cx, ResponseTemplate::new(201), extra);
    let lines: Vec<_> = s.result.assertions.iter().map(assertion_line).collect();
    assert_eq!(
        lines,
        [
            ("pass", "res.status eq 201".to_string()),
            ("fail", "res.status eq 500: actual 201".to_string()),
            (
                "skipped",
                "res.url isDefined: left side \"res.url\" needs a JavaScript runtime".to_string()
            ),
        ]
    );
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Tests, cx));
    assert!(present(cx, win, ("assertion", 2usize)));
}

#[gpui_kit::test]
fn a_run_error_replaces_the_body(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/{{nope}}")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    let errored = Sent {
        model,
        ws,
        root,
        col,
        ..s
    };
    let (win, view) = pane(cx, &errored);
    let error = view
        .read_with(cx, |p, cx| p.error_view(cx))
        .and_then(|v| v.message)
        .expect("an error");
    assert!(error.contains("nope"), "{error}");
    assert!(
        error.contains("\"nope\"") && !error.contains('`'),
        "{error}"
    );
    assert!(present(cx, win, "response-error"));
    assert!(!present(cx, win, "response-body"));
}

#[gpui_kit::test]
fn idle_and_sending_have_their_own_placeholders(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30)),
    );
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    support::init(cx);
    let m = model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    assert!(present(cx, win, "response-idle"));
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    assert!(present(cx, win, "response-sending"));
    // Cancelled while planning or in flight, the pane says the same.
    model.update(cx, |m, cx| m.cancel(cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    assert_eq!(
        view.read_with(cx, |p, cx| p.error_view(cx).map(|v| v.title)),
        Some("Cancelled")
    );
}

#[gpui_kit::test]
fn a_shown_result_fills_the_pane(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(201), "");
    let model = sender(cx, &s.ws);
    model.update(cx, |m, cx| m.show(s.result.clone(), cx));
    let (win, view) = pane(cx, &Sent { model, ..s });
    let status = view.read_with(cx, |p, cx| p.summary(cx)).map(|s| s.status);
    assert_eq!(status, Some(201));
    assert!(present(cx, win, "response-status"));
}

#[gpui_kit::test]
fn a_skipped_result_reads_as_cancelled(cx: &mut TestAppContext) {
    // A cancel that lands before the run polls yields Skipped rather than Cancelled.
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let mut skipped = (*s.result).clone();
    skipped.outcome = apinata_core::Outcome::Skipped;
    skipped.response = None;
    s.model
        .update(cx, |m, cx| m.show(std::sync::Arc::new(skipped), cx));
    let (win, view) = pane(cx, &s);
    assert_eq!(
        view.read_with(cx, |p, cx| p.error_view(cx).map(|v| v.title)),
        Some("Cancelled")
    );
    assert!(present(cx, win, "response-error"));
}

fn json_rows(n: usize) -> String {
    let fields: Vec<String> = (0..n).map(|i| format!("\"k{i}\":{i}")).collect();
    format!("{{{}}}", fields.join(","))
}

#[gpui_kit::test]
fn a_json_body_fills_the_body_area(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw(json_rows(60), "application/json"),
        "",
    );
    let (win, view) = pane(cx, &s);
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(1024.), gpui_kit::px(600.)));
    cx.run_until_parked();
    let editor = view.read_with(cx, |p, cx| p.body().read(cx).editor().clone());
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let area = w.find("response-body").bounds();
        let slot = w.find("body-editor").bounds();
        let input = editor.read(cx).input_bounds();
        assert!(
            input.size.height > gpui_kit::px(400.),
            "editor clipped: {input:?} in {slot:?}"
        );
        assert!(
            slot.bottom() <= area.bottom() + gpui_kit::px(0.5),
            "{slot:?} past {area:?}"
        );
        assert!(
            area.bottom() <= gpui_kit::px(600.5),
            "{area:?} past the window"
        );
        assert!(
            input.bottom() <= slot.bottom() + gpui_kit::px(0.5),
            "{input:?} past {slot:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_long_header_list_scrolls(cx: &mut TestAppContext) {
    let mut response = ResponseTemplate::new(200);
    for i in 0..40 {
        response = response.insert_header(format!("x-h{i:02}").as_str(), "v");
    }
    let s = send_once(cx, response, "");
    let (win, view) = pane(cx, &s);
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(1024.), gpui_kit::px(400.)));
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    cx.run_until_parked();
    let before = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let list = w.find("response-headers").bounds();
            assert!(
                list.bottom() <= gpui_kit::px(400.5),
                "{list:?} past the window"
            );
            w.find(("header", 0usize)).bounds()
        })
        .unwrap();
    let after = cx
        .update_window(win, |_, w, cx| {
            w.scroll(
                "response-headers",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-200.),
                )),
                cx,
            );
            w.find(("header", 0usize)).bounds()
        })
        .unwrap();
    assert!(
        after.top() < before.top(),
        "no scroll: {before:?} then {after:?}"
    );
}

#[gpui_kit::test]
fn header_values_share_one_column(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200)
            .insert_header("a", "1")
            .insert_header("x-a-much-longer-header-name", "2"),
        "",
    );
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let lefts: Vec<_> = (0..2usize)
            .map(|ix| w.find(("header-value", ix)).bounds().left())
            .collect();
        assert_eq!(lefts[0], lefts[1]);
        let name = w.find(("header-name", 0usize)).bounds();
        assert_eq!(
            name.size.width,
            apinata_app::style::at_zoom(apinata_app::style::HEADER_NAME_WIDTH, 1.0)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn each_hop_splits_request_and_response_headers(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200).insert_header("x-a", "1"), "");
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let hop = w.within(("hop", 0usize));
        let request = hop.find("hop-request-heading").bounds();
        let response = hop.find("hop-response-heading").bounds();
        let first_response_row = hop.find(("hop-response-header", 0usize)).bounds();
        assert!(
            request.bottom() <= response.top(),
            "{request:?} {response:?}"
        );
        assert!(response.bottom() <= first_response_row.top());
        if let Some(row) = hop.try_find(("hop-request-header", 0usize)) {
            assert!(row.bounds().bottom() <= response.top());
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn no_tests_says_so(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Tests, cx));
    assert!(present(cx, win, "tests-none"));
}

// Renders `tab` in a 400 px window and reports where `first` sits before and after a
// 200 px wheel scroll over `frame`; the frame must stay inside the window.
fn scroll_once(
    cx: &mut TestAppContext,
    s: &Sent,
    tab: ResponseTab,
    frame: &'static str,
    first: (&'static str, usize),
) -> (
    gpui_kit::Bounds<gpui_kit::Pixels>,
    gpui_kit::Bounds<gpui_kit::Pixels>,
) {
    let (win, view) = pane(cx, s);
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(1024.), gpui_kit::px(400.)));
    view.update(cx, |p, cx| jar_tab(p, tab, cx));
    cx.run_until_parked();
    let before = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let list = w.find(frame).bounds();
            assert!(
                list.bottom() <= gpui_kit::px(400.5),
                "{list:?} past the window"
            );
            w.find(first).bounds()
        })
        .unwrap();
    let after = cx
        .update_window(win, |_, w, cx| {
            w.scroll(
                frame,
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-200.),
                )),
                cx,
            );
            w.find(first).bounds()
        })
        .unwrap();
    (before, after)
}

#[gpui_kit::test]
fn a_long_cookie_list_scrolls(cx: &mut TestAppContext) {
    let mut response = ResponseTemplate::new(200);
    for i in 0..40 {
        response = response.append_header("set-cookie", format!("c{i:02}=v; Path=/").as_str());
    }
    let s = send_once(cx, response, "");
    let (before, after) = scroll_once(
        cx,
        &s,
        ResponseTab::Cookies,
        "response-cookies",
        ("cookie", 0usize),
    );
    assert!(
        after.top() < before.top(),
        "no scroll: {before:?} then {after:?}"
    );
}

#[gpui_kit::test]
fn a_long_assertion_list_scrolls(cx: &mut TestAppContext) {
    let mut extra = String::from("runtime:\n  assertions:\n");
    for _ in 0..40 {
        extra.push_str("    - expression: res.status\n      operator: eq\n      value: \"200\"\n");
    }
    let s = send_once(cx, ResponseTemplate::new(200), &extra);
    let (before, after) = scroll_once(
        cx,
        &s,
        ResponseTab::Tests,
        "response-assertions",
        ("assertion", 0usize),
    );
    assert!(
        after.top() < before.top(),
        "no scroll: {before:?} then {after:?}"
    );
}

// The read-only Cookies heading keeps its natural band; only edit grids take `KV_HEAD_HEIGHT`.
#[gpui_kit::test]
fn the_cookie_head_keeps_its_natural_height(cx: &mut TestAppContext) {
    use apinata_app::style::{KV_HEAD_HEIGHT, at_zoom};
    let response = ResponseTemplate::new(200).append_header("set-cookie", "c=v; Path=/");
    let s = send_once(cx, response, "");
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    cx.run_until_parked();
    let height = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("cookies-head").bounds().size.height
        })
        .unwrap();
    assert!(
        height < at_zoom(KV_HEAD_HEIGHT, 1.0),
        "cookies head is {height:?}, the edit-grid band"
    );
}

#[gpui_kit::test]
fn the_cookie_head_stays_while_the_rows_scroll(cx: &mut TestAppContext) {
    let mut response = ResponseTemplate::new(200);
    for i in 0..40 {
        response = response.append_header("set-cookie", format!("c{i:02}=v; Path=/").as_str());
    }
    let s = send_once(cx, response, "");
    let (win, view) = pane(cx, &s);
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(1024.), gpui_kit::px(400.)));
    view.update(cx, |p, cx| jar_tab(p, ResponseTab::Cookies, cx));
    cx.run_until_parked();
    let bounds = |w: &mut gpui_kit::Window| {
        (
            w.find("cookies-head").bounds(),
            w.find(("cookie", 0usize)).bounds(),
        )
    };
    let (head, row) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            bounds(w)
        })
        .unwrap();
    let (head_after, row_after) = cx
        .update_window(win, |_, w, cx| {
            w.scroll(
                "response-cookies",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-200.),
                )),
                cx,
            );
            bounds(w)
        })
        .unwrap();
    assert!(
        row_after.top() < row.top(),
        "no scroll: {row:?} then {row_after:?}"
    );
    assert_eq!(head_after, head);
}

#[gpui_kit::test]
fn assertion_results_are_summed_and_coloured(cx: &mut TestAppContext) {
    use apinata_app::response_pane::{assertion_summary, verdict_colour};
    use gpui_kit::component::ActiveTheme as _;
    let extra = concat!(
        "runtime:\n  assertions:\n",
        "    - expression: res.status\n      operator: eq\n      value: \"201\"\n",
        "    - expression: res.status\n      operator: eq\n      value: \"500\"\n",
        "    - expression: res.url\n      operator: isDefined\n",
    );
    let s = send_once(cx, ResponseTemplate::new(201), extra);
    assert_eq!(
        assertion_summary(&s.result.assertions).as_deref(),
        Some("1 passed, 1 failed, 1 skipped")
    );
    assert_eq!(assertion_summary(&[]), None);
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Tests, cx));
    assert!(present(cx, win, "assertions-summary"));
    assert!(present(cx, win, ("assertion-verdict", 1usize)));
    cx.update(|cx| {
        let t = cx.theme();
        assert_eq!(verdict_colour("pass", cx), t.success);
        assert_eq!(verdict_colour("fail", cx), t.danger);
        assert_eq!(verdict_colour("skipped", cx), t.muted_foreground);
    });
}

#[test]
fn hop_bodies_show_as_text_or_a_size() {
    use apinata_app::response_pane::hop_body_text;
    assert_eq!(hop_body_text(b"", 0), None);
    assert_eq!(hop_body_text(b"hello", 5).as_deref(), Some("hello"));
    assert_eq!(
        hop_body_text(&[0xff, 0xfe, 0x00], 3).as_deref(),
        Some("3 B not shown")
    );
    let cut = vec![b'a'; 64 * 1024];
    assert_eq!(
        hop_body_text(&cut, 70 * 1024).as_deref(),
        Some("70.0 KB not shown")
    );
}

#[gpui_kit::test]
fn the_timeline_shows_the_request_body(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200),
        "  body:\n    type: text\n    data: hello\n",
    );
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    assert!(present(cx, win, "hop-request-body"));
}

#[gpui_kit::test]
fn assertion_values_keep_their_backticks(cx: &mut TestAppContext) {
    let extra = concat!(
        "runtime:\n  assertions:\n",
        "    - expression: res.status\n      operator: eq\n      value: \"`${x}`\"\n",
    );
    let s = send_once(cx, ResponseTemplate::new(200), extra);
    let (_, text) = assertion_line(&s.result.assertions[0]);
    assert!(text.starts_with("res.status eq `${x}`"), "{text}");
}

#[gpui_kit::test]
fn the_headers_tab_heads_its_columns_and_labels_each_cell(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).insert_header("x-probe", "probe-value"),
        "",
    );
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    let shown = s.result.response.as_ref().unwrap().headers().len();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let head = w.find("headers-head").bounds();
        assert!(head.bottom() <= w.find(("header", 0usize)).bounds().top());
        let probe = (0..shown)
            .find(|ix| w.find(("header-name", *ix)).label() == Some("x-probe"))
            .expect("the probe header");
        assert_eq!(w.find(("header-value", probe)).label(), Some("probe-value"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_idle_send_shows_its_key_with_focus_elsewhere(cx: &mut TestAppContext) {
    let col = two_requests();
    let (_state, ws, _root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let m = model.clone();
    let (win, _view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    let hinted = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.within("response-idle-actions")
                .try_find(("empty-kbd", 0usize))
                .is_some()
        })
        .unwrap();
    assert!(
        hinted,
        "Send reads Send Ctrl+Enter with no request editor focused"
    );
}

// An NTLM Type 1 / Type 2 message as base64: the "NTLMSSP\0" signature then the type word.
const NTLM_TYPE1: &str = "TlRMTVNTUAABAAAAB4IIAAAAAAAAAAAAAAAAAAAAAAA=";
const NTLM_TYPE2: &str = "TlRMTVNTUAACAAAADAAMADgAAAAFgomiYWJjZGVmZ2gAAAAAAAAAAA==";

#[gpui_kit::test]
fn the_timeline_masks_credential_header_values(cx: &mut TestAppContext) {
    let extra = format!(
        "  headers:\n    - name: Authorization\n      value: \"NTLM {NTLM_TYPE1}\"\n    - name: X-Plain\n      value: visible-value\n"
    );
    let s = send_once(
        cx,
        ResponseTemplate::new(200)
            .insert_header("www-authenticate", format!("NTLM {NTLM_TYPE2}").as_str())
            .insert_header("proxy-authenticate", format!("NTLM {NTLM_TYPE2}").as_str()),
        &extra,
    );
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Timeline, cx));
    let shown = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let mut all = Vec::new();
            for id in ["hop-request-header-value", "hop-response-header-value"] {
                for ix in 0..40usize {
                    if let Some(e) = w.try_find((id, ix)) {
                        all.push(e.label().unwrap_or_default().to_string());
                    }
                }
            }
            all
        })
        .unwrap();
    assert!(shown.iter().any(|t| t == "visible-value"), "{shown:?}");
    for blob in [NTLM_TYPE1, NTLM_TYPE2] {
        assert!(!shown.iter().any(|t| t.contains(blob)), "{shown:?}");
    }
    assert!(shown.iter().any(|t| t.starts_with("NTLM ")), "{shown:?}");
}

#[gpui_kit::test]
fn hovering_a_cut_header_name_shows_it_in_full(cx: &mut TestAppContext) {
    let name = format!("x-{}", "very-long-header-name-".repeat(6));
    let s = send_once(
        cx,
        ResponseTemplate::new(200).insert_header(name.as_str(), "1"),
        "",
    );
    let (win, view) = pane(cx, &s);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    let shown = s.result.response.as_ref().unwrap().headers().len();
    let ix = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (0..shown).find(|ix| w.find(("header-name", *ix)).label() == Some(name.as_str()))
        })
        .unwrap()
        .expect("the long header");
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(("header-name", ix), cx)
    })
    .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(800));
    frame(cx, win);
    let tip = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("kv-cut-tooltip")
                .map(|t| t.label().unwrap_or_default().to_owned())
        })
        .unwrap();
    assert_eq!(tip, Some(name));
}

#[gpui_kit::test]
fn a_narrow_headers_table_gives_the_value_the_room(cx: &mut TestAppContext) {
    let value = "application/json; charset=utf-8";
    let s = send_once(
        cx,
        ResponseTemplate::new(200).insert_header("content-type", value),
        "",
    );
    let (win, view) = pane(cx, &s);
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(320.), gpui_kit::px(600.)));
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let ix = (0usize..16)
            .find(|ix| {
                (w.try_find(("header-value", *ix))).is_some_and(|e| e.label() == Some(value))
            })
            .expect("the content type row");
        let name = w.find(("header-name", ix)).bounds();
        let cell = w.find(("header-value", ix)).bounds();
        assert!(
            cell.size.width > name.size.width * 1.5,
            "the name {name:?} crowds the value {cell:?}"
        );
        // One line pads to the name's height; the value breaks once, at its space.
        assert!(
            cell.size.height < name.size.height * 2.,
            "the value wraps mid-word: {cell:?} beside {name:?}"
        );
    })
    .unwrap();
}
