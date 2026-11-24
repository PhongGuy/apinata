//! The collection's Network tab: the proxy choice and fields, and moving literal secrets.

use crate::support;

use apinata_app::command::{Edit, NetworkSlot};
use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::network_section::{NetworkField, NetworkSection, ProxyChoice};
use apinata_app::workspace::Workspace;
use apinata_core::{CollectionNetwork, CollectionProxy, ProxyAuth, ProxyConfig};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use std::path::{Path, PathBuf};
use support::*;

type Host = Entity<SectionHost<NetworkSection>>;

fn section(cx: &mut TestAppContext, network: CollectionNetwork) -> (AnyWindowHandle, Host) {
    let (win, host) = host_section(cx, |w, cx| NetworkSection::new(PathBuf::from("/c"), w, cx));
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, w, cx))
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

// The accessible label of element `id`, after a fresh frame.
fn label_of(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> Option<String> {
    let id = id.into();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
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
    let (win, host) = section(cx, with_proxy(custom("http", "proxy.local", None)));
    let s = host.read_with(cx, |h, _| h.view.clone());
    assert_eq!(s.read_with(cx, |s, _| s.choice()), ProxyChoice::Custom);
    assert!(present(cx, win, "proxy-host"));
    assert_eq!(edit_count(cx, &host), 0);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&CollectionNetwork::default(), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(!present(cx, win, "proxy-host"));
    assert_eq!(edit_count(cx, &host), 0);
}

#[gpui_kit::test]
fn the_choice_sends_the_proxy(cx: &mut TestAppContext) {
    let (win, host) = section(cx, with_proxy(custom("http", "proxy.local", None)));
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
    let (win, host) = section(cx, with_proxy(custom("http", "proxy.local", None)));
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
fn a_literal_password_offers_its_own_move(cx: &mut TestAppContext) {
    let (win, host) = section(
        cx,
        with_proxy(custom("http", "proxy.local", Some(("u", "pw")))),
    );
    assert!(!present(cx, win, "network-plain-notice"));
    cx.update_window(win, |_, w, cx| w.click("proxy-password-move", cx))
        .unwrap();
    cx.run_until_parked();
    let moved = host.read_with(cx, |h, _| {
        matches!(
            h.edits.last(),
            Some(Edit::MoveNetworkSecret(NetworkSlot::ProxyPassword))
        )
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
    assert_eq!(
        label_of(cx, win, "proxy-password-storage").as_deref(),
        Some("Saved to .env when you save")
    );
    ed.update(cx, |e, cx| e.save(cx));
    frame(cx, win);
    cx.run_until_parked();
    let env = std::fs::read_to_string(col.path().join(".env")).unwrap_or_default();
    assert_eq!(env, "APINATA_PROXY_PASSWORD='pw'\n");
}

#[gpui_kit::test]
fn an_invalid_port_shows_its_error_and_keeps_the_last_valid_port(cx: &mut TestAppContext) {
    let (win, host) = section(cx, with_proxy(custom("http", "proxy.local", None)));
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
    let (win, _host) = section(cx, with_proxy(custom("http", "proxy.local", None)));
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
    let (win, _host) = section(cx, with_proxy(custom("http", "proxy.local", None)));
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
fn a_dotenv_password_shows_its_reference_and_takes_a_new_value(cx: &mut TestAppContext) {
    let stored = "{{process.env.APINATA_PROXY_PASSWORD}}";
    let (win, host) = section(
        cx,
        with_proxy(custom("http", "proxy.local", Some(("u", stored)))),
    );
    stored_names(cx, win, &host, &["APINATA_PROXY_PASSWORD"]);
    assert_eq!(
        label_of(cx, win, "proxy-password-storage").as_deref(),
        Some("Stored in .env")
    );
    assert!(!present(cx, win, "proxy-password-move"));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let shown = s.read_with(cx, |s, cx| {
        s.input(NetworkField::Password).read(cx).value().to_string()
    });
    assert_eq!(shown, stored);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    replace_input(cx, win, None, "proxy-password", "new-pw");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(
        last_proxy(cx, &host),
        Some(custom("http", "proxy.local", Some(("u", "new-pw"))))
    );
}

#[gpui_kit::test]
fn a_custom_proxy_without_a_host_is_held_back_and_named(cx: &mut TestAppContext) {
    let (win, host) = section(cx, CollectionNetwork::default());
    segment(cx, win, "proxy-mode", 2);
    assert_eq!(last_proxy(cx, &host), None, "no host, nothing sent");
    assert!(present(cx, win, "proxy-host-incomplete"));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let unsaved = s.read_with(cx, |s, cx| s.unsaved(cx));
    assert!(
        unsaved.iter().any(|w| w.starts_with("Not saved: Proxy")),
        "{unsaved:?}"
    );
    type_at_end(cx, win, None, "proxy-host", "p.test");
    cx.run_until_parked();
    frame(cx, win);
    let mut sent = ProxyConfig::new("http", "p.test", "");
    sent.auth = None;
    assert_eq!(last_proxy(cx, &host), Some(CollectionProxy::Custom(sent)));
    assert!(!present(cx, win, "proxy-host-incomplete"));
    assert!(s.read_with(cx, |s, cx| s.unsaved(cx)).is_empty());
}

#[gpui_kit::test]
fn a_held_back_custom_proxy_survives_a_reload(cx: &mut TestAppContext) {
    let (win, host) = section(cx, CollectionNetwork::default());
    segment(cx, win, "proxy-mode", 2);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&CollectionNetwork::default(), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(s.read_with(cx, |s, _| s.choice()), ProxyChoice::Custom);
    assert!(present(cx, win, "proxy-host"));
}

#[gpui_kit::test]
fn an_empty_custom_proxy_and_an_empty_host_override_never_reach_the_file(cx: &mut TestAppContext) {
    let (col, win, ed) = network_editor(cx, ROOT);
    segment(cx, win, "proxy-mode", 2);
    cx.update_window(win, |_, w, cx| w.click("host-add", cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(save_and_read(cx, win, &ed, &col), ROOT);
    segment(cx, win, "proxy-mode", 0);
    assert_eq!(
        save_and_read(cx, win, &ed, &col),
        ROOT,
        "no empty proxy block"
    );
}

#[gpui_kit::test]
fn a_dotenv_passphrase_shows_its_reference_and_takes_a_new_value(cx: &mut TestAppContext) {
    let stored = "{{process.env.APINATA_CERT_PASSPHRASE_1}}";
    let mut cert = apinata_core::ClientCertificate::pem("a.test", "c.pem", "k.pem");
    cert.passphrase = stored.into();
    let mut network = CollectionNetwork::default();
    network.certificates = vec![cert];
    let (win, host) = section(cx, network);
    stored_names(cx, win, &host, &["APINATA_CERT_PASSPHRASE_1"]);
    assert_eq!(
        label_of(cx, win, ("cert-passphrase-storage", 0usize)).as_deref(),
        Some("Stored in .env")
    );
    assert!(!present(cx, win, ("cert-passphrase-move", 0usize)));
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    replace_input(cx, win, None, ("cert-passphrase", 0usize), "new-pp");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
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

#[gpui_kit::test]
fn a_saved_custom_proxy_without_a_host_says_requests_fail(cx: &mut TestAppContext) {
    let (win, host) = section(cx, with_proxy(custom("http", "", None)));
    assert!(present(cx, win, "proxy-host-fails"));
    assert!(!present(cx, win, "proxy-host-incomplete"));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let unsaved = s.read_with(cx, |s, cx| s.unsaved(cx));
    assert!(
        !unsaved.iter().any(|w| w.starts_with("Not saved: Proxy")),
        "{unsaved:?}"
    );
}

#[gpui_kit::test]
fn an_echo_keeps_a_cleared_proxy_host_cleared(cx: &mut TestAppContext) {
    let network = with_proxy(custom("http", "proxy.local", None));
    let (win, host) = section(cx, network.clone());
    let before = edit_count(cx, &host);
    replace_input(cx, win, None, "proxy-host", "");
    cx.run_until_parked();
    assert_eq!(edit_count(cx, &host), before, "a blank host is not sent");
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&network, w, cx))
    })
    .unwrap();
    frame(cx, win);
    use apinata_app::network_section::NetworkField;
    let typed = s.read_with(cx, |s, cx| {
        s.input(NetworkField::Host).read(cx).value().to_string()
    });
    assert_eq!(typed, "");
    assert!(present(cx, win, "proxy-host-incomplete"));
}

// One collection with a custom proxy, a host override and a certificate row.
fn every_part(cx: &mut TestAppContext) -> AnyWindowHandle {
    let mut network = with_proxy(custom("http", "proxy.local", None));
    network.host_overrides = vec![apinata_core::HostOverride::new("a.test", "127.0.0.1")];
    let (win, _host) = section(cx, network);
    cx.update_window(win, |_, w, cx| w.click("cert-add", cx))
        .unwrap();
    frame(cx, win);
    win
}

#[gpui_kit::test]
fn proxy_fields_and_the_grids_share_one_input_style(cx: &mut TestAppContext) {
    let win = every_part(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let proxy = w.find("proxy-host").bounds().size.height;
        let host = w.find(("host-name", 0usize)).bounds().size.height;
        let cert = w.find(("cert-domain", 0usize)).bounds().size.height;
        assert_eq!(proxy, host, "proxy {proxy:?} host {host:?}");
        assert_eq!(proxy, cert, "proxy {proxy:?} cert {cert:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_grid_notes_line_up_with_their_rows_and_buttons(cx: &mut TestAppContext) {
    let win = every_part(cx);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        for (note, add, row) in [
            ("hosts-note", "host-add", ("host-row", 0usize)),
            ("cert-note", "cert-add", ("cert-row", 0usize)),
        ] {
            let note = w.find(note).bounds().left();
            assert_eq!(note, w.find(add).bounds().left(), "{add}");
            assert_eq!(note, w.find(row).bounds().left(), "{row:?}");
        }
        let text = w.find("cert-note").label().map(str::to_owned);
        assert!(text.is_some_and(|t| t.contains("domain")));
    })
    .unwrap();
}

const PKCS12: &str = "config:\n  clientCertificates:\n    - domain: p.test\n      type: pkcs12\n      pkcs12FilePath: c.p12\n";

// Types `text` at the end of field `id` and saves with the key while the field keeps the
// focus, so no Enter or blur commits the typing first; returns the `.env` written.
fn save_key_while_typing(
    cx: &mut TestAppContext,
    root_file: &str,
    id: impl Into<gpui_kit::ElementId>,
    text: &str,
) -> String {
    let col = write_tree(&[("opencollection.yml", root_file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor(cx, &ws, &root, ConfigTarget::Collection);
    ed.update(cx, |e, cx| e.set_section(ConfigSection::Network, cx));
    frame(cx, win);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, id, text);
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.press(&key("s"), cx))
        .unwrap();
    frame(cx, win);
    cx.run_until_parked();
    std::fs::read_to_string(col.path().join(".env")).unwrap_or_default()
}

#[gpui_kit::test]
fn the_save_key_keeps_a_passphrase_still_being_typed(cx: &mut TestAppContext) {
    let root_file = format!("{ROOT}{PKCS12}");
    let env = save_key_while_typing(cx, &root_file, ("cert-passphrase", 0usize), "p12-pass");
    assert!(env.contains("='p12-pass'"), "{env:?}");
}

#[gpui_kit::test]
fn the_save_key_keeps_a_proxy_password_still_being_typed(cx: &mut TestAppContext) {
    let env = save_key_while_typing(cx, &root_with_password(""), "proxy-password", "pw");
    assert_eq!(env, "APINATA_PROXY_PASSWORD='pw'\n");
}

// A PEM row over a PKCS#12 row whose bundle sits deep in folders, its passphrase held in
// `.env` until the save, drawn in a window `width` wide.
fn pem_over_bundle(cx: &mut TestAppContext, width: f32) -> (AnyWindowHandle, Host) {
    let pem = apinata_core::ClientCertificate::pem("a.test", "c.pem", "k.pem");
    let mut bundle = apinata_core::ClientCertificate::pkcs12("p.test", BUNDLE);
    bundle.passphrase = "{{process.env.APINATA_CERT_PASSPHRASE_2}}".into();
    let mut network = CollectionNetwork::default();
    network.certificates = vec![pem, bundle];
    let (win, host) = section(cx, network);
    gpui_kit::VisualTestContext::from_window(win, cx)
        .simulate_resize(gpui_kit::size(gpui_kit::px(width), gpui_kit::px(700.)));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let names = apinata_app::snapshot::DotenvNames {
        stored: Default::default(),
        held: ["APINATA_CERT_PASSPHRASE_2".to_string()].into(),
    };
    cx.update_window(win, |_, _, cx| {
        s.update(cx, |s, cx| s.set_dotenv(names, cx))
    })
    .unwrap();
    for _ in 0..3 {
        frame(cx, win);
    }
    (win, host)
}

const BUNDLE: &str = "certs/staging/eu-west/clients/team-payments/client-bundle.p12";

#[gpui_kit::test]
fn a_held_passphrase_hint_sits_under_its_row(cx: &mut TestAppContext) {
    let (win, _host) = pem_over_bundle(cx, 1040.);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let row = w.find(("cert-row", 1usize)).bounds();
        let note = w.find(("cert-passphrase-storage", 1usize)).bounds();
        assert!(note.top() >= row.bottom(), "{note:?} over {row:?}");
        assert!(note.right() <= row.right(), "{note:?} past {row:?}");
    })
    .unwrap();
    assert_eq!(
        label_of(cx, win, ("cert-passphrase-storage", 1usize)).as_deref(),
        Some("Saved to .env when you save")
    );
}

#[gpui_kit::test]
fn a_bundle_path_takes_the_unused_key_room_and_keeps_its_file_name(cx: &mut TestAppContext) {
    let (win, _host) = pem_over_bundle(cx, 1040.);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let pem_key_pick = w.find(("cert-key-pick", 0usize)).bounds();
        let bundle = w.find(("cert-file", 1usize)).bounds();
        let bundle_pick = w.find(("cert-file-pick", 1usize)).bounds();
        assert_eq!(
            bundle_pick.right(),
            pem_key_pick.right(),
            "the bundle's picker ends where the PEM key's does"
        );
        assert!(bundle.right() < bundle_pick.left());
        let pem_pass = w.find(("cert-passphrase", 0usize)).bounds().left();
        let bundle_pass = w.find(("cert-passphrase", 1usize)).bounds().left();
        assert_eq!(pem_pass, bundle_pass, "the passphrase columns line up");
        let shown = shown_path(w).expect("a cut path shows its fitted form");
        assert!(shown.ends_with("/client-bundle.p12"), "{shown:?}");
        assert!(shown.contains('…'), "{shown:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn add_certificate_focuses_the_new_domain(cx: &mut TestAppContext) {
    let (win, host) = pem_over_bundle(cx, 1040.);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click("cert-add", cx))
        .unwrap();
    frame(cx, win);
    let s = host.read_with(cx, |h, _| h.view.clone());
    use apinata_app::network_section::CertField;
    let domain = s.read_with(cx, |s, _| s.cert_input(2, CertField::Domain).cloned());
    let domain = domain.expect("a third row");
    cx.update_window(win, |_, w, cx| {
        assert!(
            domain.focus_handle(cx).is_focused(w),
            "focus is on the new domain"
        )
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_type_menu_opens_with_every_entry_shown(cx: &mut TestAppContext) {
    let (win, _host) = pem_over_bundle(cx, 1040.);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click("cert-type-1", cx))
        .unwrap();
    for _ in 0..3 {
        frame(cx, win);
    }
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let menu = w.find("popup-menu").bounds();
        let first = w.within("popup-menu").find(0usize).bounds();
        let last = w.within("popup-menu").find(1usize).bounds();
        assert!(first.top() >= menu.top(), "PEM {first:?} cut by {menu:?}");
        assert!(last.bottom() <= menu.bottom(), "{last:?} cut by {menu:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_click_on_the_fitted_path_edits_the_whole_path(cx: &mut TestAppContext) {
    let (win, host) = pem_over_bundle(cx, 1040.);
    assert!(present(cx, win, ("cert-file-shown", 1usize)));
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    cx.update_window(win, |_, w, cx| w.click(("cert-file", 1usize), cx))
        .unwrap();
    frame(cx, win);
    let s = host.read_with(cx, |h, _| h.view.clone());
    use apinata_app::network_section::CertField;
    let file = s.read_with(cx, |s, _| s.cert_input(1, CertField::Certificate).cloned());
    cx.update_window(win, |_, w, cx| {
        assert!(
            file.unwrap().focus_handle(cx).is_focused(w),
            "the click focuses the path"
        )
    })
    .unwrap();
    assert!(
        !present(cx, win, ("cert-file-shown", 1usize)),
        "the whole path shows"
    );
}

// The text the fitted-path cover over the bundle row painted, if it is drawn.
fn shown_path(w: &gpui_kit::Window) -> Option<String> {
    let id = gpui_kit::ElementId::from(("cert-file-shown", 1usize));
    let texts = gpui_kit::base::test_support::texts(w);
    let mut parts = texts.iter().filter(|t| t.path().contains(&id));
    parts.next().map(|t| t.text().to_owned())
}

#[gpui_kit::test]
fn the_fitted_path_is_not_a_second_accessible_name(cx: &mut TestAppContext) {
    let (win, _host) = pem_over_bundle(cx, 1040.);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let cover = w.find(("cert-file-shown", 1usize));
        assert_eq!(cover.label(), None, "the input already names the path");
        assert_eq!(cover.role(), None);
    })
    .unwrap();
}

// The cover hides the live input's text, so in every bundled theme it paints the field's
// look as one opaque fill, rounded just inside the input's border.
#[gpui_kit::test]
fn the_fitted_path_cover_is_opaque_in_every_theme(cx: &mut TestAppContext) {
    use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
    use gpui_kit::component::ActiveTheme as _;
    let (win, _host) = pem_over_bundle(cx, 1040.);
    for theme in [DARK, LIGHT, HC_LIGHT, HC_DARK] {
        support::use_theme(cx, theme);
        for _ in 0..3 {
            frame(cx, win);
        }
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            let scale = w.scale_factor();
            let b = w.find(("cert-file-shown", 1usize)).bounds();
            let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
            let width = b.size.width.as_f32() * scale;
            let fills: Vec<_> = (w.painted_quads().into_iter())
                .filter(|q| {
                    (q.bounds.origin.x.0 - l).abs() < 0.5
                        && (q.bounds.origin.y.0 - t).abs() < 0.5
                        && (q.bounds.size.width.0 - width).abs() < 0.5
                })
                .filter_map(|q| Some((q.background.as_solid()?, q.corner_radii.top_left.0)))
                .collect();
            let want = (cx.theme().background).blend(apinata_app::ui::field_fill(false, cx));
            let radius = (cx.theme().radius - gpui_kit::px(1.)).as_f32() * scale;
            assert!(
                fills
                    .iter()
                    .any(|(c, r)| *c == want && c.a >= 0.999 && (r - radius).abs() < 0.01),
                "{theme}: cover painted {fills:?}, wanted {want:?} at radius {radius}"
            );
        })
        .unwrap();
    }
}

// The cert row `row`'s passphrase input.
fn passphrase_input(
    cx: &mut TestAppContext,
    host: &Host,
    row: usize,
) -> Entity<gpui_kit::component::input::InputState> {
    use apinata_app::network_section::CertField;
    let s = host.read_with(cx, |h, _| h.view.clone());
    s.read_with(cx, |s, _| s.cert_input(row, CertField::Passphrase).cloned())
        .expect("a row")
}

#[gpui_kit::test]
fn a_move_on_a_row_after_an_open_draft_names_its_file_row(cx: &mut TestAppContext) {
    let mut network = CollectionNetwork::default();
    network.certificates = vec![apinata_core::ClientCertificate::pem(
        "a.test", "c.pem", "k.pem",
    )];
    let (win, host) = section(cx, network);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    for _ in 0..2 {
        cx.update_window(win, |_, w, cx| w.click("cert-add", cx))
            .unwrap();
        frame(cx, win);
    }
    // Row 2 completes while row 1 stays an open draft, so row 2 is the file's second row.
    type_at_end(cx, win, None, ("cert-domain", 2usize), "c.test");
    type_at_end(cx, win, None, ("cert-file", 2usize), "c2.pem");
    type_at_end(cx, win, None, ("cert-key", 2usize), "k2.pem");
    type_at_end(cx, win, None, ("cert-passphrase", 2usize), "pp");
    cx.update_window(win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, win);
    assert!(
        present(cx, win, ("cert-domain", 1usize)),
        "row 1 is a draft"
    );
    cx.update_window(win, |_, w, cx| {
        w.click(("cert-passphrase-move", 2usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let last = host.read_with(cx, |h, _| h.edits.last().cloned());
    assert!(
        matches!(
            last,
            Some(Edit::MoveNetworkSecret(NetworkSlot::Passphrase(1)))
        ),
        "{last:?}"
    );
}

#[gpui_kit::test]
fn a_move_sends_the_other_fields_typing_first(cx: &mut TestAppContext) {
    let mut cert = apinata_core::ClientCertificate::pem("a.test", "c.pem", "k.pem");
    cert.passphrase = "{{old}}".into();
    let mut network = with_proxy(custom("http", "proxy.local", Some(("u", "pw"))));
    network.certificates = vec![cert];
    let (win, host) = section(cx, network);
    let pass = passphrase_input(cx, &host, 0);
    cx.update_window(win, |_, w, cx| {
        pass.update(cx, |i, cx| i.set_value("{{new}}".to_string(), w, cx))
    })
    .unwrap();
    frame(cx, win);
    let before = edit_count(cx, &host);
    cx.update_window(win, |_, w, cx| w.click("proxy-password-move", cx))
        .unwrap();
    cx.run_until_parked();
    let sent = host.read_with(cx, |h, _| h.edits[before..].to_vec());
    match sent.as_slice() {
        [
            Edit::ClientCertificates(certs),
            Edit::MoveNetworkSecret(NetworkSlot::ProxyPassword),
        ] => assert_eq!(certs[0].passphrase, "{{new}}"),
        other => panic!("{other:?}"),
    }
}

#[gpui_kit::test]
fn a_reveal_ends_when_a_reload_changes_the_value(cx: &mut TestAppContext) {
    let (win, host) = section(
        cx,
        with_proxy(custom("http", "proxy.local", Some(("u", "pw1")))),
    );
    let s = host.read_with(cx, |h, _| h.view.clone());
    let password = s.read_with(cx, |s, _| s.input(NetworkField::Password).clone());
    let masked =
        |cx: &mut TestAppContext| password.read_with(cx, |i, _| i.presentation().is_masked());
    assert!(masked(cx));
    // The field's eye reveals the value, and the reveal holds while the value stays.
    cx.update_window(win, |_, w, cx| {
        password.update(cx, |i, cx| i.set_masked(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(!masked(cx));
    cx.update_window(win, |_, w, cx| {
        let network = with_proxy(custom("http", "proxy.local", Some(("u", "pw2"))));
        s.update(cx, |s, cx| s.load(&network, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(masked(cx), "a reloaded value is masked again");
    // A move's reference shows, and a value restored over it is masked again.
    let reference = "{{process.env.APINATA_PROXY_PASSWORD}}";
    cx.update_window(win, |_, w, cx| {
        let network = with_proxy(custom("http", "proxy.local", Some(("u", reference))));
        s.update(cx, |s, cx| s.load(&network, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(!masked(cx), "a reference shows");
    cx.update_window(win, |_, w, cx| {
        let network = with_proxy(custom("http", "proxy.local", Some(("u", "pw2"))));
        s.update(cx, |s, cx| s.load(&network, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(
        masked(cx),
        "a literal restored over the reference is masked"
    );
}

#[gpui_kit::test]
fn a_revealed_passphrase_is_masked_again_after_a_reload(cx: &mut TestAppContext) {
    let cert = |pass: &str| {
        let mut cert = apinata_core::ClientCertificate::pem("a.test", "c.pem", "k.pem");
        cert.passphrase = pass.into();
        let mut network = CollectionNetwork::default();
        network.certificates = vec![cert];
        network
    };
    let (win, host) = section(cx, cert("pp1"));
    let s = host.read_with(cx, |h, _| h.view.clone());
    let pass = passphrase_input(cx, &host, 0);
    cx.update_window(win, |_, w, cx| {
        pass.update(cx, |i, cx| i.set_masked(false, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(!pass.read_with(cx, |i, _| i.presentation().is_masked()));
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&cert("pp2"), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(pass.read_with(cx, |i, _| i.presentation().is_masked()));
}
