//! The HTTP version preference: stored kebab-case, chosen in Preferences > Network, applied
//! to the executor.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::executor::{self, exec_options};
use apinata_app::settings::{HttpVersionPref, Settings};
use apinata_core::{Executor, HttpVersion};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::sync::Arc;
use support::*;
use wiremock::ResponseTemplate;

fn http_version(cx: &mut TestAppContext) -> HttpVersionPref {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().http_version)
}

#[test]
fn http1_only_is_the_default_also_for_older_settings_files() {
    assert_eq!(Settings::default().http_version, HttpVersionPref::Http1Only);
    let old: Settings = serde_json::from_str(r#"{"insecure":false}"#).unwrap();
    assert_eq!(old.http_version, HttpVersionPref::Http1Only);
}

#[test]
fn each_choice_round_trips_in_kebab_case() {
    for (pref, text) in [
        (HttpVersionPref::Http1Only, "http1-only"),
        (HttpVersionPref::Auto, "auto"),
        (
            HttpVersionPref::Http2PriorKnowledge,
            "http2-prior-knowledge",
        ),
    ] {
        let s = Settings {
            http_version: pref,
            ..Settings::default()
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["http_version"], text);
        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(back.http_version, pref);
    }
    assert!(serde_json::from_str::<Settings>(r#"{"http_version":"h3"}"#).is_err());
}

#[gpui_kit::test]
fn the_network_section_switches_the_version(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        assert!(
            w.within("prefs-network")
                .try_find("pref-http-version")
                .is_some()
        );
    })
    .unwrap();
    assert_eq!(http_version(cx), HttpVersionPref::Http1Only);
    // Every segment, the last included, must sit inside the dialog's rows to be clickable.
    prefs_reveal(cx, &s, "http-version");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let rows = w.find("preferences-rows").bounds();
        let last = w.within("http-version").find(2usize).bounds();
        assert!(
            last.left() >= rows.left() && last.right() <= rows.right(),
            "segment 2 {last:?} overflows the rows {rows:?}"
        );
        assert!(
            w.within("pref-http-version")
                .try_find("prefs-http-version-hint")
                .is_some()
        );
    })
    .unwrap();
    for (ix, pref) in [
        (1usize, HttpVersionPref::Auto),
        (2, HttpVersionPref::Http2PriorKnowledge),
        (0, HttpVersionPref::Http1Only),
    ] {
        prefs_reveal(cx, &s, "http-version");
        cx.update_window(s.win, |_, w, cx| w.within("http-version").click(ix, cx))
            .unwrap();
        frame(cx, s.win);
        assert_eq!(http_version(cx), pref, "segment {ix}");
    }
}

#[gpui_kit::test]
fn the_preference_reaches_the_executor(cx: &mut TestAppContext) {
    http_ready(cx);
    let settings = Settings {
        http_version: HttpVersionPref::Http2PriorKnowledge,
        ..test_settings()
    };
    let exec = Executor::new(exec_options(&settings)).unwrap();
    cx.update(|cx| cx.set_global(executor::SharedExecutor(Arc::new(exec))));
    let server = mock_server(cx, ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let res = finished(cx, &model).response.clone().expect("a response");
    assert_eq!(res.timeline()[0].version(), Some(HttpVersion::Http2));
}

// Like every other segmented control in Preferences, the version picker hugs its segments
// instead of stretching across the control column its hint fills.
#[gpui_kit::test]
fn the_version_picker_takes_its_own_width(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    prefs_reveal(cx, &s, "http-version");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let picker = w.find("http-version").bounds();
        let last = w.within("http-version").find(2usize).bounds();
        let hint = w.find("prefs-http-version-hint").bounds();
        assert!(
            picker.right() - last.right() <= gpui_kit::px(4.),
            "picker {picker:?} runs past its last segment {last:?}"
        );
        assert!(
            picker.size.width < hint.size.width,
            "{picker:?} vs {hint:?}"
        );
    })
    .unwrap();
}
