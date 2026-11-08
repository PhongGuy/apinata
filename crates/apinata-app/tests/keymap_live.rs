//! Live re-binding: keymap.json loads at start, edits apply at once and survive a restart.

use crate::support;

use apinata_app::keymap::{self, bindings, bindings_with};
use apinata_app::persist::Paths;
use apinata_app::shortcuts::Overrides;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, KeyBinding, TestAppContext};
use support::*;

const INSPECTOR: &str = "inspector::ToggleInspector";

fn listed(cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .map(|b| format!("{b:?}"))
            .collect()
    })
}

fn debug(list: &[KeyBinding]) -> Vec<String> {
    list.iter().map(|b| format!("{b:?}")).collect()
}

fn sidebar_on_backslash() -> Overrides {
    overrides(&[("view.toggle_sidebar", &["secondary-\\"])])
}

fn has_filter_guard(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| {
        cx.key_bindings().borrow().bindings().any(|b| {
            b.action().name() == "zed::NoAction"
                && b.predicate()
                    .is_some_and(|p| p.to_string() == "SidebarFilter > Input")
        })
    })
}

fn sidebar_shown(cx: &mut TestAppContext, s: &Started) -> bool {
    s.shell.read_with(cx, |v, _| v.sidebar_visible())
}

fn notes(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[test]
fn the_keymap_sits_beside_the_settings() {
    let p = Paths::in_dir(std::path::Path::new("/x"));
    assert_eq!(p.keymap, std::path::Path::new("/x/keymap.json"));
    if let Some(p) = Paths::platform() {
        assert_eq!(p.keymap.parent(), p.settings.parent());
    }
}

#[gpui_kit::test]
fn the_kit_bindings_stay_first_through_init_and_apply(cx: &mut TestAppContext) {
    support::init(cx);
    let kit = listed(cx)
        .into_iter()
        .filter(|b| !b.contains(INSPECTOR))
        .collect::<Vec<_>>();
    assert!(!kit.is_empty(), "the kit binds keys at init");
    cx.update(keymap::init);
    assert_eq!(
        listed(cx),
        [kit.clone(), debug(&cx.update(|cx| bindings(cx)))].concat()
    );
    cx.update(keymap::init);
    assert_eq!(
        listed(cx),
        [kit.clone(), debug(&cx.update(|cx| bindings(cx)))].concat(),
        "a second init binds once"
    );
    cx.update(|cx| keymap::apply(&sidebar_on_backslash(), cx));
    assert_eq!(
        listed(cx),
        [
            kit,
            debug(&cx.update(|cx| bindings_with(&sidebar_on_backslash(), cx)))
        ]
        .concat()
    );
    assert!(
        has_filter_guard(cx),
        "the fixed filter guard survives apply"
    );
}

#[gpui_kit::test]
fn the_kit_inspector_chord_is_dropped(cx: &mut TestAppContext) {
    let col = two_requests();
    let _s = started(cx, &col);
    let bound = cx.update(|cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .any(|b| b.action().name() == INSPECTOR)
    });
    assert!(!bound, "the inspector chord is not bound");
    assert!(
        cx.update(|cx| keymap::kit_bindings(cx))
            .iter()
            .all(|b| b.action().name() != INSPECTOR)
    );
}

#[gpui_kit::test]
fn the_import_chord_works_from_the_url_field(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(s.win, &key("shift-i"));
    cx.run_until_parked();
    assert!(
        present(cx, s.win, "import-link-dialog"),
        "Import collection opens its dialog"
    );
    cx.update_window(s.win, |_, w, cx| w.click("import-link-browse", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths(), "Browse prompts for a file");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn an_edit_rebinds_at_once_and_the_old_key_stops(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    edit_keys(cx, |o| *o = sidebar_on_backslash());
    press_in_shell(cx, &s, &key("b"));
    assert!(sidebar_shown(cx, &s), "the old key does nothing");
    press_in_shell(cx, &s, &key("\\"));
    assert!(!sidebar_shown(cx, &s));
    assert_eq!(keymap_overrides(cx), sidebar_on_backslash());
}

#[gpui_kit::test]
fn the_palette_shows_the_new_key(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    edit_keys(cx, |o| *o = sidebar_on_backslash());
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("toggle sidebar", cx))
        .unwrap();
    frame(cx, s.win);
    let ix = apinata_app::palette::palette_items()
        .iter()
        .position(|e| e.label == "Toggle sidebar")
        .unwrap();
    let label = cx
        .update_window(s.win, |_, w, _| {
            w.within("command-palette")
                .find(("palette-kbd", ix))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label, Some(secondary("\\")));
}

#[gpui_kit::test]
fn send_still_beats_the_url_inputs_enter_after_an_edit(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    edit_keys(cx, |o| *o = sidebar_on_backslash());
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    cx.simulate_keystrokes(s.win, &key("enter"));
    wait_real(cx, |cx| !served(cx, &server).is_empty());
    assert_eq!(served(cx, &server), ["/ok"]);
}

#[gpui_kit::test]
fn a_saved_keymap_applies_at_start_and_quit_keeps_what_it_cannot_use(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started_with(cx, &col, |paths| {
        let text = r#"{"view.toggle_sidebar":["secondary-\\"],"old.command":["secondary-q"],"tabs.close":["secondary-shift-["]}"#;
        std::fs::write(&paths.keymap, text).unwrap();
    });
    assert_eq!(
        notes(cx, &s),
        2,
        "one for the unknown id, one for the invalid entry"
    );
    press_in_shell(cx, &s, &key("\\"));
    assert!(!sidebar_shown(cx, &s));
    open_tab(cx, &s.ws, &s.root, "a.yml");
    press_in_shell(cx, &s, &key("w"));
    assert!(
        tab_names(cx, &s.ws).is_empty(),
        "the invalid entry falls back to its default"
    );
    edit_keys(cx, |o| {
        o.insert("view.toggle_layout".into(), vec!["secondary-l".into()]);
    });
    cx.quit();
    let text = std::fs::read_to_string(Paths::in_dir(s.files.path()).keymap).unwrap();
    let saved: Overrides = serde_json::from_str(&text).unwrap();
    assert_eq!(
        saved,
        overrides(&[
            ("old.command", &["secondary-q"]),
            ("tabs.close", &["secondary-shift-["]),
            ("view.toggle_layout", &["secondary-l"]),
            ("view.toggle_sidebar", &["secondary-\\"]),
        ])
    );
}

fn starts_with_a_broken_file(cx: &mut TestAppContext, text: &'static str) {
    let col = two_requests();
    let s = started_with(cx, &col, |paths| {
        std::fs::write(&paths.keymap, text).unwrap()
    });
    assert!(s.files.path().join("keymap.json.bad").exists());
    assert_eq!(notes(cx, &s), 1);
    assert!(keymap_overrides(cx).is_empty());
    press_in_shell(cx, &s, &key("b"));
    assert!(!sidebar_shown(cx, &s), "the defaults apply");
}

#[gpui_kit::test]
fn invalid_json_is_set_aside(cx: &mut TestAppContext) {
    starts_with_a_broken_file(cx, "{not json");
}

#[gpui_kit::test]
fn a_wrong_typed_value_is_set_aside(cx: &mut TestAppContext) {
    starts_with_a_broken_file(cx, r#"{"tabs.close":"secondary-w"}"#);
}

#[gpui_kit::test]
fn a_failed_keymap_save_is_reported(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    // A non-empty directory where keymap.json belongs makes the write fail.
    let path = Paths::in_dir(s.files.path()).keymap;
    std::fs::create_dir_all(path.join("blocker")).unwrap();
    edit_keys(cx, |o| *o = sidebar_on_backslash());
    let keymap = cx.update(|cx| cx.global::<apinata_app::app::AppState>().keymap.clone());
    let write = keymap.update(cx, |p, cx| p.flush(cx));
    cx.run_until_parked();
    drop(write);
    assert_eq!(notes(cx, &s), 1);
}
