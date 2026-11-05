//! A newer build may write a kind this one does not know in any list or enum position of
//! `session.json`; only that entry is lost and every other field loads.

use apinata_app::session::{
    SavedExample, SavedExampleTab, SavedRunnerOptions, Session, WindowMode,
};
use std::path::Path;

const BASE: &str = r#"{
  "recent": ["/c"], "open": ["/c"], "sidebar_width": 250.0, "global_environment": "g",
  "window": {"x": 1.0, "y": 2.0, "width": 800.0, "height": 600.0, "mode": "windowed"},
  "collections": {"/c": {
    "tabs": ["a.yml"], "environment": "dev",
    "examples": [{"item": "a.yml", "index": 0}],
    "runners": {"": {"bail": true, "delay_ms": 5, "fail_on_status": "error"}},
    "expanded": ["f"]
  }}
}"#;

fn load(json: &str) -> Session {
    serde_json::from_str(json).expect("a newer session loads")
}

// Every field of BASE besides the one a test changes.
fn assert_rest(s: &Session) {
    assert_eq!(s.open, [Path::new("/c")]);
    assert_eq!(s.recent, [Path::new("/c")]);
    assert_eq!(s.sidebar_width, Some(250.0));
    assert_eq!(s.global_environment.as_deref(), Some("g"));
    let c = &s.collections[Path::new("/c")];
    assert_eq!(c.environment.as_deref(), Some("dev"));
    assert_eq!(c.expanded, Some(vec!["f".to_owned()]));
}

#[test]
fn an_unknown_window_mode_restores_the_window_as_windowed() {
    let s = load(&BASE.replace(r#""mode": "windowed""#, r#""mode": "tiled_left""#));
    let window = s.window.as_ref().expect("the window is kept");
    assert_eq!(window.mode, WindowMode::Windowed);
    assert_eq!((window.width, window.height), (800.0, 600.0));
    assert_rest(&s);
    assert_eq!(s.collections[Path::new("/c")].tabs, ["a.yml"]);
}

#[test]
fn an_unknown_example_entry_is_dropped_alone() {
    let s = load(&BASE.replace(
        r#""examples": ["#,
        r#""examples": [{"item": "b.yml", "index": "first"}, {"graph": 1}, "#,
    ));
    let examples = &s.collections[Path::new("/c")].examples;
    let kept = SavedExample {
        item: "a.yml".into(),
        index: 0,
    };
    assert_eq!(examples, &[SavedExampleTab::from(kept)]);
    assert_rest(&s);
}

#[test]
fn an_unknown_runner_entry_is_dropped_alone() {
    let s = load(&BASE.replace(
        r#""runners": {"#,
        r#""runners": {"f": {"bail": "sometimes"}, "g": "matrix", "#,
    ));
    let runners = &s.collections[Path::new("/c")].runners;
    assert_eq!(runners.keys().collect::<Vec<_>>(), [""]);
    let kept = SavedRunnerOptions {
        bail: true,
        delay_ms: 5,
        fail_on_status: Some(apinata_core::StatusRule::Error),
        ..SavedRunnerOptions::default()
    };
    assert_eq!(runners[""], kept);
    assert_rest(&s);
}

// Regression guard: this case already loaded before the unknown-kinds fix.
#[test]
fn an_unknown_status_rule_loads_as_no_rule() {
    let s = load(&BASE.replace(r#""error""#, r#""teapot""#));
    let runner = &s.collections[Path::new("/c")].runners[""];
    assert_eq!(runner.fail_on_status, None);
    assert!(runner.bail);
    assert_rest(&s);
}

#[test]
fn an_unknown_tab_entry_is_dropped_alone() {
    let s = load(&BASE.replace(
        r#""tabs": ["#,
        r#""tabs": [{"request": "b.yml", "split": true}, "#,
    ));
    assert_eq!(s.collections[Path::new("/c")].tabs, ["a.yml"]);
    assert_rest(&s);
}
