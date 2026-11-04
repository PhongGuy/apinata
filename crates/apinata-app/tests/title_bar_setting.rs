//! The Linux title bar preference: stored in the settings, offered in Preferences, and
//! turned into the window's decoration request.

use crate::support;

use apinata_app::app::window_decorations;
use apinata_app::settings::{LinuxTitleBar, Settings};
use gpui_kit::WindowDecorations;

#[test]
fn the_setting_defaults_to_client_and_round_trips() {
    assert_eq!(Settings::default().linux_title_bar, LinuxTitleBar::Client);
    let old: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
    assert_eq!(old.linux_title_bar, LinuxTitleBar::Client);
    let system = Settings {
        linux_title_bar: LinuxTitleBar::System,
        ..Settings::default()
    };
    let json = serde_json::to_value(&system).unwrap();
    assert_eq!(json["linux_title_bar"], "system");
    assert_eq!(serde_json::from_value::<Settings>(json).unwrap(), system);
}

#[test]
fn the_choice_maps_to_a_request_on_linux_only() {
    let got = (
        window_decorations(LinuxTitleBar::Client),
        window_decorations(LinuxTitleBar::System),
    );
    if cfg!(target_os = "linux") {
        assert_eq!(
            got,
            (
                Some(WindowDecorations::Client),
                Some(WindowDecorations::Server)
            )
        );
    } else {
        assert_eq!(got, (None, None));
    }
}

#[cfg(target_os = "linux")]
#[gpui_kit::test]
fn preferences_switch_the_title_bar_and_the_shell_requests_it(cx: &mut gpui_kit::TestAppContext) {
    // Linux-only imports live here so other platforms' clippy sees no unused import.
    use apinata_app::app::AppState;
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    use support::*;
    let col = two_requests();
    let s = started(cx, &col);
    assert_eq!(
        s.shell.read_with(cx, |v, _| v.requested_title_bar()),
        LinuxTitleBar::Client
    );
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar-mode").click(1usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    let saved = cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .linux_title_bar
    });
    assert_eq!(saved, LinuxTitleBar::System);
    assert_eq!(
        s.shell.read_with(cx, |v, _| v.requested_title_bar()),
        LinuxTitleBar::System
    );
}
