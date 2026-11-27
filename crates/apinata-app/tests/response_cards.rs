//! The response status line's hover cards: the status in words, the timing breakdown on the
//! time, header and body size on the size, and how the answer travelled on the address.

use crate::support;

use apinata_app::response_pane::ResponsePane;
use apinata_app::send_model::Restored;
use apinata_app::timing_bar::{Phases, tooltip_lines};
use apinata_core::{Outcome, RequestResult, Response, status_description};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use std::sync::Arc;
use std::time::Duration;
use support::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject as _;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use wiremock::ResponseTemplate;

/// A self-signed certificate for `localhost` and `127.0.0.1`, valid until 2126.
const CERT: &str = "
-----BEGIN CERTIFICATE-----
MIIBoTCCAUegAwIBAgIUFIBqvfCcF05xnBExaqLFzAG96MYwCgYIKoZIzj0EAwIw
FzEVMBMGA1UEAwwMYXBpbmF0YS10ZXN0MCAXDTI2MTAwOTA1MDMxM1oYDzIxMjYw
OTE1MDUwMzEzWjAXMRUwEwYDVQQDDAxhcGluYXRhLXRlc3QwWTATBgcqhkjOPQIB
BggqhkjOPQMBBwNCAAT4LqVaRLaBVS+l6zG3D0hJhiOKySg8X6J3r+7NoCxM/7Ua
jl+teuS/Hv8sS4b5DrjNwdg5S4yigSdWk56f/SWBo28wbTAdBgNVHQ4EFgQUsYA1
fOv6AaZgpupbIYQppcsxadYwHwYDVR0jBBgwFoAUsYA1fOv6AaZgpupbIYQppcsx
adYwDwYDVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEw
CgYIKoZIzj0EAwIDSAAwRQIgW78PGr1EL47OJvRV50d7nfIDyL+PTV1QciVzwFeG
pZoCIQCQ5L5QFFNh13q0EVPIK+kzXpBaCRlz989/cAVRwiaWKg==
-----END CERTIFICATE-----
";
/// The certificate's private key; it protects nothing.
const KEY: &str = "
-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg7OQzuCwTpdphOUYm
iWiOvC4cK98jZlaqF0EQgsdGN7ahRANCAAT4LqVaRLaBVS+l6zG3D0hJhiOKySg8
X6J3r+7NoCxM/7Uajl+teuS/Hv8sS4b5DrjNwdg5S4yigSdWk56f/SWB
-----END PRIVATE KEY-----
";

// A wide window, so the status line keeps its address rather than folding to compact.
fn pane(cx: &mut TestAppContext, s: &Sent) -> (AnyWindowHandle, Entity<ResponsePane>) {
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..WindowOptions::default()
            };
            gpui_kit::open_window(options, cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view)
}

/// The text of tooltip `tip` after hovering `target`; `None` when no such tooltip shows.
fn card(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    target: &'static str,
    tip: &'static str,
) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(target, cx)
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(tip).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_status_card_says_what_the_code_means(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(201), "");
    let (win, _view) = pane(cx, &s);
    assert_eq!(
        card(cx, win, "response-status", "response-status-tooltip").as_deref(),
        status_description(201)
    );
}

#[gpui_kit::test]
fn the_time_card_is_the_timing_breakdown(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string("ok"), "");
    let (win, _view) = pane(cx, &s);
    let timing = s.result.response.as_ref().unwrap().timeline()[0]
        .timing
        .clone()
        .expect("timed");
    let expected = tooltip_lines(&Phases::from(&timing)).join("\n");
    assert_eq!(
        card(cx, win, "response-time", "response-time-tooltip"),
        Some(expected)
    );
}

#[gpui_kit::test]
fn the_size_card_splits_header_and_body_size(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_raw("hello", "text/plain"),
        "",
    );
    let (win, _view) = pane(cx, &s);
    let head = s.result.response.as_ref().unwrap().timeline()[0].response_header_size();
    assert!(head > 0);
    assert_eq!(
        card(cx, win, "response-size", "response-size-tooltip"),
        Some(format!("Headers  {head} B\nBody  5 B"))
    );
}

#[gpui_kit::test]
fn the_connection_card_over_http_has_addresses_and_no_tls(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let (win, _view) = pane(cx, &s);
    let hop = &s.result.response.as_ref().unwrap().timeline()[0];
    let local = hop.local_addr().expect("local address");
    let expected = format!(
        "Remote  {}\nLocal  {local}\nVersion  HTTP/1.1",
        s.server.address()
    );
    assert_eq!(
        card(cx, win, "response-address", "response-address-tooltip"),
        Some(expected)
    );
}

/// Serve `ok` over TLS on 127.0.0.1 with the fixture certificate; returns the base URL.
fn https_server(cx: &mut TestAppContext) -> String {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from_pem_slice(CERT.as_bytes()).expect("cert")],
            PrivateKeyDer::from_pem_slice(KEY.as_bytes()).expect("key"),
        )
        .expect("tls config");
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = handle
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    handle.spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.expect("accept");
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = vec![0; 4096];
                let _ = tls.read(&mut buf).await;
                let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";
                let _ = tls.write_all(reply).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    format!("https://127.0.0.1:{port}")
}

#[gpui_kit::test]
fn the_connection_card_over_https_adds_the_tls_block(cx: &mut TestAppContext) {
    http_ready(cx);
    // The fixture certificate is self-signed, so the shared executor skips verification.
    let mut settings = test_settings();
    settings.insecure = true;
    cx.update(|cx| {
        apinata_app::executor::install(&settings, &apinata_core::MemoryStore::new(), cx)
    })
    .unwrap();
    let base = https_server(cx);
    let col = collection_for(&base, "");
    let (state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    let hop = result.response.as_ref().expect("answered").timeline()[0].clone();
    let s = Sent {
        server: mock_server(cx, ResponseTemplate::new(200)),
        col,
        state,
        ws,
        root,
        model,
        result,
    };
    let (win, _view) = pane(cx, &s);
    let text = card(cx, win, "response-address", "response-address-tooltip").expect("a card");
    let expected = format!(
        "Remote  {}\nLocal  {}\nVersion  HTTP/1.1\nTLS  TLS 1.3\nCertificate  not verified\n\
         Subject  CN=apinata-test\n\
         Issuer  CN=apinata-test\nExpires  2126-09-15 05:03:13 UTC",
        base.trim_start_matches("https://"),
        hop.local_addr().expect("local address"),
    );
    assert_eq!(text, expected);
}

#[gpui_kit::test]
fn a_kept_response_has_no_connection_card(cx: &mut TestAppContext) {
    // A kept response is shown only on a model that has not sent, as on opening a request.
    let s = send_once(cx, ResponseTemplate::new(200), "");
    let idle = sender(cx, &s.ws);
    let kept = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body("ok")),
        Vec::new(),
    );
    let restored = Restored {
        saved_at: std::time::UNIX_EPOCH,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    idle.update(cx, |m, cx| m.restore(Arc::new(kept), restored, cx));
    let s = Sent { model: idle, ..s };
    let (win, _view) = pane(cx, &s);
    assert!(present(cx, win, "response-status"));
    assert!(!present(cx, win, "response-address"));
    assert!(!present(cx, win, "response-address-tooltip"));
    // A kept response never knew its header block, so its size card has the body alone.
    assert_eq!(
        card(cx, win, "response-size", "response-size-tooltip").as_deref(),
        Some("Body  2 B")
    );
}
