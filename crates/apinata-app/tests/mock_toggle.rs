//! The status bar's mock toggle: core's mock server on 127.0.0.1 and a free port, its address
//! shown and copyable, stopped by the toggle or a close, and remembered per collection.

use crate::support;

use apinata_app::mock_server::MockState;
use apinata_app::session::{CollectionSession, Session};
use apinata_app::workspace::Workspace;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;
use support::*;

const USERS: &str = "examples:\n  - name: ok\n    response:\n      status: 200\n      headers:\n        - name: Content-Type\n          value: application/json\n      body:\n        type: json\n        data: '{\"id\": 7}'\n";

fn mocked() -> tempfile::TempDir {
    let users = format!("{}{USERS}", request("users", "http://api.test/users"));
    write_tree(&[("opencollection.yml", ROOT), ("users.yml", &users)])
}

fn start(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
    started(cx, col)
}

fn state(cx: &mut TestAppContext, ws: &gpui_kit::Entity<Workspace>, root: &Path) -> MockState {
    ws.read_with(cx, |w, _| w.mock_state(root))
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| w.within("status-bar").click(id, cx))
        .unwrap();
    frame(cx, win);
}

// Waits for the server to answer, a bounded wait since it binds on a tokio thread.
fn running(cx: &mut TestAppContext, ws: &gpui_kit::Entity<Workspace>, root: &Path) -> SocketAddr {
    wait_real(cx, |cx| matches!(state(cx, ws, root), MockState::On(_)));
    match state(cx, ws, root) {
        MockState::On(addr) => addr,
        other => panic!("not running: {other:?}"),
    }
}

// A plain HTTP/1.1 GET, so the test needs no client of its own; the reply is read to close.
fn get(addr: SocketAddr, path: &str) -> String {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = String::new();
    s.read_to_string(&mut reply).unwrap();
    reply
}

fn refused(addr: SocketAddr) -> bool {
    TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err()
}

fn label(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

fn pressed(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<bool> {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find("mock-toggle").and_then(|e| e.checked())
    })
    .unwrap()
}

// A port free right now, so a start that binds it twice would fail with the port in use.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn remembered(cx: &mut TestAppContext, ws: &gpui_kit::Entity<Workspace>, root: &Path) -> bool {
    let session = ws.read_with(cx, |w, _| w.session().clone());
    let value = session_value(cx, &session);
    value.collections.get(root).is_some_and(|c| c.mock)
}

#[gpui_kit::test]
fn the_toggle_serves_the_examples_on_loopback_and_shows_the_address(cx: &mut TestAppContext) {
    let col = mocked();
    let s = start(cx, &col);
    assert_eq!(state(cx, &s.ws, &s.root), MockState::Off);
    assert!(present(cx, s.win, "mock-toggle"));
    assert!(!present(cx, s.win, "mock-address"));
    assert_eq!(pressed(cx, s.win), Some(false));
    click(cx, s.win, "mock-toggle");
    let addr = running(cx, &s.ws, &s.root);
    assert_eq!(pressed(cx, s.win), Some(true));
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    assert_ne!(addr.port(), 0);
    let url = format!("http://{addr}");
    assert_eq!(
        label(cx, s.win, "mock-address").as_deref(),
        Some(url.as_str())
    );
    let reply = get(addr, "/users");
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("{\"id\": 7}"), "{reply}");
    assert!(remembered(cx, &s.ws, &s.root));
}

#[gpui_kit::test]
fn the_copy_button_copies_the_address(cx: &mut TestAppContext) {
    let col = mocked();
    let s = start(cx, &col);
    click(cx, s.win, "mock-toggle");
    let addr = running(cx, &s.ws, &s.root);
    frame(cx, s.win);
    click(cx, s.win, "mock-copy");
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied, Some(format!("http://{addr}")));
}

#[gpui_kit::test]
fn toggling_off_stops_the_server_and_releases_the_port(cx: &mut TestAppContext) {
    let col = mocked();
    let s = start(cx, &col);
    click(cx, s.win, "mock-toggle");
    let addr = running(cx, &s.ws, &s.root);
    frame(cx, s.win);
    click(cx, s.win, "mock-toggle");
    assert_eq!(state(cx, &s.ws, &s.root), MockState::Off);
    wait_real(cx, |_| refused(addr));
    assert!(!present(cx, s.win, "mock-address"));
    assert!(!remembered(cx, &s.ws, &s.root));
}

#[gpui_kit::test]
fn a_failed_start_shows_an_error_and_leaves_the_toggle_off(cx: &mut TestAppContext) {
    let col = mocked();
    let s = start(cx, &col);
    // A port already taken makes core's bind fail, the one failure a real start can meet.
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    s.ws.update(cx, |w, _| w.set_mock_port(port));
    click(cx, s.win, "mock-toggle");
    wait_real(cx, |cx| present(cx, s.win, "toast-error"));
    assert_eq!(state(cx, &s.ws, &s.root), MockState::Off);
    assert!(!present(cx, s.win, "mock-address"));
    assert!(!remembered(cx, &s.ws, &s.root));
}

#[gpui_kit::test]
fn a_remembered_toggle_starts_again_on_reopen_and_a_close_stops_it(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
    let col = mocked();
    let root = col.path().to_path_buf();
    let state_dir = tempfile::tempdir().unwrap();
    let mut saved = Session::default();
    saved.collections.insert(
        root.clone(),
        CollectionSession {
            mock: true,
            ..Default::default()
        },
    );
    let session = session_at(cx, &state_dir, saved);
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    let addr = running(cx, &ws, &root);
    assert!(get(addr, "/users").starts_with("HTTP/1.1 200"));
    ws.update(cx, |w, cx| w.close(&root, cx));
    assert_eq!(state(cx, &ws, &root), MockState::Off);
    wait_real(cx, |_| refused(addr));
    // Closing is not turning it off: the collection keeps its choice for the next open.
    assert!(remembered(cx, &ws, &root));
}

#[gpui_kit::test]
fn hovering_the_toggle_explains_it(cx: &mut TestAppContext) {
    let col = mocked();
    let s = start(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.hover("mock-toggle", cx))
        .unwrap();
    cx.executor().advance_clock(Duration::from_secs(2));
    assert_eq!(
        label(cx, s.win, "mock-tooltip").as_deref(),
        Some(apinata_app::mock_server::MOCK_TIP)
    );
}

#[test]
fn the_session_keeps_the_choice_only_when_on() {
    let on = CollectionSession {
        mock: true,
        ..Default::default()
    };
    let text = serde_json::to_string(&on).unwrap();
    assert!(text.contains("\"mock\":true"), "{text}");
    let off = serde_json::to_string(&CollectionSession::default()).unwrap();
    assert!(!off.contains("mock"), "{off}");
    let back: CollectionSession = serde_json::from_str(&text).unwrap();
    assert!(back.mock);
}

#[gpui_kit::test]
fn turning_off_during_a_start_leaves_no_server_and_no_toast(cx: &mut TestAppContext) {
    let col = mocked();
    let s = start(cx, &col);
    let port = free_port();
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.set_mock_port(port);
        w.set_mock(&root, true, cx);
        assert_eq!(w.mock_state(&root), MockState::Starting);
        w.set_mock(&root, false, cx);
    });
    cx.run_until_parked();
    assert_eq!(state(cx, &s.ws, &s.root), MockState::Off);
    assert!(!present(cx, s.win, "toast-error"));
    assert!(!remembered(cx, &s.ws, &s.root));
    // The abandoned start holds nothing: a new one binds the same fixed port.
    s.ws.update(cx, |w, cx| w.set_mock(&root, true, cx));
    let addr = running(cx, &s.ws, &s.root);
    assert_eq!(addr.port(), port);
    assert!(!present(cx, s.win, "toast-error"));
}

#[gpui_kit::test]
fn on_off_on_in_quick_succession_leaves_exactly_one_server(cx: &mut TestAppContext) {
    let col = mocked();
    let s = start(cx, &col);
    let port = free_port();
    let root = s.root.clone();
    // On a fixed port a second live server could not bind, so one success means one server.
    s.ws.update(cx, |w, cx| {
        w.set_mock_port(port);
        w.set_mock(&root, true, cx);
        w.set_mock(&root, false, cx);
        w.set_mock(&root, true, cx);
    });
    let addr = running(cx, &s.ws, &s.root);
    assert_eq!(addr.port(), port);
    assert!(get(addr, "/users").starts_with("HTTP/1.1 200"));
    assert!(!present(cx, s.win, "toast-error"));
    assert!(remembered(cx, &s.ws, &s.root));
    s.ws.update(cx, |w, cx| w.set_mock(&root, false, cx));
    wait_real(cx, |_| refused(addr));
}
