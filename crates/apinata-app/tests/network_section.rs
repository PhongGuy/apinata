//! The collection's Network tab: the proxy choice and fields, and moving plain secrets.

use crate::support;

use apinata_app::command::Edit;
use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::network_section::{NetworkSection, ProxyChoice};
use apinata_app::workspace::Workspace;
use apinata_core::{CollectionNetwork, CollectionProxy, ProxyAuth, ProxyConfig};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use std::path::{Path, PathBuf};
use support::*;

type Host = Entity<SectionHost<NetworkSection>>;

fn section(
    cx: &mut TestAppContext,
    network: CollectionNetwork,
    plain: bool,
) -> (AnyWindowHandle, Host) {
    let (win, host) = host_section(cx, |w, cx| NetworkSection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, plain, w, cx))
    })
    .unwrap();
    frame(cx, win);
    (win, host)
}

fn with_proxy(proxy: CollectionProxy) -> CollectionNetwork {
    let mut n = CollectionNetwork::default();
    n.proxy = proxy;
    n
}

fn custom(protocol: &str, host: &str, auth: Option<(&str, &str)>) -> CollectionProxy {
    let mut p = ProxyConfig::new(protocol, host, "3128");
    p.auth = auth.map(|(u, pw)| ProxyAuth::new(u, pw));
    CollectionProxy::Custom(p)
}

fn last_proxy(cx: &mut TestAppContext, host: &Host) -> Option<CollectionProxy> {
    host.read_with(cx, |h, _| {
        h.edits.iter().rev().find_map(|e| match e {
            Edit::Proxy(p) => Some(p.clone()),
            _ => None,
        })
    })
}

// Tells the section that `.env` defines `stored`.
fn stored_names(cx: &mut TestAppContext, win: AnyWindowHandle, host: &Host, stored: &[&str]) {
    let s = host.read_with(cx, |h, _| h.view.clone());
    let names = apinata_app::snapshot::DotenvNames {
        stored: std::sync::Arc::new(stored.iter().map(|n| n.to_string()).collect()),
        held: Default::default(),
    };
    cx.update_window(win, |_, _, cx| {
        s.update(cx, |s, cx| s.set_dotenv(names, cx))
    })
    .unwrap();
    frame(cx, win);
}

fn edit_count(cx: &mut TestAppContext, host: &Host) -> usize {
    host.read_with(cx, |h, _| h.edits.len())
}

fn segment(cx: &mut TestAppContext, win: AnyWindowHandle, group: &'static str, ix: usize) {
    cx.update_window(win, |_, w, cx| w.within(group).click(ix, cx))
        .unwrap();
    frame(cx, win);
}

fn root_with_password(password: &str) -> String {
    format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: proxy.local\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"{password}\"\n      bypassProxy: \"\"\n"
    )
}

fn editor(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    root: &Path,
    target: ConfigTarget,
) -> (AnyWindowHandle, Entity<ConfigEditor>) {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws, root, target, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view)
}

#[gpui_kit::test]
fn loading_shows_the_choice_without_sending(cx: &mut TestAppContext) {
    let (win, host) = section(cx, with_proxy(custom("http", "proxy.local", None)), false);
    let s = host.read_with(cx, |h, _| h.view.clone());
    assert_eq!(s.read_with(cx, |s, _| s.choice()), ProxyChoice::Custom);
    assert!(present(cx, win, "proxy-host"));
    assert_eq!(edit_count(cx, &host), 0);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.load(&CollectionNetwork::default(), false, w, cx)
        })
    })
    .unwrap();
    frame(cx, win);
    assert!(!present(cx, win, "proxy-host"));
    assert_eq!(edit_count(cx, &host), 0);
}

#[gpui_kit::test]
fn the_choice_sends_the_proxy(cx: &mut TestAppContext) {
    let (win, host) = section(cx, with_proxy(custom("http", "proxy.local", None)), false);
    segment(cx, win, "proxy-mode", 1);
    assert_eq!(last_proxy(cx, &host), Some(CollectionProxy::Off));
    segment(cx, win, "proxy-mode", 2);
    assert_eq!(
        last_proxy(cx, &host),
        Some(custom("http", "proxy.local", None))
    );
    segment(cx, win, "proxy-mode", 0);
    assert_eq!(last_proxy(cx, &host), Some(CollectionProxy::Inherit));
}

#[gpui_kit::test]
fn typing_sends_the_custom_fields(cx: &mut TestAppContext) {
    let (win, host) = section(cx, with_proxy(custom("http", "proxy.local", None)), false);
    type_at_end(cx, win, None, "proxy-host", "x");
    cx.run_until_parked();
    assert_eq!(
        last_proxy(cx, &host),
        Some(custom("http", "proxy.localx", None))
    );
    segment(cx, win, "proxy-protocol", 3);
    assert_eq!(
        last_proxy(cx, &host),
        Some(custom("socks5", "proxy.localx", None))
    );
}

#[gpui_kit::test]
fn the_password_is_sent_on_enter_only(cx: &mut TestAppContext) {
    let (win, host) = section(
        cx,
        with_proxy(custom("http", "proxy.local", Some(("u", "")))),
        false,
    );
    let before = edit_count(cx, &host);
    type_at_end(cx, win, None, "proxy-password", "pw");
    cx.run_until_parked();
    assert_eq!(edit_count(cx, &host), before);
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        last_proxy(cx, &host),
        Some(custom("http", "proxy.local", Some(("u", "pw"))))
    );
}

#[gpui_kit::test]
fn plain_secrets_offer_a_move(cx: &mut TestAppContext) {
    let (win, host) = section(
        cx,
        with_proxy(custom("http", "proxy.local", Some(("u", "pw")))),
        true,
    );
    assert!(present(cx, win, "network-plain-notice"));
    cx.update_window(win, |_, w, cx| w.click("network-move-secrets", cx))
        .unwrap();
    cx.run_until_parked();
    let moved = host.read_with(cx, |h, _| {
        matches!(h.edits.last(), Some(Edit::MoveNetworkSecrets))
    });
    assert!(moved);
}

#[gpui_kit::test]
fn the_collection_settings_have_a_network_tab(cx: &mut TestAppContext) {
    use ConfigSection::*;
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    assert_eq!(
        ed.read_with(cx, |e, _| e.sections()),
        [Headers, Auth, Vars, Script, Tests, Docs, Network, Presets]
    );
    let ix = ed.read_with(cx, |e, _| e.sections().iter().position(|x| *x == Network));
    cx.update_window(win, |_, w, cx| w.click(("config-section", ix.unwrap()), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(ed.read_with(cx, |e, _| e.section()), Network);
    assert!(present(cx, win, "config-network") && present(cx, win, "proxy-mode"));
}

#[gpui_kit::test]
fn move_to_dotenv_clears_the_plain_text_notice(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", &root_with_password("hunter2"))]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    assert!(present(cx, win, "network-plain-notice"));
    cx.update_window(win, |_, w, cx| w.click("network-move-secrets", cx))
        .unwrap();
    frame(cx, win);
    assert!(!ws_snap(cx, &ws, &root).content.collection.plain_secrets);
    assert!(!col.path().join(".env").exists());
    ed.update(cx, |e, cx| e.save(cx));
    cx.run_until_parked();
    frame(cx, win);
    let dotenv = std::fs::read_to_string(col.path().join(".env")).unwrap();
    assert!(
        dotenv.contains("APINATA_PROXY_PASSWORD='hunter2'"),
        "{dotenv}"
    );
    assert!(!present(cx, win, "network-plain-notice"));
}

#[gpui_kit::test]
fn a_typed_password_reaches_dotenv_on_save(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    segment(cx, win, "proxy-mode", 2);
    type_at_end(cx, win, None, "proxy-host", "proxy.local");
    type_at_end(cx, win, None, "proxy-password", "pw");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, win);
    let network = ed
        .read_with(cx, |e, _| e.network().cloned())
        .expect("a network section");
    let notice = network.read_with(cx, |n, _| n.notice().map(str::to_owned));
    assert_eq!(notice, None);
    assert!(!col.path().join(".env").exists() && !col.path().join(".gitignore").exists());
    assert!(present(cx, win, "proxy-password-stored"));
    ed.update(cx, |e, cx| e.save(cx));
    frame(cx, win);
    cx.run_until_parked();
    let env = std::fs::read_to_string(col.path().join(".env")).unwrap_or_default();
    assert_eq!(env, "APINATA_PROXY_PASSWORD='pw'\n");
}

#[gpui_kit::test]
fn an_invalid_port_shows_its_error_and_keeps_the_last_valid_port(cx: &mut TestAppContext) {
    let (win, host) = section(cx, with_proxy(custom("http", "proxy.local", None)), false);
    let before = edit_count(cx, &host);
    type_at_end(cx, win, None, "proxy-port", "x");
    frame(cx, win);
    assert!(present(cx, win, "proxy-port-error"));
    assert_eq!(edit_count(cx, &host), before, "a refused port is not sent");
    type_at_end(cx, win, None, "proxy-host", "y");
    frame(cx, win);
    assert_eq!(
        last_proxy(cx, &host),
        Some(custom("http", "proxy.localy", None))
    );
    cx.update_window(win, |_, w, cx| w.press("backspace", cx))
        .unwrap();
    frame(cx, win);
    replace_input(cx, win, None, "proxy-port", "8080");
    frame(cx, win);
    assert!(!present(cx, win, "proxy-port-error"));
    let mut fixed = ProxyConfig::new("http", "proxy.local", "8080");
    fixed.auth = None;
    assert_eq!(last_proxy(cx, &host), Some(CollectionProxy::Custom(fixed)));
}

#[gpui_kit::test]
fn a_template_password_shows_and_a_literal_stays_masked(cx: &mut TestAppContext) {
    use apinata_app::network_section::NetworkField;
    for (password, masked) in [("{{proxy_pw}}", false), ("pw", true)] {
        let (_win, host) = section(
            cx,
            with_proxy(custom("http", "proxy.local", Some(("u", password)))),
            false,
        );
        let s = host.read_with(cx, |h, _| h.view.clone());
        let input = s.read_with(cx, |s, _| s.input(NetworkField::Password).clone());
        assert_eq!(
            input.read_with(cx, |i, _| i.presentation().is_masked()),
            masked,
            "{password}"
        );
    }
}

#[gpui_kit::test]
fn saving_with_a_refused_port_keeps_the_last_valid_one_and_says_so(cx: &mut TestAppContext) {
    refused_port_on_save(cx, |cx, win| {
        cx.dispatch_action(win, apinata_app::keymap::SaveActive)
    });
}

#[gpui_kit::test]
fn the_save_key_in_the_port_field_names_a_refused_port(cx: &mut TestAppContext) {
    // With focus in the settings tab the key resolves to the editor's own save action.
    refused_port_on_save(cx, |cx, win| {
        cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
            .unwrap()
    });
}

fn refused_port_on_save(
    cx: &mut TestAppContext,
    save: impl FnOnce(&mut TestAppContext, AnyWindowHandle),
) {
    use apinata_app::tabs::TabKind;
    use gpui_kit::component::WindowExt as _;
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let tab =
        s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Collection, cx)
        })
        .unwrap();
    frame(cx, s.win);
    let ed = s
        .shell
        .read_with(cx, |v, cx| {
            v.tab_area().read(cx).config_editor(tab).cloned()
        })
        .unwrap();
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, s.win);
    segment(cx, s.win, "proxy-mode", 2);
    type_at_end(cx, s.win, None, "proxy-host", "proxy.local");
    type_at_end(cx, s.win, None, "proxy-port", "abc");
    frame(cx, s.win);
    let toasts = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
            .unwrap()
    };
    let before = toasts(cx);
    save(cx, s.win);
    frame(cx, s.win);
    assert_eq!(toasts(cx), before + 1, "a refused port is named on save");
    let unsaved = ed.read_with(cx, |e, cx| e.unsaved(cx));
    assert_eq!(unsaved.len(), 1, "{unsaved:?}");
    assert!(unsaved[0].starts_with("Not saved: Port: "), "{unsaved:?}");
    assert!(present(cx, s.win, "proxy-port-error"));
    let network = ed.read_with(cx, |e, _| e.network().cloned()).unwrap();
    let port = network.read_with(cx, |n, cx| {
        n.input(apinata_app::network_section::NetworkField::Port)
            .read(cx)
            .value()
            .to_string()
    });
    assert_eq!(port, "abc", "the refused text stays as typed");
    let saved = std::fs::read_to_string(col.path().join("opencollection.yml")).unwrap();
    assert!(
        saved.contains("proxy.local") && !saved.contains("abc"),
        "{saved}"
    );
}

const OFF_WITH_STORED: &str = "config:\n  proxy:\n    inherit: false\n    disabled: true\n    config:\n      protocol: http\n      hostname: proxy.local\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"{{process.env.APINATA_PROXY_PASSWORD}}\"\n      bypassProxy: \"\"\n";

#[gpui_kit::test]
fn switching_to_custom_restores_the_stored_proxy_and_saving_keeps_its_auth(
    cx: &mut TestAppContext,
) {
    let col = write_tree(&[("opencollection.yml", &format!("{ROOT}{OFF_WITH_STORED}"))]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    segment(cx, win, "proxy-mode", 2);
    let network = ed.read_with(cx, |e, _| e.network().cloned()).unwrap();
    let text = |cx: &mut TestAppContext, f| {
        network.read_with(cx, |n, cx| n.input(f).read(cx).value().to_string())
    };
    use apinata_app::network_section::NetworkField as F;
    assert_eq!(
        (text(cx, F::Host), text(cx, F::Port), text(cx, F::Username)),
        ("proxy.local".into(), "3128".into(), "u".into())
    );
    ed.update(cx, |e, cx| e.save(cx));
    frame(cx, win);
    let saved = std::fs::read_to_string(col.path().join("opencollection.yml")).unwrap();
    assert!(
        saved.contains("{{process.env.APINATA_PROXY_PASSWORD}}") && saved.contains("username: u"),
        "{saved}"
    );
}

#[gpui_kit::test]
fn choosing_off_from_the_port_keeps_focus_so_the_save_key_saves(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", &root_with_password("{{x}}"))]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("proxy-port", cx))
        .unwrap();
    frame(cx, win);
    segment(cx, win, "proxy-mode", 1);
    let focused = cx
        .update_window(win, |_, w, cx| {
            ed.read(cx).focus_handle(cx).contains_focused(w, cx)
        })
        .unwrap();
    assert!(focused, "focus stays in the editor");
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    frame(cx, win);
    let saved = std::fs::read_to_string(col.path().join("opencollection.yml")).unwrap();
    assert!(saved.contains("disabled: true"), "{saved}");
}

#[gpui_kit::test]
fn a_typed_password_survives_choosing_off(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", &root_with_password("{{x}}"))]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, "proxy-password", "z");
    frame(cx, win);
    segment(cx, win, "proxy-mode", 1);
    ed.update(cx, |e, cx| e.save(cx));
    frame(cx, win);
    let saved = std::fs::read_to_string(col.path().join("opencollection.yml")).unwrap();
    assert!(
        saved.contains("{{x}}z") && saved.contains("disabled: true"),
        "{saved}"
    );
}

#[gpui_kit::test]
fn the_network_form_stacks_at_the_minimum_window(cx: &mut TestAppContext) {
    use apinata_app::session::{MIN_HEIGHT, MIN_WIDTH};
    let (win, _host) = section(cx, with_proxy(custom("http", "proxy.local", None)), false);
    cx.simulate_window_resize(
        win,
        gpui_kit::size(gpui_kit::px(MIN_WIDTH), gpui_kit::px(MIN_HEIGHT.max(600.))),
    );
    frame(cx, win);
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        for r in [
            "proxy-mode-row",
            "proxy-protocol-row",
            "proxy-host-row",
            "proxy-port-row",
            "proxy-username-row",
            "proxy-password-row",
            "proxy-bypass-row",
        ] {
            let label = w.within(r).find("form-label").bounds();
            let control = w.within(r).find("form-control").bounds();
            assert!(
                label.bottom() <= control.top(),
                "{r}: {label:?} over {control:?}"
            );
            assert_eq!(label.left(), control.left(), "{r}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_bypass_hint_sits_under_its_field(cx: &mut TestAppContext) {
    let (win, _host) = section(cx, with_proxy(custom("http", "proxy.local", None)), false);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let field = w.find("proxy-bypass").bounds();
        let hint = w
            .within("proxy-bypass-row")
            .find("proxy-bypass-note")
            .bounds();
        assert_eq!(hint.left(), field.left());
        assert!(hint.top() >= field.bottom(), "{hint:?} under {field:?}");
    })
    .unwrap();
}

// A root file in the writer's layout with a custom proxy whose port line is `port: PORT`.
const PORTED: &str = "opencollection: 1.0.0\n\ninfo:\n  name: c\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: h\n      port: PORT\n      auth:\n        username: \"\"\n        password: \"\"\n      bypassProxy: \"\"\n";

// The collection and the app state directories, kept alive for the test.
type Dirs = (tempfile::TempDir, tempfile::TempDir);

// The network tab of the collection settings over a root file holding `text`.
fn network_editor(
    cx: &mut TestAppContext,
    text: &str,
) -> (Dirs, AnyWindowHandle, Entity<ConfigEditor>) {
    let col = write_tree(&[("opencollection.yml", text)]);
    let (state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    ((col, state), win, ed)
}

fn save_and_read(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ed: &Entity<ConfigEditor>,
    (col, _): &Dirs,
) -> String {
    cx.run_until_parked();
    ed.update(cx, |e, cx| e.save(cx));
    frame(cx, win);
    std::fs::read_to_string(col.path().join("opencollection.yml")).unwrap()
}

#[gpui_kit::test]
fn a_numeric_port_stays_a_number_and_a_typed_port_is_written_as_one(cx: &mut TestAppContext) {
    let original = PORTED.replace("PORT", "80");
    let (col, win, ed) = network_editor(cx, &original);
    type_at_end(cx, win, None, "proxy-host", "x");
    let hx = original.replace("hostname: h\n", "hostname: hx\n");
    assert_eq!(save_and_read(cx, win, &ed, &col), hx);
    type_at_end(cx, win, None, "proxy-port", "8");
    assert_eq!(
        save_and_read(cx, win, &ed, &col),
        hx.replace("port: 80\n", "port: 808\n")
    );
}

#[gpui_kit::test]
fn a_string_port_stays_a_string(cx: &mut TestAppContext) {
    let original = PORTED.replace("PORT", "\"80\"");
    let (col, win, ed) = network_editor(cx, &original);
    type_at_end(cx, win, None, "proxy-host", "x");
    assert_eq!(
        save_and_read(cx, win, &ed, &col),
        original.replace("hostname: h\n", "hostname: hx\n")
    );
}

#[gpui_kit::test]
fn an_unfilled_certificate_row_is_not_saved_until_it_is_complete(cx: &mut TestAppContext) {
    let original = PORTED.replace("PORT", "\"80\"");
    let (col, win, ed) = network_editor(cx, &original);
    type_at_end(cx, win, None, "proxy-host", "x");
    cx.update_window(win, |_, w, cx| w.click("cert-add", cx))
        .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, ("cert-domain", 0usize), "api.test");
    let hx = original.replace("hostname: h\n", "hostname: hx\n");
    assert_eq!(save_and_read(cx, win, &ed, &col), hx);
    assert!(
        present(cx, win, ("cert-domain", 0usize)),
        "the draft row stays"
    );
    type_at_end(cx, win, None, ("cert-file", 0usize), "c.pem");
    type_at_end(cx, win, None, ("cert-key", 0usize), "k.pem");
    let certs = "  clientCertificates:\n    - domain: api.test\n      type: pem\n      certificateFilePath: c.pem\n      privateKeyFilePath: k.pem\n";
    assert_eq!(save_and_read(cx, win, &ed, &col), format!("{hx}{certs}"));
}

#[gpui_kit::test]
fn a_dotenv_password_shows_a_badge_and_replace_takes_a_new_value(cx: &mut TestAppContext) {
    let stored = "{{process.env.APINATA_PROXY_PASSWORD}}";
    let (win, host) = section(
        cx,
        with_proxy(custom("http", "proxy.local", Some(("u", stored)))),
        false,
    );
    assert!(
        !present(cx, win, "proxy-password-stored"),
        "no such .env key"
    );
    stored_names(cx, win, &host, &["APINATA_PROXY_PASSWORD"]);
    assert!(present(cx, win, "proxy-password-stored"));
    assert!(!present(cx, win, "proxy-password"));
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    let before = edit_count(cx, &host);
    // A replace left empty keeps the stored password.
    cx.update_window(win, |_, w, cx| w.click("proxy-password-replace", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("proxy-host", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(edit_count(cx, &host), before);
    assert!(present(cx, win, "proxy-password-stored"));
    cx.update_window(win, |_, w, cx| w.click("proxy-password-replace", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.input("new-pw", cx);
        w.press("enter", cx);
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(
        last_proxy(cx, &host),
        Some(custom("http", "proxy.local", Some(("u", "new-pw"))))
    );
}

#[gpui_kit::test]
fn a_dotenv_passphrase_shows_a_badge_and_replace_takes_a_new_value(cx: &mut TestAppContext) {
    let mut cert = apinata_core::ClientCertificate::pem("a.test", "c.pem", "k.pem");
    cert.passphrase = "{{process.env.APINATA_CERT_PASSPHRASE_1}}".into();
    let mut network = CollectionNetwork::default();
    network.certificates = vec![cert];
    let (win, host) = section(cx, network, false);
    stored_names(cx, win, &host, &["APINATA_CERT_PASSPHRASE_1"]);
    assert!(present(cx, win, ("cert-passphrase-stored", 0usize)));
    assert!(!present(cx, win, ("cert-passphrase", 0usize)));
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.click(("cert-passphrase-replace", 0usize), cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.input("new-pp", cx);
        w.press("enter", cx);
    })
    .unwrap();
    frame(cx, win);
    let sent = host.read_with(cx, |h, _| {
        h.edits.iter().rev().find_map(|e| match e {
            Edit::ClientCertificates(c) => Some(c.clone()),
            _ => None,
        })
    });
    assert_eq!(sent.expect("certificates sent")[0].passphrase, "new-pp");
}
