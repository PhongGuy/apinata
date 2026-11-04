//! An outside reload leaves text typed but not yet committed in the settings inputs alone,
//! unless the file changed that very value.

use crate::support;

use apinata_app::config_editor::{ConfigEditor, ConfigSection, ConfigTarget};
use apinata_app::network_section::{CertField, HostField, NetworkField};
use gpui_kit::component::input::InputState;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, Focusable, TestAppContext};
use support::*;

const PROXY: &str = "config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: h.local\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"{{old}}\"\n      bypassProxy: \"\"\n";
const CERT: &str = "  clientCertificates:\n    - domain: a.test\n      type: pem\n      certificateFilePath: a.pem\n      privateKeyFilePath: a.key\n      passphrase: \"{{old}}\"\n";
const HOSTS: &str = "extensions:\n  apinata:\n    hostOverrides:\n      - host: api.test\n        address: 127.0.0.1\n";
const OAUTH2: &str = "request:\n  auth:\n    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: http://id/token\n    scope: read\n    credentials:\n      clientId: svc\n      clientSecret: \"{{old}}\"\n    resourceOwner:\n      username: u\n      password: old\n";
const FOLDER_TAGS: &str = "info:\n  name: Users\n  type: folder\n  seq: 1\n  tags:\n    - one\n";

struct Rig {
    _col: tempfile::TempDir,
    _state: tempfile::TempDir,
    ws: Entity<apinata_app::workspace::Workspace>,
    root: std::path::PathBuf,
    win: AnyWindowHandle,
    ed: Entity<ConfigEditor>,
}

// The settings of `target` opened with the real watcher on a collection whose root file is
// `collection`, beside a request `a.yml` the tests rewrite as the unrelated file.
fn rig(
    cx: &mut TestAppContext,
    collection: &str,
    target: ConfigTarget,
    section: ConfigSection,
) -> Rig {
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let col = write_tree(&[
        ("opencollection.yml", collection),
        ("users/folder.yml", FOLDER_TAGS),
        ("a.yml", &request("a", "http://x/1")),
    ]);
    let w = watched(cx, &col);
    let (ws2, r) = (w.ws.clone(), w.root.clone());
    let (win, ed) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ConfigEditor::new(ws2.clone(), r.clone(), target, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    ed.update(cx, |e, cx| e.set_section(section, cx));
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    Rig {
        _col: col,
        _state: w.state,
        ws: w.ws,
        root: w.root,
        win,
        ed,
    }
}

impl Rig {
    // Focuses `input` at its end and types `text` one keystroke per character.
    fn type_into(&self, cx: &mut TestAppContext, input: &Entity<InputState>, text: &str) {
        cx.update_window(self.win, |_, w, cx| {
            input.read(cx).focus_handle(cx).focus(w, cx);
        })
        .unwrap();
        // Blur is seen by comparing drawn frames, so each focus change gets a frame.
        frame(cx, self.win);
        cx.simulate_keystrokes(self.win, "end");
        cx.run_until_parked();
        for ch in text.chars() {
            cx.simulate_keystrokes(self.win, &ch.to_string());
            cx.run_until_parked();
        }
    }

    fn focus(&self, cx: &mut TestAppContext, input: &Entity<InputState>) {
        cx.update_window(self.win, |_, w, cx| {
            input.read(cx).focus_handle(cx).focus(w, cx);
        })
        .unwrap();
        // Blur is seen by comparing drawn frames, so each focus change gets a frame.
        frame(cx, self.win);
    }

    // Writes `text` to `rel` behind the app's back and waits until the snapshot shows it.
    fn outside_write(
        &self,
        cx: &mut TestAppContext,
        rel: &str,
        text: &str,
        shown: impl Fn(&apinata_app::snapshot::CollectionSnapshot) -> bool,
    ) {
        settle(cx);
        std::fs::write(self.root.join(rel), text).unwrap();
        wait_for(cx, "the reload", |cx| {
            shown(&ws_snap(cx, &self.ws, &self.root))
        });
        frame(cx, self.win);
    }

    // Rewrites the unrelated request's URL on disk and waits for its reload.
    fn reload_unrelated(&self, cx: &mut TestAppContext) {
        let path = ip("a.yml");
        self.outside_write(cx, "a.yml", &request("a", "http://x/2"), |s| {
            s.content
                .requests
                .get(&path)
                .is_some_and(|v| v.url == "http://x/2")
        });
    }

    fn network(
        &self,
        cx: &mut TestAppContext,
    ) -> Entity<apinata_app::network_section::NetworkSection> {
        self.ed.read_with(cx, |e, _| e.network().unwrap().clone())
    }

    fn net_input(&self, cx: &mut TestAppContext, f: NetworkField) -> Entity<InputState> {
        let n = self.network(cx);
        n.read_with(cx, |n, _| n.input(f).clone())
    }

    fn cert_input(&self, cx: &mut TestAppContext, f: CertField) -> Entity<InputState> {
        let n = self.network(cx);
        n.read_with(cx, |n, _| n.cert_input(0, f).unwrap().clone())
    }

    fn host_input(&self, cx: &mut TestAppContext, f: HostField) -> Entity<InputState> {
        let n = self.network(cx);
        n.read_with(cx, |n, _| n.host_input(0, f).unwrap().clone())
    }
}

fn text_of(cx: &mut TestAppContext, input: &Entity<InputState>) -> String {
    input.read_with(cx, |i, _| i.value().to_string())
}

type Read = fn(&apinata_app::snapshot::ConfigView) -> String;

// Types `typed` at the end of `input`, reloads an unrelated file, and checks the field still
// shows the typing; moving focus to `blur_to` then commits that text, as `read` finds it.
fn kept_through_an_unrelated_reload(
    r: &Rig,
    cx: &mut TestAppContext,
    input: Entity<InputState>,
    blur_to: Entity<InputState>,
    typed: &str,
    read: Read,
) {
    r.type_into(cx, &input, typed);
    let shown = text_of(cx, &input);
    r.reload_unrelated(cx);
    assert_eq!(text_of(cx, &input), shown, "the reload replaced the typing");
    r.focus(cx, &blur_to);
    let view = ws_snap(cx, &r.ws, &r.root).content.collection.clone();
    assert_eq!(read(&view), shown, "the blur did not commit the typing");
}

fn network(v: &apinata_app::snapshot::ConfigView) -> &apinata_core::CollectionNetwork {
    v.network.as_ref().unwrap()
}

fn proxy(v: &apinata_app::snapshot::ConfigView) -> &apinata_core::ProxyConfig {
    match &network(v).proxy {
        apinata_core::CollectionProxy::Custom(p) => p,
        other => panic!("not a custom proxy: {other:?}"),
    }
}

fn cert(v: &apinata_app::snapshot::ConfigView) -> &apinata_core::ClientCertificate {
    &network(v).certificates[0]
}

fn network_rig(cx: &mut TestAppContext) -> Rig {
    let text = format!("{ROOT}{PROXY}{CERT}{HOSTS}");
    rig(cx, &text, ConfigTarget::Collection, ConfigSection::Network)
}

#[gpui_kit::test]
fn an_uncommitted_proxy_password_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    let r = network_rig(cx);
    let (pw, host) = (
        r.net_input(cx, NetworkField::Password),
        r.net_input(cx, NetworkField::Host),
    );
    kept_through_an_unrelated_reload(&r, cx, pw, host, "new", |v| {
        proxy(v).auth.as_ref().unwrap().password.clone()
    });
}

#[gpui_kit::test]
fn an_uncommitted_proxy_password_survives_a_reload_changing_another_field(cx: &mut TestAppContext) {
    let r = network_rig(cx);
    let pw = r.net_input(cx, NetworkField::Password);
    r.type_into(cx, &pw, "new");
    let text = format!("{ROOT}{}{CERT}{HOSTS}", PROXY.replace("h.local", "g.local"));
    let host = r.net_input(cx, NetworkField::Host);
    let h = host.clone();
    r.outside_write(cx, "opencollection.yml", &text, |_| true);
    wait_for(cx, "the host to load", |cx| text_of(cx, &h) == "g.local");
    assert_eq!(text_of(cx, &pw), "{{old}}new");
}

#[gpui_kit::test]
fn a_reload_changing_the_proxy_password_replaces_the_typing(cx: &mut TestAppContext) {
    let r = network_rig(cx);
    let pw = r.net_input(cx, NetworkField::Password);
    r.type_into(cx, &pw, "new");
    let text = format!(
        "{ROOT}{}{CERT}{HOSTS}",
        PROXY.replace("{{old}}", "{{disk}}")
    );
    r.outside_write(cx, "opencollection.yml", &text, |_| true);
    let p = pw.clone();
    wait_for(cx, "the password to load", |cx| {
        text_of(cx, &p) == "{{disk}}"
    });
}

#[gpui_kit::test]
fn an_uncommitted_cert_passphrase_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    let r = network_rig(cx);
    let (pass, domain) = (
        r.cert_input(cx, CertField::Passphrase),
        r.cert_input(cx, CertField::Domain),
    );
    kept_through_an_unrelated_reload(&r, cx, pass, domain, "new", |v| cert(v).passphrase.clone());
}

#[gpui_kit::test]
fn an_uncommitted_cert_passphrase_survives_a_reload_changing_another_field(
    cx: &mut TestAppContext,
) {
    let r = network_rig(cx);
    let pass = r.cert_input(cx, CertField::Passphrase);
    r.type_into(cx, &pass, "new");
    let text = format!("{ROOT}{}{CERT}{HOSTS}", PROXY.replace("h.local", "g.local"));
    let host = r.net_input(cx, NetworkField::Host);
    r.outside_write(cx, "opencollection.yml", &text, |_| true);
    wait_for(cx, "the host to load", |cx| text_of(cx, &host) == "g.local");
    assert_eq!(text_of(cx, &pass), "{{old}}new");
}

// The typing belongs to the certificate, not the row index: another certificate with the
// same passphrase in its place takes the file's value.
#[gpui_kit::test]
fn a_reload_putting_another_cert_in_the_row_replaces_the_passphrase_typing(
    cx: &mut TestAppContext,
) {
    let r = network_rig(cx);
    let (pass, domain) = (
        r.cert_input(cx, CertField::Passphrase),
        r.cert_input(cx, CertField::Domain),
    );
    r.type_into(cx, &pass, "new");
    let other = CERT
        .replace("a.test", "b.test")
        .replace("a.pem", "b.pem")
        .replace("a.key", "b.key");
    let text = format!("{ROOT}{PROXY}{other}{HOSTS}");
    r.outside_write(cx, "opencollection.yml", &text, |_| true);
    wait_for(cx, "the domain to load", |cx| {
        text_of(cx, &domain) == "b.test"
    });
    assert_eq!(text_of(cx, &pass), "{{old}}");
}

#[gpui_kit::test]
fn a_typed_cert_path_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    let r = network_rig(cx);
    let (key, domain) = (
        r.cert_input(cx, CertField::Key),
        r.cert_input(cx, CertField::Domain),
    );
    kept_through_an_unrelated_reload(&r, cx, key, domain, "x", |v| match &cert(v).kind {
        apinata_core::CertificateKind::Pem { private_key, .. } => private_key.clone(),
        _ => String::new(),
    });
}

#[gpui_kit::test]
fn a_typed_host_override_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    let r = network_rig(cx);
    let (host, address) = (
        r.host_input(cx, HostField::Host),
        r.host_input(cx, HostField::Address),
    );
    kept_through_an_unrelated_reload(&r, cx, host, address, "x", |v| {
        network(v).host_overrides[0].host.clone()
    });
}

#[gpui_kit::test]
fn a_typed_proxy_port_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    let r = network_rig(cx);
    let (port, host) = (
        r.net_input(cx, NetworkField::Port),
        r.net_input(cx, NetworkField::Host),
    );
    kept_through_an_unrelated_reload(&r, cx, port, host, "1", |v| proxy(v).port.clone());
}

#[gpui_kit::test]
fn a_refused_proxy_port_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    let r = network_rig(cx);
    let port = r.net_input(cx, NetworkField::Port);
    r.type_into(cx, &port, "x");
    r.reload_unrelated(cx);
    assert_eq!(text_of(cx, &port), "3128x");
}

#[gpui_kit::test]
fn a_typed_presets_url_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    let text = format!(
        "{ROOT}extensions:\n  bruno:\n    presets:\n      request:\n        type: http\n        url: https://old.test\n"
    );
    let r = rig(cx, &text, ConfigTarget::Collection, ConfigSection::Presets);
    cx.update_window(r.win, |_, w, cx| {
        use gpui_kit::test::TestWindowExt as _;
        w.click("presets-url", cx);
        w.press("end", cx);
        w.input("x", cx);
    })
    .unwrap();
    cx.run_until_parked();
    r.reload_unrelated(cx);
    let url =
        r.ed.read_with(cx, |e, cx| e.presets().unwrap().read(cx).url(cx));
    assert_eq!(url, "https://old.testx");
}

fn oauth2_input(
    r: &Rig,
    cx: &mut TestAppContext,
    f: apinata_app::oauth2_editor::OAuth2Field,
) -> Entity<InputState> {
    r.ed.read_with(cx, |e, cx| {
        e.auth().read(cx).oauth2().read(cx).input(f).clone()
    })
}

#[gpui_kit::test]
fn an_uncommitted_oauth2_secret_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    use apinata_app::oauth2_editor::OAuth2Field;
    let text = format!("{ROOT}{OAUTH2}");
    let r = rig(cx, &text, ConfigTarget::Collection, ConfigSection::Auth);
    let (secret, id) = (
        oauth2_input(&r, cx, OAuth2Field::ClientSecret),
        oauth2_input(&r, cx, OAuth2Field::ClientId),
    );
    kept_through_an_unrelated_reload(&r, cx, secret, id, "new", |v| match &v.auth {
        apinata_core::AuthSpec::OAuth2(spec) => spec.client_secret.clone(),
        _ => String::new(),
    });
}

#[gpui_kit::test]
fn an_uncommitted_oauth2_secret_survives_a_reload_changing_another_field(cx: &mut TestAppContext) {
    use apinata_app::oauth2_editor::OAuth2Field;
    let text = format!("{ROOT}{OAUTH2}");
    let r = rig(cx, &text, ConfigTarget::Collection, ConfigSection::Auth);
    let secret = oauth2_input(&r, cx, OAuth2Field::ClientSecret);
    let scope = oauth2_input(&r, cx, OAuth2Field::Scope);
    r.type_into(cx, &secret, "new");
    let outside = format!("{ROOT}{}", OAUTH2.replace("scope: read", "scope: write"));
    r.outside_write(cx, "opencollection.yml", &outside, |_| true);
    wait_for(cx, "the scope to load", |cx| text_of(cx, &scope) == "write");
    assert_eq!(text_of(cx, &secret), "{{old}}new");
}

#[gpui_kit::test]
fn a_pending_tag_survives_an_unrelated_reload(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        ROOT,
        ConfigTarget::Folder(ip("users")),
        ConfigSection::Tags,
    );
    let input =
        r.ed.read_with(cx, |e, cx| e.tags().unwrap().read(cx).input().clone());
    r.type_into(cx, &input, "two");
    r.reload_unrelated(cx);
    assert_eq!(text_of(cx, &input), "two");
}
