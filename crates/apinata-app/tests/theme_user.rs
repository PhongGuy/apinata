//! `theme::load_user_themes` registers valid theme files and turns every bad one into one
//! Console warning naming it.

use crate::support;

use apinata_app::persist::Paths;
use apinata_app::theme::{self, DARK, THEME_JSON};
use apinata_core::ConsoleLevel;
use gpui_kit::TestAppContext;
use gpui_kit::component::ThemeRegistry;
use serde_json::{Value, json};
use std::path::Path;

/// A one-theme set copied from the shipped dark theme under `name`.
fn set_named(name: &str, edit: impl FnOnce(&mut Value)) -> String {
    let mut set: Value = serde_json::from_str(THEME_JSON).unwrap();
    let mut theme = set["themes"][0].take();
    theme["name"] = json!(name);
    edit(&mut theme);
    json!({ "name": name, "themes": [theme] }).to_string()
}

fn write(dir: &Path, file: &str, text: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), text).unwrap();
}

fn registered(cx: &mut TestAppContext, name: &str) -> bool {
    cx.update(|cx| {
        ThemeRegistry::global(cx)
            .themes()
            .keys()
            .any(|n| n.as_ref() == name)
    })
}

fn console(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        apinata_app::log_feed::ensure(cx)
            .console_since(0)
            .entries
            .iter()
            .map(|l| {
                assert_eq!(l.level, ConsoleLevel::Warn);
                l.text.clone()
            })
            .collect()
    })
}

fn load(cx: &mut TestAppContext, dir: &Path) -> Vec<String> {
    support::init(cx);
    cx.update(|cx| theme::load_user_themes(dir, cx))
}

#[gpui_kit::test]
fn a_valid_file_is_registered_without_a_message(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "mine.json", &set_named("Mine", |_| {}));
    assert!(load(cx, tmp.path()).is_empty());
    assert!(registered(cx, "Mine"));
    assert!(console(cx).is_empty());
}

#[gpui_kit::test]
fn each_bad_file_gives_one_console_message_and_stays_out(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "a-bad-json.json", "{ not json");
    write(tmp.path(), "b-shape.json", r#"{"themes": 5}"#);
    let bad_colour = set_named("Bad Colour", |t| t["colors"]["background"] = json!("nope"));
    write(tmp.path(), "c-colour.json", &bad_colour);
    write(tmp.path(), "d-good.json", &set_named("Good", |_| {}));
    let notes = load(cx, tmp.path());
    assert_eq!(notes.len(), 3, "{notes:?}");
    for (note, file) in notes.iter().zip(["a-bad-json", "b-shape", "c-colour"]) {
        assert!(note.contains(file), "{note}");
    }
    assert!(!registered(cx, "Bad Colour"));
    assert!(registered(cx, "Good"));
    // The same messages reached the Console tab's log, once each.
    assert_eq!(console(cx), notes);
}

#[gpui_kit::test]
fn a_file_reusing_a_bundled_name_does_not_replace_it(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let text = set_named(DARK, |t| t["colors"]["background"] = json!("#010203"));
    write(tmp.path(), "dup.json", &text);
    load(cx, tmp.path());
    cx.update(|cx| {
        let themes = ThemeRegistry::global(cx).themes();
        let dark = themes.get(DARK).unwrap();
        assert_ne!(dark.colors.background.as_deref(), Some("#010203"));
    });
}

#[gpui_kit::test]
fn a_missing_directory_is_fine_and_is_not_created(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    assert!(load(cx, &dir).is_empty());
    assert!(!dir.exists());
    assert!(console(cx).is_empty());
}

#[gpui_kit::test]
fn start_loads_the_profile_themes_and_logs_bad_ones(cx: &mut TestAppContext) {
    let col = support::write_tree(&[("opencollection.yml", support::ROOT)]);
    let _s = support::started_with(cx, &col, |paths: &Paths| {
        write(&paths.themes, "ok.json", &set_named("From Start", |_| {}));
        write(&paths.themes, "broken.json", "[");
    });
    assert!(registered(cx, "From Start"));
    let lines = console(cx);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("broken"));
}

#[gpui_kit::test]
fn a_gradient_background_is_accepted(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let text = set_named("Gradient", |t| {
        t["colors"]["background"] = json!("linear-gradient(135deg, #4F46E5, #06B6D4)");
    });
    write(tmp.path(), "g.json", &text);
    assert!(load(cx, tmp.path()).is_empty());
    assert!(registered(cx, "Gradient"));
}

#[gpui_kit::test]
fn short_and_non_ascii_hex_colours_are_skipped_without_a_panic(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    for (file, bad) in [("a.json", "#12"), ("b.json", "#\u{e9}\u{e9}")] {
        let text = set_named(&format!("Bad {file}"), |t| {
            t["colors"]["background"] = json!(bad);
        });
        write(tmp.path(), file, &text);
    }
    let notes = load(cx, tmp.path());
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(!registered(cx, "Bad a.json") && !registered(cx, "Bad b.json"));
}
