//! The Preferences script time limit: its setting, its row, and a runaway script stopped by it.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::preferences::parse_script_timeout;
use apinata_app::settings::Settings;
use apinata_core::Verdict;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

#[test]
fn the_limit_defaults_to_thirty_seconds_also_for_older_settings_files() {
    assert_eq!(Settings::default().script_timeout_secs, 30);
    let old: Settings = serde_json::from_str(r#"{"remember_responses":true}"#).unwrap();
    assert_eq!(old.script_timeout_secs, 30);
}

#[test]
fn the_limit_is_whole_seconds_from_1_to_3600() {
    assert_eq!(parse_script_timeout("1"), Ok(1));
    assert_eq!(parse_script_timeout(" 90 "), Ok(90));
    assert_eq!(parse_script_timeout("3600"), Ok(3600));
    for bad in ["0", "3601", "-1", "x", "", "1.5"] {
        let err = parse_script_timeout(bad).unwrap_err();
        assert!(err.contains("from 1 to 3600"), "{bad:?}: {err}");
    }
}

#[gpui_kit::test]
fn the_row_sets_the_limit(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    prefs_click(cx, &s, "script-timeout");
    cx.update_window(s.win, |_, w, cx| {
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        w.input("5", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let limit = cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .script_timeout_secs
    });
    assert_eq!(limit, 5);
    assert!(present(cx, s.win, "pref-script-timeout"));
}

#[gpui_kit::test]
fn a_runaway_test_script_stops_at_the_preference(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: tests\n      code: \"while (true) {{}}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |v| v.script_timeout_secs = 1));
    });
    let started_at = std::time::Instant::now();
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(started_at.elapsed() < std::time::Duration::from_secs(20));
    let entry = result
        .tests
        .iter()
        .find(|t| t.script_error)
        .expect("a script error entry");
    match &entry.verdict {
        Verdict::Fail {
            detail: Some(d), ..
        } => assert!(d.contains("exceeded the 1 s limit"), "{d}"),
        other => panic!("not a failure: {other:?}"),
    }
}
