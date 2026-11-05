//! "Export as HAR": the response pane, its narrow menu, the palette and the network dock
//! write one HAR 1.2 entry through the save dialog, with secrets redacted.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::console_dock::DockTab;
use apinata_app::keymap::ToggleConsole;
use apinata_core::{Collection, MemoryStore, SecretStore};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use support::*;
use wiremock::ResponseTemplate;

const TOKEN: &str = "Har-S3cret-77";

/// A request that sends `{{token}}` and a server that echoes it back in a header and the body.
fn echo_app(cx: &mut TestAppContext) -> (wiremock::MockServer, tempfile::TempDir, Started) {
    let reply = ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .insert_header("x-echo", TOKEN)
        .set_body_string(format!("{{\"echo\":\"{TOKEN}\"}}"));
    echo_app_with(cx, reply)
}

fn echo_app_with(
    cx: &mut TestAppContext,
    reply: ResponseTemplate,
) -> (wiremock::MockServer, tempfile::TempDir, Started) {
    http_ready(cx);
    let server = mock_server(cx, reply);
    let file = format!(
        "{}  headers:\n    - name: x-token\n      value: \"{{{{token}}}}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &file),
    ]);
    let c = Collection::open(col.path()).expect("opens");
    let store = MemoryStore::new();
    store.set(&c.secret_key("dev", "token"), TOKEN).unwrap();
    let s = started_with_store(cx, &col, Arc::new(store));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    (server, col, s)
}

/// Opens `a.yml`, sends it from the toolbar and waits for the response.
fn send(cx: &mut TestAppContext, s: &Started) {
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area.read_with(cx, |a, _| a.editor(tab).cloned()).unwrap();
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    finished(cx, &model);
    frame(cx, s.win);
}

/// Picks `file` in the open save dialog and returns the parsed HAR once it is written.
fn answer(cx: &mut TestAppContext, file: &Path) -> Value {
    wait_real(cx, |cx| cx.did_prompt_for_new_path());
    let chosen = file.to_path_buf();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    wait_real(cx, |_| file.exists());
    let text = std::fs::read_to_string(file).unwrap();
    serde_json::from_str(&text).expect("a HAR file")
}

// Every export describes the request as sent: the secret's value is `[redacted]`, never the
// `{{token}}` template that produced it.
fn check(har: &Value, server: &wiremock::MockServer) {
    let e = &har["log"]["entries"][0];
    assert_eq!(har["log"]["version"], "1.2");
    assert_eq!(e["request"]["method"], "GET");
    assert_eq!(e["request"]["url"], format!("{}/ok", server.uri()));
    assert_eq!(e["response"]["status"], 200);
    let text = har.to_string();
    assert!(!text.contains(TOKEN), "the secret is redacted: {text}");
    assert!(!text.contains("{{token}}"), "not the template: {text}");
    let sent = e["request"]["headers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["name"] == "x-token")
        .unwrap_or_else(|| panic!("the sent header: {text}"));
    assert_eq!(sent["value"], "[redacted]", "{text}");
    assert_eq!(e["timings"]["dns"], -1);
}

#[gpui_kit::test]
fn the_response_toolbar_exports_the_exchange(cx: &mut TestAppContext) {
    let (server, _col, s) = echo_app(cx);
    assert!(
        !present(cx, s.win, "response-export-har"),
        "no response yet"
    );
    send(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("response-export-har", cx))
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let har = answer(cx, &out.path().join("a.har"));
    check(&har, &server);
}

#[gpui_kit::test]
fn the_narrow_menu_and_the_palette_export_too(cx: &mut TestAppContext) {
    let (server, _col, s) = echo_app(cx);
    send(cx, &s);
    VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(520.), gpui_kit::px(700.)));
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "response-export-har"), "folded away");
    cx.update_window(s.win, |_, w, cx| w.click("response-more", cx))
        .unwrap();
    frame(cx, s.win);
    // After Save as example, Copy body and Save body.
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(3usize, cx))
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    check(&answer(cx, &out.path().join("menu.har")), &server);

    let commands = registry();
    let command = commands
        .iter()
        .find(|c| c.id == "request.export_har")
        .expect("a palette command");
    assert_eq!(command.label, "Export response as HAR");
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::tab_area::ExportHar), cx)
    })
    .unwrap();
    check(&answer(cx, &out.path().join("palette.har")), &server);
}

#[gpui_kit::test]
fn the_network_dock_exports_the_selected_entry(cx: &mut TestAppContext) {
    let (server, _col, s) = echo_app(cx);
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    frame(cx, s.win);
    send(cx, &s);
    cx.executor()
        .advance_clock(apinata_app::console_dock::ConsoleDock::POLL);
    cx.run_until_parked();
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    dock.update(cx, |d, cx| d.set_tab(DockTab::Network, cx));
    assert!(!present(cx, s.win, "dock-export-har"), "nothing selected");
    let seq = dock.read_with(cx, |d, _| d.network_shown()[0].seq);
    dock.update(cx, |d, cx| d.select(Some(seq), cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("dock-export-har", cx))
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    check(&answer(cx, &out.path().join("dock.har")), &server);
}

/// Sends once, selects that send in the network dock and returns the dock.
fn send_and_select(
    cx: &mut TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::console_dock::ConsoleDock> {
    VisualTestContext::from_window(s.win, cx).dispatch_action(ToggleConsole);
    frame(cx, s.win);
    send(cx, s);
    cx.executor()
        .advance_clock(apinata_app::console_dock::ConsoleDock::POLL);
    cx.run_until_parked();
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    dock.update(cx, |d, cx| d.set_tab(DockTab::Network, cx));
    let seq = dock.read_with(cx, |d, _| d.network_shown()[0].seq);
    dock.update(cx, |d, cx| d.select(Some(seq), cx));
    frame(cx, s.win);
    dock
}

#[gpui_kit::test]
fn the_dock_and_the_tab_write_the_same_file_for_one_exchange(cx: &mut TestAppContext) {
    let reply = ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .insert_header("x-echo", TOKEN)
        .insert_header("set-cookie", "sid=Cookie-V4lue-91; Path=/")
        .set_body_string(format!("{{\"echo\":\"{TOKEN}\"}}"));
    let (_server, _col, s) = echo_app_with(cx, reply);
    let _dock = send_and_select(cx, &s);
    let out = tempfile::tempdir().unwrap();
    cx.update_window(s.win, |_, w, cx| w.click("response-export-har", cx))
        .unwrap();
    let tab = out.path().join("tab.har");
    answer(cx, &tab);
    cx.update_window(s.win, |_, w, cx| w.click("dock-export-har", cx))
        .unwrap();
    let dock = out.path().join("dock.har");
    answer(cx, &dock);
    let (tab, dock) = (
        std::fs::read_to_string(tab).unwrap(),
        std::fs::read_to_string(dock).unwrap(),
    );
    for text in [&tab, &dock] {
        assert!(!text.contains(TOKEN), "the secret is gone: {text}");
        assert!(
            !text.contains("Cookie-V4lue-91"),
            "the cookie is gone: {text}"
        );
    }
    assert_eq!(dock, tab, "one exchange, one file");
}

#[gpui_kit::test]
fn the_dock_still_exports_redacted_after_its_tab_is_closed(cx: &mut TestAppContext) {
    let (server, _col, s) = echo_app(cx);
    let _dock = send_and_select(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::tab_area::CloseTab), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("dock-export-har", cx))
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    check(&answer(cx, &out.path().join("dock.har")), &server);
}

#[gpui_kit::test]
fn a_response_without_its_request_cannot_export(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let editor = area.read_with(cx, |a, _| a.editor(tab).cloned()).unwrap();
    let send = editor.read_with(cx, |e, _| e.send_model().clone());
    // A response built by hand, like one kept from an earlier session, has no timeline.
    let result = apinata_core::RequestResult::from_parts(
        apinata_core::ItemPath::new("a.yml").unwrap(),
        "a",
        apinata_core::Outcome::Passed,
        Some(apinata_core::Response::new(200).with_text_body("kept")),
        Vec::new(),
    );
    send.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("response-export-har", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        !cx.did_prompt_for_new_path(),
        "a disabled button opens nothing"
    );
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::tab_area::ExportHar), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(
        !cx.did_prompt_for_new_path(),
        "the command says why instead"
    );
}

#[gpui_kit::test]
fn a_binary_body_holding_a_secret_is_not_exported(cx: &mut TestAppContext) {
    let body = [format!("t={TOKEN}").as_bytes(), &[0xff, 0x00, 0xfe]].concat();
    let reply = ResponseTemplate::new(200)
        .insert_header("content-type", "application/octet-stream")
        .set_body_bytes(body);
    let (_server, _col, s) = echo_app_with(cx, reply);
    send(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.click("response-export-har", cx))
        .unwrap();
    let win = s.win;
    wait_real(cx, |cx| {
        cx.update_window(win, |_, w, cx| !w.notifications(cx).is_empty())
            .unwrap()
    });
    assert!(!cx.did_prompt_for_new_path(), "no file is offered");
}

#[gpui_kit::test]
fn a_binary_body_round_trips_through_the_dock_after_its_tab_is_closed(cx: &mut TestAppContext) {
    use base64::Engine as _;
    let body = vec![0x89, b'P', b'N', b'G', 0x00, 0xff, 0xfe, 0x10, 0x00];
    let reply = ResponseTemplate::new(200)
        .insert_header("content-type", "image/png")
        .set_body_bytes(body.clone());
    let (_server, _col, s) = echo_app_with(cx, reply);
    let _dock = send_and_select(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::tab_area::CloseTab), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("dock-export-har", cx))
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let har = answer(cx, &out.path().join("dock.har"));
    let response = &har["log"]["entries"][0]["response"];
    assert_eq!(response["statusText"], "OK");
    assert_eq!(response["content"]["encoding"], "base64");
    assert_eq!(response["content"]["size"], body.len());
    let text = response["content"]["text"].as_str().expect("text");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap();
    assert_eq!(decoded, body);
}

#[gpui_kit::test]
fn the_dock_export_button_is_sized_to_its_label(cx: &mut TestAppContext) {
    let (_server, _col, s) = echo_app(cx);
    let _dock = send_and_select(cx, &s);
    let button = bounds(cx, s.win, "dock-export-har");
    let pane = bounds(cx, s.win, "dock-detail");
    assert!(
        button.size.width * 3. < pane.size.width,
        "the button {button:?} does not stretch across the pane {pane:?}"
    );
    assert!(
        button.left() - pane.left() < gpui_kit::px(24.),
        "{button:?} in {pane:?}"
    );
}

fn bounds(
    cx: &mut TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: &'static str,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}
