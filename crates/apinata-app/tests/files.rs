use apinata_app::persist::{Loaded, Paths, load_json, save_json};
use apinata_app::session::{
    CollectionSession, RECENT_LIMIT, SavedBounds, SavedExample, SavedExampleTab, Session,
};
use apinata_app::settings::{Orientation, Settings, ThemeChoice};
use gpui_kit::{Bounds, Pixels, WindowBounds, point, px, size};
use std::path::PathBuf;

fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
}

#[test]
fn defaults_match_the_spec() {
    let s = Settings::default();
    assert_eq!(s.theme, ThemeChoice::System);
    assert_eq!(s.response_orientation, Orientation::Vertical);
    assert_eq!(s.max_body_bytes, Some(10_485_760));
    assert!(!s.insecure);
    assert_eq!((s.zoom, s.response_font_offset), (1.0, 0.0));
}

#[test]
fn settings_round_trip_through_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/settings.json");
    let s = Settings {
        theme: ThemeChoice::Dark,
        insecure: true,
        max_body_bytes: None,
        ..Settings::default()
    };
    save_json(&path, &s).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"theme\": \"dark\""), "{text}");
    let Loaded { value, recovered } = load_json::<Settings>(&path);
    assert_eq!(value, s);
    assert!(recovered.is_none());
}

#[test]
fn a_missing_file_gives_defaults_silently() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = load_json::<Session>(&dir.path().join("session.json"));
    assert_eq!(loaded.value, Session::default());
    assert!(loaded.recovered.is_none());
}

#[test]
fn missing_fields_take_defaults_and_unknown_fields_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, r#"{"insecure": true, "from_the_future": 1}"#).unwrap();
    let loaded = load_json::<Settings>(&path);
    assert!(loaded.value.insecure);
    assert_eq!(loaded.value.theme, ThemeChoice::System);
    assert!(loaded.recovered.is_none());
}

#[test]
fn a_corrupt_file_is_set_aside_as_bad() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, "{not json").unwrap();
    let loaded = load_json::<Settings>(&path);
    assert_eq!(loaded.value, Settings::default());
    let message = loaded.recovered.expect("a recovery message");
    assert!(message.contains("settings.json.bad"), "{message}");
    assert!(!path.exists());
    let bad = dir.path().join("settings.json.bad");
    assert_eq!(std::fs::read_to_string(bad).unwrap(), "{not json");
}

#[test]
fn saving_replaces_the_file_and_leaves_no_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut s = Session::default();
    save_json(&path, &s).unwrap();
    s.sidebar_width = Some(300.0);
    save_json(&path, &s).unwrap();
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["session.json"]);
    assert_eq!(load_json::<Session>(&path).value, s);
}

#[test]
fn platform_paths_live_in_the_config_and_data_directories() {
    let p = Paths::platform().expect("a home directory");
    assert_eq!(
        p.settings,
        dirs::config_dir().unwrap().join("apinata/settings.json")
    );
    assert_eq!(
        p.session,
        dirs::data_local_dir().unwrap().join("apinata/session.json")
    );
    let t = Paths::in_dir(std::path::Path::new("/x"));
    assert_eq!(
        (t.settings, t.session),
        ("/x/settings.json".into(), "/x/session.json".into())
    );
}

#[test]
fn remember_moves_a_root_to_the_front_and_caps_the_list() {
    let mut s = Session::default();
    for i in 0..12 {
        s.remember(&PathBuf::from(format!("/c/{i}")));
    }
    s.remember(&PathBuf::from("/c/5"));
    assert_eq!(s.recent.len(), RECENT_LIMIT);
    assert_eq!(s.recent[0], PathBuf::from("/c/5"));
    assert_eq!(s.recent[1], PathBuf::from("/c/11"));
    assert_eq!(
        s.recent
            .iter()
            .filter(|p| p.as_path() == std::path::Path::new("/c/5"))
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn a_non_utf8_root_is_not_remembered() {
    use std::os::unix::ffi::OsStrExt;
    let mut s = Session::default();
    let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/c/\xff"));
    assert!(!s.remember(&odd));
    assert!(s.recent.is_empty());
    assert!(s.remember(&PathBuf::from("/c/ok")));
}

#[test]
fn session_round_trips_with_per_collection_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut s = Session {
        open: vec!["/c/a".into()],
        sidebar_width: Some(280.0),
        ..Session::default()
    };
    let c = CollectionSession {
        tabs: vec!["users/list.yml".into()],
        environment: Some("dev".into()),
        preview: Some("users/list.yml".into()),
        examples: vec![SavedExampleTab {
            example: SavedExample {
                item: "users/list.yml".into(),
                index: 0,
            },
            position: Some(1),
        }],
        panels: vec![],
        pinned: vec![],
        active: None,
        active_draft: None,
        expanded: Some(vec!["".into(), "users".into()]),
        runners: Default::default(),
        mock: true,
    };
    s.collections.insert("/c/a".into(), c);
    s.window = Some(SavedBounds::from_window(WindowBounds::Maximized(b(
        10., 20., 800., 600.,
    ))));
    save_json(&path, &s).unwrap();
    assert_eq!(load_json::<Session>(&path).value, s);
}

#[test]
fn saved_bounds_restore_on_a_visible_display() {
    let screens = [b(0., 0., 1920., 1080.)];
    for wb in [
        WindowBounds::Windowed(b(100., 80., 900., 700.)),
        WindowBounds::Maximized(b(0., 0., 1200., 800.)),
        WindowBounds::Fullscreen(b(0., 0., 1920., 1080.)),
    ] {
        assert_eq!(SavedBounds::from_window(wb).to_window(&screens), Some(wb));
    }
}

#[test]
fn bounds_off_every_display_or_too_small_are_dropped() {
    let screens = [b(0., 0., 1920., 1080.)];
    let off = SavedBounds::from_window(WindowBounds::Windowed(b(5000., 5000., 800., 600.)));
    assert_eq!(off.to_window(&screens), None);
    let tiny = SavedBounds::from_window(WindowBounds::Windowed(b(0., 0., 100., 80.)));
    assert_eq!(tiny.to_window(&screens), None);
}

#[test]
fn dismissed_tips_default_to_none_and_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, r#"{"insecure": true}"#).unwrap();
    let loaded = load_json::<Settings>(&path);
    assert!(loaded.recovered.is_none());
    assert!(loaded.value.dismissed_tips.is_empty());
    let settings = Settings {
        dismissed_tips: vec!["example-commit-note".into()],
        ..Settings::default()
    };
    save_json(&path, &settings).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains(r#""dismissed_tips": ["#),
        "the flag is stored under its own key: {text}"
    );
    assert_eq!(load_json::<Settings>(&path).value, settings);
}

#[test]
fn drafts_live_in_their_own_directory_of_the_data_dir() {
    let dir = std::path::Path::new("/tmp/apinata-profile");
    assert_eq!(
        apinata_app::persist::Paths::in_dir(dir).drafts,
        dir.join("drafts")
    );
}
