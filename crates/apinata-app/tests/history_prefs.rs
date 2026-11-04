//! The per-request history limit: its setting, its Preferences row, and the sends it keeps.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::last_response::ResponseStore;
use apinata_app::preferences::parse_history_limit;
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

#[test]
fn the_limit_defaults_to_twenty_also_for_older_settings_files() {
    assert_eq!(Settings::default().history_limit, 20);
    let old: Settings = serde_json::from_str(r#"{"remember_responses":true}"#).unwrap();
    assert_eq!(old.history_limit, 20);
}

#[test]
fn the_limit_is_a_whole_number_from_0_to_100() {
    assert_eq!(parse_history_limit("0"), Ok(0));
    assert_eq!(parse_history_limit(" 7 "), Ok(7));
    assert_eq!(parse_history_limit("100"), Ok(100));
    for bad in ["101", "-1", "x", "", "2.5"] {
        let err = parse_history_limit(bad).unwrap_err();
        assert!(err.contains("from 0 to 100"), "{bad:?}: {err}");
    }
}

#[gpui_kit::test]
fn the_history_row_sets_the_limit(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    prefs_click(cx, &s, "history-limit");
    cx.update_window(s.win, |_, w, cx| {
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        w.input("3", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let limit = cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .history_limit
    });
    assert_eq!(limit, 3);
    assert!(present(cx, s.win, "pref-history-limit"));
}

#[gpui_kit::test]
fn sends_beyond_the_limit_drop_the_oldest(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("x"));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |v| v.history_limit = 2));
    });
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    let model = sender(cx, &s.ws);
    for _ in 0..3 {
        let r = s.root.clone();
        assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
        finished(cx, &model);
    }
    wait_real(cx, |_| {
        let seqs: Vec<u64> = store
            .history(&s.root, &ip("a.yml"))
            .iter()
            .map(|k| k.seq)
            .collect();
        seqs == [2, 1]
    });
}
