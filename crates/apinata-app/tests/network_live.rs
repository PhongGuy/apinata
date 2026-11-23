//! Network preferences reach the next send without a restart.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::executor::{self, proxy_password_key};
use apinata_app::settings::{ProxyMode, ProxyPrefs, Settings};
use apinata_core::{MemoryStore, SecretStore};
use gpui_kit::TestAppContext;
use std::sync::Arc;
use support::*;

fn requests_to(cx: &mut TestAppContext, server: &wiremock::MockServer) -> Vec<wiremock::Request> {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle
        .block_on(server.received_requests())
        .unwrap_or_default()
}

// Sends one GET to `target` through whatever shared executor is installed now.
fn send_now(cx: &mut TestAppContext, target: &wiremock::MockServer) {
    let exec = cx.update(|cx| executor::shared(cx)).unwrap();
    send_direct(cx, &exec, &target.uri());
}

fn edit_settings(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

fn manual_proxy(proxy: &wiremock::MockServer, username: &str) -> ProxyPrefs {
    let addr = proxy.address();
    ProxyPrefs {
        mode: ProxyMode::Manual,
        protocol: "http".into(),
        host: addr.ip().to_string(),
        port: addr.port().to_string(),
        username: username.into(),
        bypass: String::new(),
        pac_source: String::new(),
    }
}

fn ready(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
}

#[gpui_kit::test]
fn a_proxy_chosen_in_preferences_carries_the_next_send(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let _s = started(cx, &col);
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    send_now(cx, &target);
    assert_eq!(requests_to(cx, &proxy).len(), 0);
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, ""));
    send_now(cx, &target);
    assert_eq!(requests_to(cx, &proxy).len(), 1);
    assert_eq!(requests_to(cx, &target).len(), 1);
}

#[gpui_kit::test]
fn a_proxy_user_chosen_in_preferences_sends_the_stored_password(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let store = Arc::new(MemoryStore::new());
    store.set(&proxy_password_key(), "p1").unwrap();
    let _s = started_with_store(cx, &col, store);
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, "u"));
    send_now(cx, &target);
    let got = requests_to(cx, &proxy);
    assert_eq!(got.len(), 1);
    // Basic base64 of "u:p1".
    let auth = got[0]
        .headers
        .get("proxy-authorization")
        .map(|v| v.to_str().unwrap().to_owned());
    assert_eq!(auth.as_deref(), Some("Basic dTpwMQ=="));
}

#[gpui_kit::test]
fn turning_the_proxy_off_again_sends_direct(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let _s = started(cx, &col);
    let target = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let proxy = mock_server(cx, wiremock::ResponseTemplate::new(200));
    edit_settings(cx, |s| s.proxy = manual_proxy(&proxy, ""));
    edit_settings(cx, |s| s.proxy.mode = ProxyMode::Off);
    send_now(cx, &target);
    assert_eq!(requests_to(cx, &proxy).len(), 0);
    assert_eq!(requests_to(cx, &target).len(), 1);
}

// The response size limit is part of the rebuilt executor too: a smaller cap cuts the next body.
#[gpui_kit::test]
fn a_smaller_body_limit_applies_to_the_next_send(cx: &mut TestAppContext) {
    ready(cx);
    let col = two_requests();
    let _s = started(cx, &col);
    let body = "x".repeat(4096);
    let target = mock_server(
        cx,
        wiremock::ResponseTemplate::new(200).set_body_string(body),
    );
    edit_settings(cx, |s| s.max_body_bytes = Some(16));
    let exec = cx.update(|cx| executor::shared(cx)).unwrap();
    let col = collection_for(&target.uri(), "");
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let ctx = apinata_core::VarContext::new(&c, apinata_core::ContextOptions::new()).unwrap();
    let req = apinata_core::resolve(c.request(&ip("a.yml")).unwrap(), &ctx).unwrap();
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let resp = handle.block_on(exec.send(&req)).unwrap();
    assert!(resp.body().len() <= 16, "the 16-byte limit did not apply");
}
