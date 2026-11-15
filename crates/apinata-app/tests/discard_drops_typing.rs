//! Discard changes drops what was typed but never committed, secrets above all: after a
//! discard, a blur and a save, the typing is in no file and no keychain entry.

use crate::discard_shows_saved::{WAYS, Way, discard};
use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::config_editor::{ConfigEditor, ConfigSection};
use apinata_app::network_section::{CertField, HostField, NetworkField, NetworkSection};
use apinata_app::oauth2_editor::OAuth2Field;
use apinata_app::tabs::TabKind;
use apinata_core::{MemoryStore, SecretStore as _};
use gpui_kit::component::input::InputState;
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext};
use std::path::Path;
use std::sync::Arc;
use support::*;

const REQUEST_OAUTH2: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://127.0.0.1:9/a\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://127.0.0.1:9/token\n    credentials:\n      clientId: svc\n      clientSecret: \"{{old}}\"\n";
const OAUTH2: &str = "request:\n  auth:\n    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: http://127.0.0.1:9/token\n    credentials:\n      clientId: svc\n      clientSecret: \"{{old}}\"\n    resourceOwner:\n      username: u\n      password: \"{{old}}\"\n";
const PROXY: &str = "config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: 127.0.0.1\n      port: \"3128\"\n      auth:\n        username: u\n        password: \"{{old}}\"\n      bypassProxy: \"\"\n";
const CERT: &str = "  clientCertificates:\n    - domain: a.test\n      type: pem\n      certificateFilePath: a.pem\n      privateKeyFilePath: a.key\n      passphrase: \"{{old}}\"\n";
const HOSTS: &str = "extensions:\n  apinata:\n    hostOverrides:\n      - host: api.test\n        address: 127.0.0.1\n";

// Every place `typed` could have landed: a file under the collection or the keychain store.
fn leaks(dir: &Path, store: &MemoryStore, typed: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for entry in std::fs::read_dir(d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if std::fs::read_to_string(&path).is_ok_and(|t| t.contains(typed)) {
                found.push(path.display().to_string());
            }
        }
    }
    for root in store.list_roots().unwrap() {
        for key in store.list(&root).unwrap() {
            if store.get(&key).unwrap().is_some_and(|v| v.contains(typed)) {
                found.push(format!("keychain {key:?}"));
            }
        }
    }
    found
}

fn text_of(cx: &mut TestAppContext, input: &Entity<InputState>) -> String {
    input.read_with(cx, |i, _| i.value().to_string())
}

// Puts `text` in `input` as typing that no Enter or blur has committed.
fn type_uncommitted(cx: &mut TestAppContext, s: &Started, input: &Entity<InputState>, text: &str) {
    cx.update_window(s.win, |_, w, cx| {
        input.update(cx, |i, cx| i.set_value(text.to_string(), w, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

// Types `text` at the end of `input` keystroke by keystroke and leaves the focus there.
fn type_keys(cx: &mut TestAppContext, s: &Started, input: &Entity<InputState>, text: &str) {
    cx.update_window(s.win, |_, w, cx| {
        w.activate_window();
        input.read(cx).focus_handle(cx).focus(w, cx);
    })
    .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "end");
    for ch in text.chars() {
        cx.simulate_keystrokes(s.win, &ch.to_string());
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn a_typed_request_oauth2_secret_is_dropped(cx: &mut TestAppContext) {
    for way in WAYS {
        let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", REQUEST_OAUTH2)]);
        let store = Arc::new(MemoryStore::new());
        let s = started_with_store(cx, &col, store.clone());
        let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
        frame(cx, s.win);
        let ed = editor_of(cx, &s, tab);
        let secret = ed.read_with(cx, |e, cx| {
            let oauth2 = e.auth().read(cx).oauth2().clone();
            oauth2.read(cx).input(OAuth2Field::ClientSecret).clone()
        });
        s.ws.update(cx, |w, cx| {
            w.send(&s.root, edit_url("a.yml", "http://127.0.0.1:9/b"), cx)
        });
        cx.run_until_parked();
        type_uncommitted(cx, &s, &secret, "typed-secret");
        discard(cx, &s, Target::Request(ip("a.yml")), way);
        assert_eq!(text_of(cx, &secret), "{{old}}", "{way:?}");
        assert!(
            !ed.read_with(cx, |e, cx| e.holds_secret_typing(cx)),
            "{way:?}"
        );
        // A save, like a send, first commits what a blur would.
        cx.update_window(s.win, |_, w, cx| ed.update(cx, |e, cx| e.save(w, cx)))
            .unwrap();
        settle(cx);
        assert_eq!(
            leaks(col.path(), &store, "typed-secret"),
            Vec::<String>::new(),
            "{way:?}"
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum ConfigWay {
    Shared(Way),
    // The Discard button of the settings editor itself.
    Button,
}

const CONFIG_WAYS: [ConfigWay; 4] = [
    ConfigWay::Shared(WAYS[0]),
    ConfigWay::Shared(WAYS[1]),
    ConfigWay::Shared(WAYS[2]),
    ConfigWay::Button,
];

struct Config {
    _col: tempfile::TempDir,
    s: Started,
    store: Arc<MemoryStore>,
    ed: Entity<ConfigEditor>,
}

impl Config {
    // The collection settings of a root file holding `extra`, open in a tab on `section` and
    // made dirty by a docs edit, so every discard path offers itself.
    fn open(cx: &mut TestAppContext, extra: &str, section: ConfigSection) -> Config {
        let root_file = format!("{ROOT}{extra}");
        let col = write_tree(&[
            ("opencollection.yml", &root_file),
            ("a.yml", &request("a", "http://127.0.0.1:9/a")),
        ]);
        let store = Arc::new(MemoryStore::new());
        let s = started_with_store(cx, &col, store.clone());
        let tab = s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Collection, cx)
        });
        let tab = tab.expect("the collection settings");
        frame(cx, s.win);
        let ed = s.shell.read_with(cx, |sh, cx| {
            sh.tab_area()
                .read(cx)
                .config_editor(tab)
                .expect("a config editor")
                .clone()
        });
        ed.update(cx, |e, cx| e.set_section(section, cx));
        frame(cx, s.win);
        let docs = Command::Edit {
            target: Target::Collection,
            edit: Edit::Docs("changed".into()),
        };
        s.ws.update(cx, |w, cx| w.send(&s.root, docs, cx));
        cx.run_until_parked();
        Config {
            _col: col,
            s,
            store,
            ed,
        }
    }

    fn network(&self, cx: &mut TestAppContext) -> Entity<NetworkSection> {
        self.ed.read_with(cx, |e, _| e.network().unwrap().clone())
    }

    fn discard(&self, cx: &mut TestAppContext, way: ConfigWay) {
        match way {
            ConfigWay::Shared(way) => discard(cx, &self.s, Target::Collection, way),
            ConfigWay::Button => {
                let ed = self.ed.clone();
                cx.update_window(self.s.win, |_, w, cx| {
                    ed.update(cx, |e, cx| e.discard_asking(w, cx))
                })
                .unwrap();
                cx.run_until_parked();
                answer_confirm(cx, "confirm-discard");
                cx.run_until_parked();
                frame(cx, self.s.win);
            }
        }
    }

    // Moves the focus off every field, as a click elsewhere does, then saves.
    fn blur_and_save(&self, cx: &mut TestAppContext) {
        let ed = self.ed.clone();
        cx.update_window(self.s.win, |_, w, cx| {
            ed.read(cx).focus_handle(cx).focus(w, cx);
        })
        .unwrap();
        frame(cx, self.s.win);
        self.ed.update(cx, |e, cx| e.save(cx));
        settle(cx);
    }

    fn leaks(&self, typed: &str) -> Vec<String> {
        leaks(&self.s.root, &self.store, typed)
    }

    fn dirty(&self, cx: &mut TestAppContext) -> bool {
        ws_snap(cx, &self.s.ws, &self.s.root)
            .content
            .collection
            .dirty
    }
}

#[gpui_kit::test]
fn a_typed_collection_oauth2_secret_is_dropped(cx: &mut TestAppContext) {
    for way in CONFIG_WAYS {
        let c = Config::open(cx, OAUTH2, ConfigSection::Auth);
        let secret = c.ed.read_with(cx, |e, cx| {
            let oauth2 = e.auth().read(cx).oauth2().clone();
            oauth2.read(cx).input(OAuth2Field::ClientSecret).clone()
        });
        type_uncommitted(cx, &c.s, &secret, "typed-secret");
        c.discard(cx, way);
        assert_eq!(text_of(cx, &secret), "{{old}}", "{way:?}");
        assert!(
            !c.ed.read_with(cx, |e, cx| e.holds_secret_typing(cx)),
            "{way:?}"
        );
        c.blur_and_save(cx);
        assert_eq!(c.leaks("typed-secret"), Vec::<String>::new(), "{way:?}");
    }
}

#[gpui_kit::test]
fn a_typed_proxy_password_is_dropped(cx: &mut TestAppContext) {
    for way in CONFIG_WAYS {
        let c = Config::open(cx, PROXY, ConfigSection::Network);
        let net = c.network(cx);
        let password = net.read_with(cx, |n, _| n.input(NetworkField::Password).clone());
        type_keys(cx, &c.s, &password, "typed");
        c.discard(cx, way);
        assert!(!c.dirty(cx), "{way:?}");
        assert_eq!(text_of(cx, &password), "{{old}}", "{way:?}");
        c.blur_and_save(cx);
        assert_eq!(c.leaks("{{old}}typed"), Vec::<String>::new(), "{way:?}");
        assert_eq!(text_of(cx, &password), "{{old}}", "{way:?}");
    }
}

#[gpui_kit::test]
fn a_typed_certificate_passphrase_is_dropped(cx: &mut TestAppContext) {
    for way in CONFIG_WAYS {
        let c = Config::open(cx, &format!("{PROXY}{CERT}"), ConfigSection::Network);
        let net = c.network(cx);
        let pass = net.read_with(cx, |n, _| {
            n.cert_input(0, CertField::Passphrase).unwrap().clone()
        });
        type_keys(cx, &c.s, &pass, "typed");
        c.discard(cx, way);
        assert!(!c.dirty(cx), "{way:?}");
        assert_eq!(text_of(cx, &pass), "{{old}}", "{way:?}");
        c.blur_and_save(cx);
        assert_eq!(c.leaks("{{old}}typed"), Vec::<String>::new(), "{way:?}");
    }
}

#[gpui_kit::test]
fn a_refused_proxy_port_goes_back_to_the_saved_port(cx: &mut TestAppContext) {
    for way in CONFIG_WAYS {
        let c = Config::open(cx, PROXY, ConfigSection::Network);
        let net = c.network(cx);
        let port = net.read_with(cx, |n, _| n.input(NetworkField::Port).clone());
        type_keys(cx, &c.s, &port, "99999");
        let refused = |cx: &mut TestAppContext| {
            net.read_with(cx, |n, cx| n.unsaved(cx).iter().any(|u| u.contains("Port")))
        };
        assert!(refused(cx), "{way:?}: the port is refused");
        c.discard(cx, way);
        assert_eq!(text_of(cx, &port), "3128", "{way:?}");
        assert!(!refused(cx), "{way:?}");
    }
}

#[gpui_kit::test]
fn unfinished_certificate_and_host_rows_are_dropped(cx: &mut TestAppContext) {
    for way in CONFIG_WAYS {
        let c = Config::open(cx, &format!("{PROXY}{CERT}{HOSTS}"), ConfigSection::Network);
        let net = c.network(cx);
        cx.update_window(c.s.win, |_, w, cx| {
            net.update(cx, |n, cx| {
                n.add_certificate(w, cx);
                n.add_host_override(w, cx);
            })
        })
        .unwrap();
        cx.run_until_parked();
        let pass = net.read_with(cx, |n, _| {
            n.cert_input(1, CertField::Passphrase).unwrap().clone()
        });
        type_uncommitted(cx, &c.s, &pass, "draft-secret");
        let host = net.read_with(cx, |n, _| n.host_input(1, HostField::Host).unwrap().clone());
        type_uncommitted(cx, &c.s, &host, "draft.test");
        c.discard(cx, way);
        let rows = net.read_with(cx, |n, _| {
            (
                n.cert_input(1, CertField::Passphrase).is_some(),
                n.host_input(1, HostField::Host).is_some(),
            )
        });
        assert_eq!(rows, (false, false), "{way:?}");
        c.blur_and_save(cx);
        assert_eq!(c.leaks("draft-secret"), Vec::<String>::new(), "{way:?}");
    }
}
