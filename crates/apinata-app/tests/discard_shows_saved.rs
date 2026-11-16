//! Discard changes shows what the file holds, also in a choice picked but never sent: the
//! request timeout's Custom with no limit yet, and the collection proxy's Custom with no host.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::keymap::DiscardChanges;
use apinata_app::network_section::{NetworkField, ProxyChoice};
use apinata_app::request_editor::Section;
use apinata_app::settings_section::TimeoutMode;
use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use support::*;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Way {
    // The tab's right-click menu.
    TabMenu,
    // The Discard changes command on the active tab.
    Command,
    // A Revert sent straight to the workspace, as the changed-on-disk banner does.
    Banner,
}

pub(crate) const WAYS: [Way; 3] = [Way::TabMenu, Way::Command, Way::Banner];

pub(crate) fn discard(cx: &mut TestAppContext, s: &Started, target: Target, way: Way) {
    match way {
        Way::TabMenu => {
            cx.update_window(s.win, |_, w, cx| w.right_click(("tab", 0usize), cx))
                .unwrap();
            frame(cx, s.win);
            click_menu(cx, s.win, "Discard changes");
            answer_confirm(cx, "confirm-discard");
        }
        Way::Command => {
            let shell = s.shell.clone();
            cx.update_window(s.win, |_, w, cx| {
                shell.read(cx).focus_handle(cx).focus(w, cx);
                w.dispatch_action(Box::new(DiscardChanges), cx);
            })
            .unwrap();
            cx.run_until_parked();
            answer_confirm(cx, "confirm-discard");
        }
        Way::Banner => {
            s.ws.update(cx, |w, cx| w.send(&s.root, Command::Revert(target), cx));
        }
    }
    cx.run_until_parked();
    frame(cx, s.win);
}

fn with_timeout(timeout: Option<&str>) -> String {
    let base = request("a", "http://x/0");
    match timeout {
        Some(t) => format!("{base}settings:\n  encodeUrl: true\n  timeout: {t}\n"),
        None => base,
    }
}

// Opens `a.yml` holding `timeout`, makes it dirty elsewhere, runs `change` on its timeout and
// discards; the pane must show the mode and field it showed when it opened.
fn timeout_is_put_back(
    cx: &mut TestAppContext,
    timeout: Option<&str>,
    saved: TimeoutMode,
    change: impl Fn(&mut TestAppContext, &Started),
) {
    for way in WAYS {
        let col = write_tree(&[
            ("opencollection.yml", ROOT),
            ("a.yml", &with_timeout(timeout)),
        ]);
        let s = started(cx, &col);
        let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
        frame(cx, s.win);
        let settings = editor_of(cx, &s, tab).read_with(cx, |e, _| e.settings().clone());
        let shown = |cx: &mut TestAppContext| {
            settings.read_with(cx, |st, cx| {
                let field = st.timeout_input().read(cx).value().to_string();
                (st.timeout_mode(), st.timeout_shown(cx), field)
            })
        };
        let before = shown(cx);
        assert_eq!(before.0, saved);
        s.ws.update(cx, |w, cx| {
            w.send(&s.root, edit_url("a.yml", "http://changed"), cx)
        });
        cx.run_until_parked();
        change(cx, &s);
        cx.run_until_parked();
        assert_ne!(shown(cx).0, saved, "{way:?}: the change shows");
        discard(cx, &s, Target::Request(ip("a.yml")), way);
        assert!(!view_of(cx, &s.ws, &s.root, "a.yml").dirty, "{way:?}");
        let after = shown(cx);
        assert_eq!((after.0, &after.1), (before.0, &before.1), "{way:?}");
        if saved == TimeoutMode::Custom {
            assert_eq!(after.2, before.2, "{way:?}: the saved limit");
        }
    }
}

fn pick(mode: TimeoutMode) -> impl Fn(&mut TestAppContext, &Started) {
    move |cx, s| {
        let tab = s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().id);
        let settings = editor_of(cx, s, tab).read_with(cx, |e, _| e.settings().clone());
        cx.update_window(s.win, |_, w, cx| {
            settings.update(cx, |st, cx| st.set_timeout_mode(mode, w, cx))
        })
        .unwrap();
    }
}

// Types `limit` into the Custom field of the Settings section, as a person would.
fn type_limit(limit: &'static str) -> impl Fn(&mut TestAppContext, &Started) {
    move |cx, s| {
        pick(TimeoutMode::Custom)(cx, s);
        cx.update_window(s.win, |_, w, cx| {
            w.click(("section", Section::Settings as usize), cx)
        })
        .unwrap();
        frame(cx, s.win);
        replace_input(cx, s.win, None, "set-timeout", limit);
    }
}

#[gpui_kit::test]
fn an_empty_custom_timeout_goes_back_to_no_limit(cx: &mut TestAppContext) {
    timeout_is_put_back(
        cx,
        Some("0"),
        TimeoutMode::Unlimited,
        pick(TimeoutMode::Custom),
    );
}

#[gpui_kit::test]
fn an_empty_custom_timeout_goes_back_to_the_default(cx: &mut TestAppContext) {
    timeout_is_put_back(cx, None, TimeoutMode::Inherit, pick(TimeoutMode::Custom));
}

#[gpui_kit::test]
fn a_typed_custom_timeout_goes_back_to_no_limit(cx: &mut TestAppContext) {
    timeout_is_put_back(cx, Some("0"), TimeoutMode::Unlimited, type_limit("500"));
}

#[gpui_kit::test]
fn a_saved_custom_timeout_comes_back_with_its_limit(cx: &mut TestAppContext) {
    timeout_is_put_back(
        cx,
        Some("2500"),
        TimeoutMode::Custom,
        pick(TimeoutMode::Unlimited),
    );
}

const PROXY: &str = "config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: h.local\n      port: \"3128\"\n      bypassProxy: \"\"\n";

// Opens the collection settings of a root holding `extra`, makes them dirty elsewhere, runs
// `change` on the proxy and discards; the proxy must show the choice and host the file holds.
fn proxy_is_put_back(
    cx: &mut TestAppContext,
    extra: &str,
    saved: (ProxyChoice, &str),
    change: impl Fn(
        &mut TestAppContext,
        &Started,
        &gpui_kit::Entity<apinata_app::network_section::NetworkSection>,
    ),
) {
    for way in WAYS {
        let root_file = format!("{ROOT}{extra}");
        let col = write_tree(&[
            ("opencollection.yml", &root_file),
            ("a.yml", &request("a", "http://x/0")),
        ]);
        let s = started(cx, &col);
        let tab = s.ws.update(cx, |w, cx| {
            w.open_settings(&s.root, TabKind::Collection, cx)
        });
        let tab = tab.expect("the collection settings");
        frame(cx, s.win);
        let net = s.shell.read_with(cx, |sh, cx| {
            let ed = sh
                .tab_area()
                .read(cx)
                .config_editor(tab)
                .expect("a config editor");
            ed.read(cx).network().expect("the network section").clone()
        });
        let shown = |cx: &mut TestAppContext| {
            net.read_with(cx, |n, cx| {
                let host = n.input(NetworkField::Host).read(cx).value().to_string();
                (n.choice(), host)
            })
        };
        let docs = Command::Edit {
            target: Target::Collection,
            edit: Edit::Docs("changed".into()),
        };
        s.ws.update(cx, |w, cx| w.send(&s.root, docs, cx));
        cx.run_until_parked();
        change(cx, &s, &net);
        cx.run_until_parked();
        assert_ne!(
            shown(cx),
            (saved.0, saved.1.to_owned()),
            "{way:?}: the change shows"
        );
        discard(cx, &s, Target::Collection, way);
        let snap = ws_snap(cx, &s.ws, &s.root);
        assert!(!snap.content.collection.dirty, "{way:?}");
        assert_eq!(shown(cx), (saved.0, saved.1.to_owned()), "{way:?}");
    }
}

#[gpui_kit::test]
fn a_custom_proxy_without_a_host_goes_back_to_inherit(cx: &mut TestAppContext) {
    proxy_is_put_back(cx, "", (ProxyChoice::Inherit, ""), |cx, s, net| {
        cx.update_window(s.win, |_, w, cx| {
            net.update(cx, |n, cx| n.set_choice(ProxyChoice::Custom, w, cx))
        })
        .unwrap();
    });
}

#[gpui_kit::test]
fn a_cleared_proxy_host_comes_back(cx: &mut TestAppContext) {
    proxy_is_put_back(cx, PROXY, (ProxyChoice::Custom, "h.local"), |cx, s, net| {
        let host = net.read_with(cx, |n, _| n.input(NetworkField::Host).clone());
        cx.update_window(s.win, |_, w, cx| {
            host.update(cx, |i, cx| i.set_value("", w, cx))
        })
        .unwrap();
    });
}
