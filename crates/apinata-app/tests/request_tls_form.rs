//! The Settings tab's certificate check and HTTP version selects: their labels name the
//! Preferences value a request inherits, and each choice edits the request file.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::HttpVersionPref;
use apinata_app::settings_section::{TlsMode, VersionMode};
use apinata_core::{HttpVersionPolicy, TlsVerify};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;

#[test]
fn default_entries_name_the_preference() {
    assert_eq!(TlsMode::Inherit.label(false), "Default (Verify)");
    assert_eq!(TlsMode::Inherit.label(true), "Default (Don't verify)");
    assert_eq!(TlsMode::On.label(true), "Verify");
    assert_eq!(TlsMode::Off.label(false), "Don't verify");
    let pref = HttpVersionPref::Auto;
    assert_eq!(VersionMode::Inherit.label(pref), "Default (Auto)");
    assert_eq!(VersionMode::Http1Only.label(pref), "HTTP/1.1");
    assert_eq!(VersionMode::Auto.label(pref), "Auto");
    assert_eq!(
        VersionMode::Http2PriorKnowledge.label(pref),
        "HTTP/2 prior knowledge"
    );
    assert_eq!(
        VersionMode::Inherit.label(HttpVersionPref::Http1Only),
        "Default (HTTP/1.1)"
    );
}

#[gpui_kit::test]
fn each_choice_edits_the_request_and_default_puts_it_back(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let settings = editor_of(cx, &s, tab).read_with(cx, |e, _| e.settings().clone());
    let shown = |cx: &mut TestAppContext| {
        settings.read_with(cx, |st, cx| {
            let text = |v: Option<gpui_kit::SharedString>| v.map(|l| l.to_string());
            (text(st.tls_shown(cx)), text(st.version_shown(cx)))
        })
    };
    assert_eq!(
        shown(cx),
        (
            Some("Default (Verify)".to_owned()),
            Some("Default (HTTP/1.1)".to_owned())
        )
    );

    cx.update_window(s.win, |_, w, cx| {
        settings.update(cx, |st, cx| {
            st.set_tls_mode(TlsMode::Off, w, cx);
            st.set_version_mode(VersionMode::Http2PriorKnowledge, w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    let view = view_of(cx, &s.ws, &s.root, "a.yml");
    assert_eq!(view.settings.verify_tls, TlsVerify::Off);
    assert_eq!(
        view.settings.http_version,
        Some(HttpVersionPolicy::Http2PriorKnowledge)
    );
    assert!(view.dirty);
    assert_eq!(
        shown(cx),
        (
            Some("Don't verify".to_owned()),
            Some("HTTP/2 prior knowledge".to_owned())
        )
    );

    cx.update_window(s.win, |_, w, cx| {
        settings.update(cx, |st, cx| {
            st.set_tls_mode(TlsMode::Inherit, w, cx);
            st.set_version_mode(VersionMode::Inherit, w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    let view = view_of(cx, &s.ws, &s.root, "a.yml");
    assert_eq!(
        (view.settings.verify_tls, view.settings.http_version),
        (TlsVerify::Inherit, None)
    );
    assert!(!view.dirty, "the file is as it was");
}

#[gpui_kit::test]
fn the_default_entries_follow_preferences(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let settings = editor_of(cx, &s, tab).read_with(cx, |e, _| e.settings().clone());
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| {
                st.insecure = true;
                st.http_version = HttpVersionPref::Auto;
            })
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    let shown = settings.read_with(cx, |st, cx| {
        (
            st.tls_shown(cx).map(|l| l.to_string()),
            st.version_shown(cx).map(|l| l.to_string()),
        )
    });
    assert_eq!(
        shown,
        (
            Some("Default (Don't verify)".to_owned()),
            Some("Default (Auto)".to_owned())
        )
    );
}

const BADGE: &str = "tls-unverified";

fn badge_label(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        w.try_find(BADGE)
            .map(|e| e.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_request_line_marks_a_request_that_skips_the_check(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    assert_eq!(badge_label(cx, s.win), None, "inherits verification");
    let settings = editor_of(cx, &s, tab).read_with(cx, |e, _| e.settings().clone());
    let pick = |cx: &mut TestAppContext, mode: TlsMode| {
        cx.update_window(s.win, |_, w, cx| {
            settings.update(cx, |st, cx| st.set_tls_mode(mode, w, cx))
        })
        .unwrap();
        cx.run_until_parked();
    };
    // The unsaved choice already shows, since that is what Send would use.
    pick(cx, TlsMode::Off);
    assert_eq!(
        badge_label(cx, s.win).as_deref(),
        Some("Certificate checks are off for this request")
    );
    pick(cx, TlsMode::On);
    assert_eq!(badge_label(cx, s.win), None);
}

#[gpui_kit::test]
fn a_file_that_skips_the_check_shows_the_mark_when_opened(cx: &mut TestAppContext) {
    let off = request("a", "https://x/0") + "settings:\n  verifySsl: false\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &off)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    assert!(badge_label(cx, s.win).is_some());
}

// Preferences turning the check off everywhere is the user's own choice, not this request's.
#[gpui_kit::test]
fn the_preference_alone_shows_no_mark(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.insecure = true));
    });
    open_tab(cx, &s.ws, &s.root, "a.yml");
    assert_eq!(badge_label(cx, s.win), None);
}
