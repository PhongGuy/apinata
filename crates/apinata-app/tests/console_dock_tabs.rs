//! The dock's tabs against real app sends: console rows, filters, network rows, detail and redaction.

use crate::support;

use std::sync::Arc;

use apinata_app::console_dock::{ConsoleDock, DockTab};
use apinata_app::keymap::ToggleConsole;
use apinata_core::ConsoleLevel;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, Entity, TestAppContext, VisualTestContext};
use support::*;
use wiremock::ResponseTemplate;

// Opens the dock on `tab`, sends `a.yml`, and polls once the send is done.
fn send_and_show(cx: &mut TestAppContext, s: &Started, tab: DockTab) -> Entity<ConsoleDock> {
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    dock.update(cx, |d, cx| d.set_tab(tab, cx));
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    dock.update(cx, |d, cx| d.poll(cx));
    frame(cx, s.win);
    dock
}

// The aria label of `id`, or "" when it has none.
fn label_of(cx: &mut TestAppContext, s: &Started, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

// Labels of `(prefix, 0..)` for as long as such an element exists.
fn labels(cx: &mut TestAppContext, s: &Started, prefix: &'static str) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        let ix = out.len();
        if !present(cx, s.win, (prefix, ix)) {
            return out;
        }
        out.push(label_of(cx, s, (prefix, ix)));
    }
}

fn scripted(uri: &str, code: &str) -> String {
    format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"{code}\"\n",
        request("a", &format!("{uri}/ok"))
    )
}

// The collection dir comes back with the app so it lives as long as the test.
fn scripted_app(cx: &mut TestAppContext, uri: &str, code: &str) -> (tempfile::TempDir, Started) {
    let a = scripted(uri, code);
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    (col, s)
}

#[gpui_kit::test]
fn console_lines_show_with_level_filter_search_and_clear(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let (_col, s) = scripted_app(
        cx,
        &server.uri(),
        "console.log('hello from a'); console.warn('careful')",
    );
    let dock = send_and_show(cx, &s, DockTab::Console);
    let rows = labels(cx, &s, "dock-console");
    assert_eq!(rows.len(), 2);
    assert!(
        rows[0].contains("hello from a") && rows[0].contains(" a "),
        "{}",
        rows[0]
    );
    assert!(rows[1].contains("careful"));
    dock.update(cx, |d, cx| d.set_level_filter(Some(ConsoleLevel::Warn), cx));
    let rows = labels(cx, &s, "dock-console");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains("careful"));
    dock.update(cx, |d, cx| d.set_level_filter(None, cx));
    // Typing into the search box filters through its Change subscription.
    cx.update_window(s.win, |_, w, cx| {
        w.within("console-dock").click("dock-console-search", cx);
        w.input("HELLO", cx);
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(dock.read_with(cx, |d, _| d.console_shown().len()), 1);
    dock.update(cx, |d, cx| d.set_search("nope", cx));
    assert!(present(cx, s.win, "dock-console-none"));
    dock.update(cx, |d, cx| d.set_search("", cx));
    cx.update_window(s.win, |_, w, cx| {
        w.within("console-dock").click("dock-console-clear", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(dock.read_with(cx, |d, _| d.console_shown().is_empty()));
    assert!(present(cx, s.win, "dock-console-none"));
    assert_eq!(
        dock.read_with(cx, |d, _| d.network_shown().len()),
        1,
        "network kept"
    );
}

#[gpui_kit::test]
fn the_console_budget_keeps_the_newest_lines(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    // 30 lines of about 4000 characters overrun the 50 000 character pane budget.
    let code = "for (let i = 0; i < 30; i++) console.log('L' + i + ':' + 'x'.repeat(4000))";
    let (_col, s) = scripted_app(cx, &server.uri(), code);
    send_and_show(cx, &s, DockTab::Console);
    let rows = labels(cx, &s, "dock-console");
    assert!(rows.len() < 30, "the budget cut some lines");
    assert!(
        rows.last().unwrap().contains("L29:"),
        "the newest line is drawn"
    );
    assert!(
        rows.iter().all(|r| !r.contains("L0:")),
        "the oldest line is left out"
    );
    assert!(present(cx, s.win, "dock-console-omitted"));
}

const SECRET: &str = "Dock-S3cret-41";

// An app whose `a.yml` sends the secret `token` in its query and logs it from a script.
fn secret_app(cx: &mut TestAppContext, uri: &str) -> (tempfile::TempDir, Started) {
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: before-request\n      code: \"console.log(bru.getEnvVar('token'))\"\n",
        request("a", &format!("{uri}/ok?token={{{{token}}}}"))
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
    let root = col.path().to_path_buf();
    let s = started_on(cx, &col, |p| allow_scripts(p, &root), Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    (col, s)
}

#[gpui_kit::test]
fn a_secret_reaches_the_console_redacted(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let (_col, s) = secret_app(cx, &server.uri());
    let dock = send_and_show(cx, &s, DockTab::Console);
    let lines = dock.read_with(cx, |d, _| d.console_shown());
    assert_eq!(lines.len(), 1);
    assert!(lines[0].text.contains("[redacted]"), "{}", lines[0].text);
    let rows = labels(cx, &s, "dock-console");
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].contains(SECRET), "{}", rows[0]);
}

#[gpui_kit::test]
fn a_long_dock_line_has_its_own_more_note(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let code = "console.log(Array(1100).fill('y').join(String.fromCharCode(10)))";
    let (_col, s) = scripted_app(cx, &server.uri(), code);
    send_and_show(cx, &s, DockTab::Console);
    assert!(present(cx, s.win, ("dock-console-more", 0usize)));
    assert!(
        !present(cx, s.win, ("console-more", 0usize)),
        "the response pane's id stays its own"
    );
}

fn click_row(cx: &mut TestAppContext, s: &Started, ix: usize) {
    cx.update_window(s.win, |_, w, cx| {
        w.within("console-dock").click(("dock-network", ix), cx)
    })
    .unwrap();
    frame(cx, s.win);
}

fn selected(cx: &mut TestAppContext, s: &Started, ix: usize) -> Option<bool> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(("dock-network", ix)).selected()
    })
    .unwrap()
}

#[gpui_kit::test]
fn network_rows_select_filter_and_clear(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let dock = send_and_show(cx, &s, DockTab::Network);
    let row = label_of(cx, &s, ("dock-network", 0usize));
    assert!(row.starts_with("request GET "), "{row}");
    assert!(row.contains("/ok") && row.contains(" 200 "), "{row}");
    assert_ne!(selected(cx, &s, 0), Some(true));
    click_row(cx, &s, 0);
    assert!(dock.read_with(cx, |d, _| d.selected().is_some()));
    assert_eq!(
        selected(cx, &s, 0),
        Some(true),
        "the selected row is highlighted"
    );
    dock.update(cx, |d, cx| {
        d.set_source_filter(Some(apinata_core::SendSource::Script), cx)
    });
    frame(cx, s.win);
    assert!(present(cx, s.win, "dock-network-none"));
    dock.update(cx, |d, cx| d.set_source_filter(None, cx));
    cx.update_window(s.win, |_, w, cx| {
        w.within("console-dock").click("dock-network-clear", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(dock.read_with(cx, |d, _| d.network_shown().is_empty()));
    assert!(present(cx, s.win, "dock-network-none"));
}

#[gpui_kit::test]
fn a_failed_send_shows_as_error(cx: &mut TestAppContext) {
    http_ready(cx);
    // A port that refuses every connection.
    let port = dead_port();
    let col = collection_for(&format!("http://127.0.0.1:{port}"), "");
    let s = started(cx, &col);
    send_and_show(cx, &s, DockTab::Network);
    let row = label_of(cx, &s, ("dock-network", 0usize));
    assert!(row.contains(" error "), "{row}");
}

// A server whose /ok redirects to /final, which answers 200 with a header and "pong".
fn redirecting_server(cx: &mut TestAppContext) -> wiremock::MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/ok"))
            .respond_with(ResponseTemplate::new(302).append_header("location", "/final"))
            .mount(&server),
    );
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/final"))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header("x-reply", "yes")
                    .set_body_string("pong"),
            )
            .mount(&server),
    );
    server
}

#[gpui_kit::test]
fn a_selected_row_shows_headers_bodies_and_hops(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = redirecting_server(cx);
    let a = format!(
        r#"info:
  name: a
  type: http
  seq: 1
http:
  method: POST
  url: "{}/ok"
  headers:
    - name: x-test
      value: one
  body:
    type: json
    data: '{{"q":1}}'
"#,
        server.uri()
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    let dock = send_and_show(cx, &s, DockTab::Network);
    assert!(!present(cx, s.win, "dock-detail"), "no selection yet");
    click_row(cx, &s, 0);
    assert!(present(cx, s.win, "dock-detail"));
    assert!(present(cx, s.win, ("dock-request-header", 0usize)));
    assert!(present(cx, s.win, ("dock-response-header", 0usize)));
    assert_eq!(label_of(cx, &s, "dock-request-body"), r#"{"q":1}"#);
    assert_eq!(label_of(cx, &s, "dock-response-body"), "pong");
    let hops = labels(cx, &s, "dock-hop");
    assert_eq!(hops.len(), 2, "redirect and final hop: {hops:?}");
    assert!(hops[0].contains(" 302 "));
    assert!(!present(cx, s.win, "dock-hops-omitted"));
    assert!(!present(cx, s.win, "dock-error"));
    let detail = label_of(cx, &s, "dock-detail");
    assert!(detail.contains("x-test: one"), "{detail}");
    assert!(detail.contains("x-reply: yes"), "{detail}");
    let e = dock.read_with(cx, |d, _| d.selected().unwrap());
    assert!(
        e.request_headers
            .iter()
            .any(|(n, v)| n == "x-test" && v == "one")
    );
}

#[gpui_kit::test]
fn a_long_redirect_chain_shows_omitted_hops(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::path("/ok"))
            .respond_with(ResponseTemplate::new(302).append_header("location", "/ok"))
            .mount(&server),
    );
    // At the limit the last 302 is the response: 41 hops, of which an entry keeps 32.
    let col = collection_for(&server.uri(), "settings:\n  maxRedirects: 40\n");
    let s = started(cx, &col);
    send_and_show(cx, &s, DockTab::Network);
    let row = label_of(cx, &s, ("dock-network", 0usize));
    assert!(row.contains(" 302 "), "{row}");
    click_row(cx, &s, 0);
    assert!(!present(cx, s.win, "dock-error"));
    assert_eq!(labels(cx, &s, "dock-hop").len(), 32);
    assert!(present(cx, s.win, "dock-hops-omitted"));
    assert_eq!(
        label_of(cx, &s, "dock-hops-omitted"),
        "9 more hops not shown"
    );
}

#[gpui_kit::test]
fn a_failed_send_shows_its_error_in_the_detail(cx: &mut TestAppContext) {
    http_ready(cx);
    let port = dead_port();
    let col = collection_for(&format!("http://127.0.0.1:{port}"), "");
    let s = started(cx, &col);
    let dock = send_and_show(cx, &s, DockTab::Network);
    click_row(cx, &s, 0);
    let error = dock.read_with(cx, |d, _| d.selected().unwrap().error.clone().unwrap());
    assert_eq!(label_of(cx, &s, "dock-error"), error);
    assert!(label_of(cx, &s, "dock-detail").contains(&error));
}

#[gpui_kit::test]
fn a_secret_stays_out_of_the_network_rows_and_detail(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let (_col, s) = secret_app(cx, &server.uri());
    let dock = send_and_show(cx, &s, DockTab::Network);
    let e = dock.read_with(cx, |d, _| d.network_shown()[0].clone());
    assert!(!e.url.contains(SECRET), "{}", e.url);
    let rows = labels(cx, &s, "dock-network");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains("[redacted]"), "{}", rows[0]);
    click_row(cx, &s, 0);
    let detail = label_of(cx, &s, "dock-detail");
    assert!(detail.contains("[redacted]"), "{detail}");
    let hops = labels(cx, &s, "dock-hop");
    assert!(!hops.is_empty());
    // Every label the dock renders for the exchange, compared without regard to case.
    let secret = SECRET.to_lowercase();
    for label in rows.iter().chain(&hops).chain([&detail]) {
        assert!(!label.to_lowercase().contains(&secret), "{label}");
    }
}

#[gpui_kit::test]
fn a_truncated_body_shows_its_marker(cx: &mut TestAppContext) {
    http_ready(cx);
    // Over the log's 64 KiB body cap, so the stored response body is cut.
    let big = "z".repeat(70 * 1024);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string(big));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    send_and_show(cx, &s, DockTab::Network);
    click_row(cx, &s, 0);
    let note = label_of(cx, &s, ("dock-response-body-truncated", 0usize));
    assert!(note.contains("truncated"), "{note}");
    assert!(!present(cx, s.win, ("dock-request-body-truncated", 0usize)));
}

#[gpui_kit::test]
fn the_list_and_detail_keep_their_height(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    send_and_show(cx, &s, DockTab::Network);
    click_row(cx, &s, 0);
    let (list, detail) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("dock-network-column").bounds().size.height,
                w.find("dock-detail-scroll").bounds().size.height,
            )
        })
        .unwrap();
    assert!(list > gpui_kit::px(0.), "{list:?}");
    assert!(detail > gpui_kit::px(0.), "{detail:?}");
}

// Bounds of the pane's final content (the last hop line, below every header and the body),
// of the final response header row, and of the pane itself.
fn tail_and_pane(cx: &mut TestAppContext, s: &Started) -> [gpui_kit::Bounds<gpui_kit::Pixels>; 3] {
    let mut rows = 0usize;
    while present(cx, s.win, ("dock-response-header", rows)) {
        rows += 1;
    }
    assert!(
        rows >= 30,
        "the 30 set headers plus the server's own: {rows}"
    );
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        [
            w.find(("dock-hop", 0usize)).bounds(),
            w.find(("dock-response-header", rows - 1)).bounds(),
            w.find("dock-detail-scroll").bounds(),
        ]
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_tall_detail_scrolls_to_its_last_content(cx: &mut TestAppContext) {
    http_ready(cx);
    let many = (0..30).fold(ResponseTemplate::new(200), |r, i| {
        r.insert_header(
            format!("x-h-{i:02}").as_str(),
            "word ".repeat(if i % 3 == 0 { 120 } else { 1 }).trim(),
        )
    });
    let body = (0..40).map(|i| format!("line {i}\n")).collect::<String>();
    let server = mock_server(cx, many.set_body_string(body));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let dock = send_and_show(cx, &s, DockTab::Network);
    click_row(cx, &s, 0);
    // The window is the test default, so the dock opens at its configured height.
    let want = apinata_app::style::at_zoom(apinata_app::style::DOCK_HEIGHT, 1.0);
    let heights = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            [
                w.find("console-dock").bounds().size.height,
                w.find("dock-detail").bounds().size.height,
            ]
        })
        .unwrap();
    assert_eq!(heights[0], want, "the default dock height: {heights:?}");
    assert!(
        heights[1] < want,
        "the detail sits inside the dock: {heights:?}"
    );
    let [tail, last_header, pane] = tail_and_pane(cx, &s);
    assert!(
        last_header.bottom() > pane.bottom(),
        "clipped: {last_header:?} in {pane:?}"
    );
    assert!(
        tail.bottom() > pane.bottom(),
        "clipped: {tail:?} in {pane:?}"
    );
    cx.update_window(s.win, |_, w, cx| {
        let down =
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-50000.)));
        w.scroll("dock-detail-scroll", down, cx)
    })
    .unwrap();
    frame(cx, s.win);
    dock.update(cx, |d, cx| d.poll(cx));
    frame(cx, s.win);
    let [tail, _, pane] = tail_and_pane(cx, &s);
    assert!(
        tail.top() >= pane.top() && tail.bottom() <= pane.bottom(),
        "the last content is reachable: {tail:?} in {pane:?}"
    );
}

// Another entry opens at its top; the same entry re-rendering keeps where it was scrolled to.
#[gpui_kit::test]
fn the_detail_scrolls_to_the_top_for_another_entry(cx: &mut TestAppContext) {
    http_ready(cx);
    let many = (0..30).fold(ResponseTemplate::new(200), |r, i| {
        r.insert_header(format!("x-h-{i:02}").as_str(), "v")
    });
    let server = mock_server(cx, many);
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let dock = send_and_show(cx, &s, DockTab::Network);
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    finished(cx, &model);
    dock.update(cx, |d, cx| d.poll(cx));
    click_row(cx, &s, 0);
    cx.update_window(s.win, |_, w, cx| {
        let down =
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-50000.)));
        w.scroll("dock-detail-scroll", down, cx)
    })
    .unwrap();
    frame(cx, s.win);
    dock.update(cx, |d, cx| d.poll(cx));
    frame(cx, s.win);
    let [tail, _, pane] = tail_and_pane(cx, &s);
    assert!(
        tail.bottom() <= pane.bottom(),
        "A is scrolled down: {tail:?} in {pane:?}"
    );
    click_row(cx, &s, 1);
    let [tail, _, pane] = tail_and_pane(cx, &s);
    assert!(
        tail.bottom() > pane.bottom(),
        "B opens at its top: {tail:?} in {pane:?}"
    );
}

#[gpui_kit::test]
fn each_measured_hop_shows_its_timing_breakdown(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = redirecting_server(cx);
    let a = format!(
        "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}/ok\"\n",
        server.uri()
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    send_and_show(cx, &s, DockTab::Network);
    click_row(cx, &s, 0);
    let lines = labels(cx, &s, "dock-hop-timing");
    assert_eq!(lines.len(), 2, "one per hop: {lines:?}");
    // A 127.0.0.1 target has no DNS phase; only measured phases are worded.
    assert!(lines[0].contains("Wait "), "{lines:?}");
    assert!(!lines[0].contains("DNS"), "{lines:?}");
}

#[gpui_kit::test]
fn a_theme_warning_row_has_no_script_phase_tag(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started_with(cx, &col, |p| {
        std::fs::create_dir_all(&p.themes).unwrap();
        std::fs::write(p.themes.join("broken.json"), "[").unwrap();
    });
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    dock.update(cx, |d, cx| {
        d.set_tab(DockTab::Console, cx);
        d.poll(cx);
    });
    frame(cx, s.win);
    let rows = labels(cx, &s, "dock-console");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("Themes") && rows[0].contains("broken"));
    assert!(!rows[0].contains("[pre-request]"), "{}", rows[0]);
}

// A detail pane that overflows shows an always-visible scrollbar; one that fits shows none.
#[gpui_kit::test]
fn an_overflowing_detail_shows_a_scrollbar(cx: &mut TestAppContext) {
    http_ready(cx);
    let many = (0..30).fold(ResponseTemplate::new(200), |r, i| {
        r.insert_header(format!("x-h-{i:02}").as_str(), "v")
    });
    let server = mock_server(cx, many);
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    send_and_show(cx, &s, DockTab::Network);
    click_row(cx, &s, 0);
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(
        present(cx, s.win, "dock-detail-scroll-bar"),
        "30+ headers overflow the 240px dock"
    );
}

#[gpui_kit::test]
fn a_list_that_fits_shows_no_scrollbar(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(204));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    send_and_show(cx, &s, DockTab::Network);
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(present(cx, s.win, "dock-network-list"));
    assert!(!present(cx, s.win, "dock-network-list-bar"), "one row fits");
}
