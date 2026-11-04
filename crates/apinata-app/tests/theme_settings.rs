use apinata_app::persist::Paths;
use apinata_app::settings::{Settings, ThemeChoice};
use std::path::Path;

#[test]
fn old_settings_json_loads_with_theme_defaults_and_keeps_its_values() {
    let old = r#"{"theme":"dark","zoom":1.4,"response_font_offset":2.0}"#;
    let s: Settings = serde_json::from_str(old).unwrap();
    assert_eq!(s.theme, ThemeChoice::Dark);
    assert_eq!(s.zoom, 1.4);
    assert_eq!(s.response_font_offset, 2.0);
    assert_eq!(s.light_theme, None);
    assert_eq!(s.dark_theme, None);
    assert_eq!(s.editor_font_offset, 0.0);
}

#[test]
fn theme_names_and_editor_offset_round_trip() {
    let s = Settings {
        light_theme: Some("Paper".into()),
        dark_theme: Some("Apinata High Contrast Dark".into()),
        editor_font_offset: 3.0,
        ..Settings::default()
    };
    let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
}

#[test]
fn in_dir_puts_user_themes_in_a_themes_folder() {
    let paths = Paths::in_dir(Path::new("/p"));
    assert_eq!(paths.themes, Path::new("/p/themes"));
}
