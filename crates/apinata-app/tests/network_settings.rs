//! The global proxy and CA settings, the executor built from them, and the error titles.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::executor::{self, network_settings, proxy_password_key};
use apinata_app::response_pane::ResponsePane;
use apinata_app::settings::{ProxyMode, ProxyPrefs, Settings};
use apinata_core::{
    GlobalProxy, MemoryStore, NetworkSettings, PacSource, ResolvedProxy, SecretError, SecretKey,
    SecretStore,
};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use std::path::PathBuf;
use std::time::Duration;
use support::*;

fn manual(host: &str, port: &str, username: &str) -> Settings {
    Settings {
        proxy: ProxyPrefs {
            mode: ProxyMode::Manual,
            protocol: "http".into(),
            host: host.into(),
            port: port.into(),
            username: username.into(),
            bypass: "localhost".into(),
            pac_source: String::new(),
        },
        ..Settings::default()
    }
}

fn closed_port() -> u16 {
    dead_port()
}

fn error_of(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
) -> apinata_app::response_pane::ErrorView {
    pane_after_send(cx, col)
        .read_with(cx, |p, cx| p.error_view(cx))
        .expect("an error")
}

// Sends `a.yml` of `col` on the installed executor and opens a response pane over the result.
fn pane_after_send(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
) -> gpui_kit::Entity<ResponsePane> {
    let (_state, ws, root) = open_dir(cx, col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    finished(cx, &model);
    support::init(cx);
    let m = model.clone();
    let (_win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    view
}

#[test]
fn defaults_hold_for_old_files_and_no_password_is_stored() {
    let fresh = Settings::default();
    assert_eq!(
        (fresh.proxy.mode, fresh.proxy.protocol.as_str()),
        (ProxyMode::System, "http")
    );
    assert!(fresh.keep_default_cas && fresh.ca_file.is_none());
    let old: Settings = serde_json::from_str(r#"{"insecure":true}"#).unwrap();
    assert_eq!(
        (old.proxy, old.ca_file, old.keep_default_cas),
        (fresh.proxy, None, true)
    );
    let json = serde_json::to_string(&manual("p", "1", "u")).unwrap();
    assert!(
        json.contains(r#""mode":"manual""#) && !json.contains("password"),
        "{json}"
    );
}

#[test]
fn the_global_layer_follows_the_settings() {
    assert_eq!(
        network_settings(&Settings::default(), None),
        Ok(NetworkSettings::new())
    );
    let mut off = Settings::default();
    off.proxy.mode = ProxyMode::Off;
    off.ca_file = Some(PathBuf::from("/ca.pem"));
    off.keep_default_cas = false;
    let want = NetworkSettings::new()
        .proxy(GlobalProxy::Off)
        .ca_file(Some(PathBuf::from("/ca.pem")))
        .keep_default_roots(false);
    assert_eq!(network_settings(&off, None), Ok(want));
    let proxy = ResolvedProxy::new("http", "127.0.0.1", "3128").unwrap();
    let with = |proxy: ResolvedProxy| {
        Ok(NetworkSettings::new().proxy(GlobalProxy::Manual {
            proxy,
            bypass: "localhost".into(),
        }))
    };
    assert_eq!(
        network_settings(&manual("127.0.0.1", "3128", "u"), Some("pw".into())),
        with(proxy.clone().with_credentials("u", "pw"))
    );
    assert_eq!(
        network_settings(&manual("127.0.0.1", "3128", ""), Some("pw".into())),
        with(proxy)
    );
    let err = network_settings(&manual("", "3128", "u"), None).unwrap_err();
    assert!(err.contains("invalid proxy"), "{err}");
}

struct LockedStore;

impl SecretStore for LockedStore {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Ok(())
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
}

fn tokio(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
}

#[gpui_kit::test]
fn the_test_harness_runs_with_the_proxy_off(cx: &mut TestAppContext) {
    assert_eq!(test_settings().proxy.mode, ProxyMode::Off);
    let col = two_requests();
    let _s = started(cx, &col);
    let mode = cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().proxy.mode);
    assert_eq!(mode, ProxyMode::Off);
}

#[gpui_kit::test]
fn install_reads_the_password_only_for_a_manual_proxy_with_a_user(cx: &mut TestAppContext) {
    tokio(cx);
    let store = CountingStore::new(Duration::ZERO);
    let notes = cx
        .update(|cx| executor::install(&Settings::default(), &store, cx))
        .unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    cx.update(|cx| executor::install(&manual("127.0.0.1", "3128", ""), &store, cx))
        .unwrap();
    assert_eq!(store.gets(), 0);
    let notes = cx
        .update(|cx| executor::install(&manual("127.0.0.1", "3128", "u"), &store, cx))
        .unwrap();
    assert_eq!((store.gets(), notes.len()), (1, 0));
}

#[gpui_kit::test]
fn a_half_configured_manual_proxy_falls_back_to_the_system_proxy(cx: &mut TestAppContext) {
    tokio(cx);
    let notes = cx
        .update(|cx| executor::install(&manual("", "3128", "u"), &MemoryStore::new(), cx))
        .unwrap();
    assert!(cx.update(|cx| executor::shared(cx)).is_some());
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].starts_with("Manual proxy ignored (invalid proxy"),
        "{}",
        notes[0]
    );
    assert!(
        notes[0].ends_with("); using the system proxy."),
        "{}",
        notes[0]
    );
}

#[gpui_kit::test]
fn a_locked_keychain_sends_without_the_password(cx: &mut TestAppContext) {
    tokio(cx);
    let notes = cx
        .update(|cx| executor::install(&manual("127.0.0.1", "3128", "u"), &LockedStore, cx))
        .unwrap();
    assert!(cx.update(|cx| executor::shared(cx)).is_some());
    assert_eq!(
        notes,
        ["Proxy password unavailable (secret store unavailable: locked); sending without it."]
    );
}

#[gpui_kit::test]
fn a_dead_manual_proxy_is_a_proxy_error_without_the_password(cx: &mut TestAppContext) {
    tokio(cx);
    let proxy_port = closed_port();
    let store = MemoryStore::new();
    store.set(&proxy_password_key(), "hunter2-secret").unwrap();
    let settings = manual("127.0.0.1", &proxy_port.to_string(), "u");
    cx.update(|cx| executor::install(&settings, &store, cx))
        .unwrap();
    let target = format!("http://127.0.0.1:{}/ok", closed_port());
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", &target)),
    ]);
    let shown = error_of(cx, &col);
    assert_eq!(shown.title, "Proxy error");
    let message = shown.message.unwrap_or_default();
    assert!(
        message.contains(&format!("http://127.0.0.1:{proxy_port}")),
        "{message}"
    );
    assert!(!message.contains("hunter2-secret"), "{message}");
}

#[gpui_kit::test]
fn a_missing_client_certificate_is_a_certificate_error(cx: &mut TestAppContext) {
    http_ready(cx);
    let root = format!(
        "{ROOT}config:\n  clientCertificates:\n    - domain: 127.0.0.1\n      type: pem\n      certificateFilePath: missing.pem\n      privateKeyFilePath: missing.key\n"
    );
    let target = format!("https://127.0.0.1:{}/ok", closed_port());
    let col = write_tree(&[
        ("opencollection.yml", &root),
        ("a.yml", &request("a", &target)),
    ]);
    let shown = error_of(cx, &col);
    assert_eq!(shown.title, "Client certificate error");
    assert!(shown.message.unwrap_or_default().contains("missing.pem"));
}

fn pac(source: &str) -> Settings {
    let mut settings = Settings::default();
    settings.proxy.mode = ProxyMode::Pac;
    settings.proxy.pac_source = source.into();
    settings
}

#[test]
fn pac_mode_round_trips_and_old_files_load_with_an_empty_source() {
    let old: ProxyPrefs = serde_json::from_str(r#"{"mode":"manual","host":"p"}"#).unwrap();
    assert_eq!((old.mode, old.pac_source.as_str()), (ProxyMode::Manual, ""));
    let settings = pac("https://example.com/proxy.pac");
    let json = serde_json::to_string(&settings).unwrap();
    assert!(
        json.contains(r#""mode":"pac""#)
            && json.contains(r#""pac_source":"https://example.com/proxy.pac""#),
        "{json}"
    );
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.proxy, settings.proxy);
}

#[test]
fn pac_mode_parses_the_source_without_a_base_directory() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("proxy.pac");
    let file = file.to_str().unwrap();
    let source = PacSource::parse(file, None).unwrap();
    assert_eq!(
        network_settings(&pac(file), None),
        Ok(NetworkSettings::new().proxy(GlobalProxy::Pac { source }))
    );
    let url = "https://example.com/proxy.pac";
    let source = PacSource::parse(url, None).unwrap();
    assert_eq!(
        network_settings(&pac(url), Some("unused".into())),
        Ok(NetworkSettings::new().proxy(GlobalProxy::Pac { source }))
    );
    assert_eq!(
        network_settings(&pac(" "), None),
        Err("the PAC source is empty".to_owned())
    );
    assert_eq!(
        network_settings(&pac("proxy.pac"), None),
        Err(
            "the PAC path proxy.pac is relative and there is no directory to resolve it against"
                .to_owned()
        )
    );
}

#[gpui_kit::test]
fn an_unusable_pac_source_falls_back_to_the_system_proxy_without_the_keychain(
    cx: &mut TestAppContext,
) {
    tokio(cx);
    let store = CountingStore::new(Duration::ZERO);
    let notes = cx
        .update(|cx| executor::install(&pac("proxy.pac"), &store, cx))
        .unwrap();
    assert!(cx.update(|cx| executor::shared(cx)).is_some());
    assert_eq!(
        notes,
        [
            "PAC file ignored (the PAC path proxy.pac is relative and there is no directory to \
          resolve it against); using the system proxy."
        ]
    );
    let mut good = pac("https://example.com/proxy.pac");
    good.proxy.username = "u".into();
    let notes = cx
        .update(|cx| executor::install(&good, &store, cx))
        .unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(store.gets(), 0);
}

#[gpui_kit::test]
fn a_pac_source_with_credentials_is_refused_and_never_echoed(cx: &mut TestAppContext) {
    // A file URL cannot carry userinfo, so its `u:pw@` is refused as credentials too.
    for source in [
        "https://u:pw-SEKRIT@example.com/proxy.pac",
        "https://pw-SEKRIT@example.com/proxy.pac",
        "file://u:pw-SEKRIT@h/x",
    ] {
        assert_eq!(
            executor::parse_pac(source).map(|_| ()),
            Err("the PAC URL must not contain credentials".to_owned())
        );
    }
    assert!(executor::parse_pac("https://example.com/p.pac").is_ok());
    tokio(cx);
    let notes = cx
        .update(|cx| {
            executor::install(
                &pac("https://u:pw-SEKRIT@example.com/proxy.pac"),
                &MemoryStore::new(),
                cx,
            )
        })
        .unwrap();
    assert_eq!(
        notes,
        ["PAC file ignored (the PAC URL must not contain credentials); using the system proxy."]
    );
}

#[gpui_kit::test]
fn a_pac_send_shows_its_route_on_the_timeline_and_its_fallback_as_a_warning(
    cx: &mut TestAppContext,
) {
    tokio(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("direct.pac");
    std::fs::write(
        &script,
        r#"function FindProxyForURL(url, host) { return "DIRECT"; }"#,
    )
    .unwrap();
    cx.update(|cx| executor::install(&pac(script.to_str().unwrap()), &MemoryStore::new(), cx))
        .unwrap();
    let pane = pane_after_send(cx, &col);
    let (lines, warnings) = pane.read_with(cx, |p, cx| (p.hop_pac_lines(cx), p.warnings(cx)));
    assert_eq!(lines, ["PAC: DIRECT"]);
    assert!(warnings.is_empty(), "{warnings:?}");

    let missing = dir.path().join("missing.pac");
    cx.update(|cx| executor::install(&pac(missing.to_str().unwrap()), &MemoryStore::new(), cx))
        .unwrap();
    let pane = pane_after_send(cx, &col);
    let (lines, warnings) = pane.read_with(cx, |p, cx| (p.hop_pac_lines(cx), p.warnings(cx)));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with("Proxy auto-config could not be fetched: ")
            && warnings[0].ends_with("; connected directly"),
        "{warnings:?}"
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("PAC: could not be fetched: "),
        "{lines:?}"
    );
}
