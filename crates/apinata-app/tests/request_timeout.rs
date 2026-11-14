//! The Preferences request timeout: its setting, its row, and the default reaching each send.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::persist::{Paths, save_json};
use apinata_app::preferences::parse_request_timeout;
use apinata_app::request_editor::RequestEditor;
use apinata_app::runner_model::{RowState, RunnerModel};
use apinata_app::settings::Settings;
use apinata_app::settings_section::TimeoutMode;
use apinata_app::token_panel::TokenPanel;
use apinata_core::{ExecError, Outcome, RunError};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::time::Duration;
use support::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn the_default_is_no_limit_also_for_older_settings_files() {
    assert_eq!(Settings::default().request_timeout_secs, 0);
    let old: Settings = serde_json::from_str(r#"{"remember_responses":true}"#).unwrap();
    assert_eq!(old.request_timeout_secs, 0);
    assert_eq!(Settings::default().request_timeout(), None);
    let five = Settings {
        request_timeout_secs: 5,
        ..Settings::default()
    };
    assert_eq!(five.request_timeout(), Some(Duration::from_secs(5)));
}

#[test]
fn the_row_takes_whole_seconds_up_to_86400_or_nothing() {
    assert_eq!(parse_request_timeout("0"), Ok(0));
    assert_eq!(parse_request_timeout(" 30 "), Ok(30));
    assert_eq!(parse_request_timeout("86400"), Ok(86400));
    // An empty field is no limit, as for the largest response to keep.
    assert_eq!(parse_request_timeout(""), Ok(0));
    assert_eq!(parse_request_timeout("  "), Ok(0));
    for bad in ["86401", "-1", "x", "1.5"] {
        let err = parse_request_timeout(bad).unwrap_err();
        assert!(err.contains("from 0 to 86400"), "{bad:?}: {err}");
    }
}

#[test]
fn inherit_names_the_timeout_that_applies() {
    assert_eq!(TimeoutMode::Inherit.label(None), "Default (no limit)");
    let thirty = Some(Duration::from_secs(30));
    assert_eq!(TimeoutMode::Inherit.label(thirty), "Default (30 s)");
    assert_eq!(TimeoutMode::Custom.label(thirty), "Custom");
    assert_eq!(TimeoutMode::Unlimited.label(thirty), "Never time out");
    assert_ne!(
        TimeoutMode::Unlimited.label(None),
        TimeoutMode::Inherit.label(None)
    );
}

fn timeout_shown(cx: &mut TestAppContext, view: &Entity<RequestEditor>) -> Option<String> {
    view.read_with(cx, |v, cx| v.settings().read(cx).timeout_shown(cx))
        .map(|l| l.to_string())
}

// The select of an inheriting request names the Preferences value and follows a change to it.
#[gpui_kit::test]
fn the_select_shows_the_preference_and_follows_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started_with(cx, &col, one_second);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let view = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("an editor");
    assert_eq!(timeout_shown(cx, &view).as_deref(), Some("Default (1 s)"));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.request_timeout_secs = 0));
    });
    cx.run_until_parked();
    frame(cx, s.win);
    assert_eq!(
        timeout_shown(cx, &view).as_deref(),
        Some("Default (no limit)")
    );
}

#[gpui_kit::test]
fn the_row_sets_the_timeout(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    prefs_click(cx, &s, "request-timeout");
    cx.update_window(s.win, |_, w, cx| {
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        w.input("7", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let secs = cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .request_timeout_secs
    });
    assert_eq!(secs, 7);
    assert!(present(cx, s.win, "pref-request-timeout"));
}

/// Saves settings with a one-second request timeout before the app reads them.
fn one_second(p: &Paths) {
    let settings = Settings {
        request_timeout_secs: 1,
        ..test_settings()
    };
    save_json(&p.settings, &settings).unwrap();
}

fn slow(cx: &mut TestAppContext) -> MockServer {
    mock_server(
        cx,
        ResponseTemplate::new(200).set_delay(Duration::from_secs(30)),
    )
}

#[gpui_kit::test]
fn send_stops_at_the_preference(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = slow(cx);
    let col = collection_for(&server.uri(), "");
    let s = started_with(cx, &col, one_second);
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(
        matches!(
            result.error,
            Some(RunError::Exec(ExecError::Timeout { .. }))
        ),
        "{:?}",
        result.error.as_ref().map(|e| e.to_string())
    );
}

#[gpui_kit::test]
fn the_runner_stops_at_the_preference(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = slow(cx);
    let one = request("one", &format!("{}/one", server.uri()));
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
        ("api/one.yml", &one),
    ]);
    let s = started_with(cx, &col, one_second);
    let (ws, root) = (s.ws.clone(), s.root.clone());
    let m: Entity<RunnerModel> = cx.new(|_| RunnerModel::new(ws, root, ip("api")));
    assert!(m.update(cx, |m, cx| m.start(cx)));
    wait_real(cx, |cx| m.read_with(cx, |m, _| !m.is_running()));
    let outcomes: Vec<Option<Outcome>> = m.read_with(cx, |m, _| {
        m.rows()
            .iter()
            .map(|r| match &r.state {
                RowState::Done(x) => Some(x.outcome),
                _ => None,
            })
            .collect()
    });
    assert_eq!(outcomes, [Some(Outcome::Errored)]);
}

fn token_panel(cx: &mut TestAppContext, view: &Entity<RequestEditor>) -> Entity<TokenPanel> {
    view.read_with(cx, |v, cx| {
        v.auth().read(cx).oauth2().read(cx).token_panel().cloned()
    })
    .expect("a token panel")
}

#[gpui_kit::test]
fn get_token_stops_at_the_preference(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .mount(&server),
    );
    let auth = format!(
        "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      id: main\n",
        server.uri()
    );
    let file = format!("{}{auth}", request("a", "http://x.test/r"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let s = started_with(cx, &col, one_second);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let view = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("an editor");
    let panel = token_panel(cx, &view);
    panel.update(cx, |p, cx| p.get_token(cx));
    // `wait_real` gives up after 10 s, well before the provider's 30 s delay.
    wait_real(cx, |cx| !panel.read_with(cx, |p, _| p.is_busy()));
    assert!(
        panel.read_with(cx, |p, _| p.notice()).is_some(),
        "the timeout is reported"
    );
}

// Get token reads the Preferences default after planning, as Send and the runner do, so a
// change made while the plan is on its way is the value used.
#[gpui_kit::test]
fn get_token_reads_the_default_after_planning(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
            .mount(&server),
    );
    let auth = format!(
        "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      id: main\n",
        server.uri()
    );
    let file = format!("{}{auth}", request("a", "http://x.test/r"));
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let view = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("an editor");
    let panel = token_panel(cx, &view);
    panel.update(cx, |p, cx| p.get_token(cx));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.request_timeout_secs = 1));
    });
    wait_real(cx, |cx| !panel.read_with(cx, |p, _| p.is_busy()));
    assert!(
        panel.read_with(cx, |p, _| p.notice()).is_some(),
        "the timeout set after the press is reported"
    );
}

#[test]
fn a_hand_edited_timeout_past_the_range_is_held_to_a_day() {
    let huge = Settings {
        request_timeout_secs: u32::MAX,
        ..Settings::default()
    };
    assert_eq!(huge.request_timeout(), Some(Duration::from_secs(86_400)));
}

// An explicit `timeout: 0` is the request's own "no limit", which the executor sends whatever
// the Preferences default, so it shows as Never time out; a trip through Default and back is clean.
#[gpui_kit::test]
fn an_explicit_zero_timeout_shows_as_no_limit_and_round_trips(cx: &mut TestAppContext) {
    for (timeout, shown, own) in [
        ("0", "Never time out", TimeoutMode::Unlimited),
        ("2500", "Custom", TimeoutMode::Custom),
    ] {
        let file = format!(
            "{}\nsettings:\n  encodeUrl: true\n  timeout: {timeout}\n  followRedirects: true\n",
            request("a", "http://x.test/r")
        );
        let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
        let s = started(cx, &col);
        let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
        frame(cx, s.win);
        let view = s
            .shell
            .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
            .expect("an editor");
        assert_eq!(timeout_shown(cx, &view).as_deref(), Some(shown));
        let settings = view.read_with(cx, |v, _| v.settings().clone());
        for mode in [TimeoutMode::Inherit, own] {
            cx.update_window(s.win, |_, w, cx| {
                settings.update(cx, |st, cx| st.set_timeout_mode(mode, w, cx))
            })
            .unwrap();
            cx.run_until_parked();
        }
        assert!(
            !view_of(cx, &s.ws, &s.root, "a.yml").dirty,
            "timeout {timeout}: back to {shown} is clean"
        );
        assert_eq!(
            std::fs::read_to_string(col.path().join("a.yml")).unwrap(),
            file
        );
    }
}

// A refused timeout's message follows the text as it is typed, not only at Enter or blur,
// and goes once the text is valid again.
#[gpui_kit::test]
fn the_timeout_error_follows_the_typing(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let prefs = open_prefs(cx, &s);
    prefs_click(cx, &s, "request-timeout");
    let retype = |cx: &mut TestAppContext, text: &str, enter: bool| {
        cx.update_window(s.win, |_, w, cx| {
            w.press(&key("a"), cx);
            w.press("backspace", cx);
            w.input(text, cx);
            if enter {
                w.press("enter", cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    };
    retype(cx, "999999", true);
    let unsaved = prefs.read_with(cx, |p, _| p.unsaved());
    assert!(
        unsaved.len() == 1 && unsaved[0].contains("999999"),
        "{unsaved:?}"
    );
    retype(cx, "-5", false);
    let unsaved = prefs.read_with(cx, |p, _| p.unsaved());
    assert!(
        unsaved.len() == 1 && unsaved[0].contains("\"-5\""),
        "{unsaved:?}"
    );
    retype(cx, "30", false);
    assert_eq!(
        prefs.read_with(cx, |p, _| p.unsaved()),
        Vec::<String>::new()
    );
}
