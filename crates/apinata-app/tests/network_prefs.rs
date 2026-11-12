//! Preferences > Network: the global proxy, its keychain password and the CA file.

use crate::support;

use apinata_app::app::{AppState, VolatileSecrets};
use apinata_app::executor::proxy_password_key;
use apinata_app::preferences::{PAC_HINT, PROXY_NOTE, parse_proxy_port};
use apinata_app::settings::{ProxyMode, Settings};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::path::PathBuf;
use support::*;

fn settings(cx: &mut TestAppContext) -> Settings {
    cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().clone())
}

// Wheels the dialog until `id` is on screen, without clicking it.
fn reveal(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    for _ in 0..40 {
        let visible = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                // Clear of the bottom fade too: a click on a field under it would miss.
                let clear = w.find("preferences-rows").bounds().bottom()
                    - w.try_find("prefs-scroll-fade")
                        .map_or(gpui_kit::px(0.), |f| f.bounds().size.height);
                w.try_find(id)
                    .is_some_and(|e| e.visible() && e.bounds().bottom() <= clear)
            })
            .unwrap();
        if visible {
            return;
        }
        cx.update_window(s.win, |_, w, cx| {
            let down = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                gpui_kit::px(0.),
                gpui_kit::px(-200.),
            ));
            w.scroll("preferences", down, cx)
        })
        .unwrap();
    }
    panic!("{id} never came into view");
}

fn enter(cx: &mut TestAppContext, s: &Started, id: &'static str, text: &str) {
    reveal(cx, s, id);
    cx.update_window(s.win, |_, w, cx| {
        w.click(id, cx);
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        if !text.is_empty() {
            w.input(text, cx);
        }
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn choose(cx: &mut TestAppContext, s: &Started, group: &'static str, ix: usize) {
    reveal(cx, s, group);
    cx.update_window(s.win, |_, w, cx| w.within(group).click(ix, cx))
        .unwrap();
    frame(cx, s.win);
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    reveal(cx, s, id);
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

#[test]
fn ports_are_blank_or_1_to_65535() {
    assert_eq!(parse_proxy_port(""), Ok(String::new()));
    assert_eq!(parse_proxy_port(" 3128 "), Ok("3128".into()));
    assert_eq!(parse_proxy_port("03128"), Ok("3128".into()));
    for bad in ["0", "65536", "x", "-1", "+3128", "00"] {
        let err = parse_proxy_port(bad).unwrap_err();
        assert!(err.contains("1 to 65535"), "{bad:?}: {err}");
    }
}

#[gpui_kit::test]
fn manual_proxy_rows_edit_the_settings(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(!present(cx, s.win, "pref-proxy-host"));
    choose(cx, &s, "global-proxy-mode", 2);
    assert_eq!(settings(cx).proxy.mode, ProxyMode::Manual);
    choose(cx, &s, "global-proxy-protocol", 3);
    enter(cx, &s, "global-proxy-host", "proxy.corp.example");
    enter(cx, &s, "global-proxy-port", "3128");
    enter(cx, &s, "global-proxy-username", "u");
    enter(cx, &s, "global-proxy-bypass", "localhost, .internal");
    let p = settings(cx).proxy;
    assert_eq!(
        (
            p.protocol.as_str(),
            p.host.as_str(),
            p.port.as_str(),
            p.username.as_str(),
            p.bypass.as_str()
        ),
        (
            "socks5",
            "proxy.corp.example",
            "3128",
            "u",
            "localhost, .internal"
        )
    );
    enter(cx, &s, "global-proxy-port", "99999");
    assert!(present(cx, s.win, "global-proxy-port-error"));
    assert_eq!(settings(cx).proxy.port, "3128");
    choose(cx, &s, "global-proxy-mode", 0);
    assert_eq!(settings(cx).proxy.mode, ProxyMode::Off);
    assert!(!present(cx, s.win, "pref-proxy-host"));
}

#[gpui_kit::test]
fn the_password_goes_to_the_secret_store_only(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    choose(cx, &s, "global-proxy-mode", 2);
    enter(cx, &s, "global-proxy-password", "hunter2");
    let store = cx.update(|cx| cx.global::<AppState>().secrets.clone());
    assert_eq!(
        store.get(&proxy_password_key()).unwrap().as_deref(),
        Some("hunter2")
    );
    let json = serde_json::to_string(&settings(cx)).unwrap();
    assert!(!json.contains("hunter2"), "{json}");
    assert!(!present(cx, s.win, "global-proxy-password-volatile"));
    click(cx, &s, "global-proxy-password-forget");
    cx.run_until_parked();
    assert_eq!(store.get(&proxy_password_key()).unwrap(), None);
}

#[gpui_kit::test]
fn without_a_keychain_the_password_row_says_so(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update(|cx| cx.set_global(VolatileSecrets));
    open_prefs(cx, &s);
    choose(cx, &s, "global-proxy-mode", 2);
    reveal(cx, &s, "global-proxy-password");
    assert!(present(cx, s.win, "global-proxy-password-volatile"));
}

#[gpui_kit::test]
fn the_ca_file_is_chosen_typed_and_cleared(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(present(cx, s.win, "prefs-proxy-note"));
    assert_eq!(
        PROXY_NOTE,
        "System uses the proxy environment variables (and the system settings on macOS and \
         Windows); a PAC file set in the system settings is not used. Choose PAC to use one."
    );
    click(cx, &s, "ca-file-choose");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|o| {
        assert!(o.files && !o.directories && !o.multiple);
        Some(vec![PathBuf::from("/certs/ca.pem")])
    });
    cx.run_until_parked();
    assert_eq!(settings(cx).ca_file, Some(PathBuf::from("/certs/ca.pem")));
    enter(cx, &s, "ca-file", "");
    assert_eq!(settings(cx).ca_file, None);
    click(cx, &s, "keep-default-cas");
    assert!(!settings(cx).keep_default_cas);
}

// Replaces the input's text and leaves with `then`, a key such as `tab`, or not at all.
fn type_then(cx: &mut TestAppContext, s: &Started, id: &'static str, text: &str, then: &str) {
    reveal(cx, s, id);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    // Blur is seen by comparing drawn frames, so the click's focus gets a frame of its own.
    click(cx, s, id);
    cx.update_window(s.win, |_, w, cx| {
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        w.input(text, cx);
        if !then.is_empty() {
            w.press(then, cx);
        }
    })
    .unwrap();
    // A focus change reaches blur listeners when the next frame is drawn.
    frame(cx, s.win);
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    use gpui_kit::component::WindowExt as _;
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn saved_file(cx: &mut TestAppContext, s: &Started) -> String {
    let task = cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.flush(cx))
    });
    cx.run_until_parked();
    drop(task);
    let path = apinata_app::persist::Paths::in_dir(s.files.path()).settings;
    std::fs::read_to_string(path).unwrap_or_default()
}

#[gpui_kit::test]
fn typed_proxy_fields_save_on_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    choose(cx, &s, "global-proxy-mode", 2);
    type_then(cx, &s, "global-proxy-host", "proxy.local", "tab");
    assert_eq!(settings(cx).proxy.host, "proxy.local");
    type_then(cx, &s, "global-proxy-port", "3128", "tab");
    assert_eq!(settings(cx).proxy.port, "3128");
    type_then(cx, &s, "global-proxy-username", "u", "tab");
    let p = settings(cx).proxy;
    assert_eq!(
        (p.host.as_str(), p.port.as_str(), p.username.as_str()),
        ("proxy.local", "3128", "u")
    );
}

#[gpui_kit::test]
fn typed_fields_save_when_the_dialog_closes(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    choose(cx, &s, "global-proxy-mode", 2);
    // Escape closes the dialog with the host still focused and unconfirmed.
    type_then(cx, &s, "global-proxy-host", "proxy.local", "escape");
    assert!(!dialog_open(cx, &s));
    assert_eq!(settings(cx).proxy.host, "proxy.local");
    assert!(saved_file(cx, &s).contains("proxy.local"));
    let view = open_prefs(cx, &s);
    assert_eq!(
        view.read_with(cx, |v, cx| v.input_text("global-proxy-host", cx)),
        Some("proxy.local".into())
    );
    // The close button is the other way out.
    type_then(cx, &s, "global-proxy-port", "3128", "");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("close", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!dialog_open(cx, &s));
    assert_eq!(settings(cx).proxy.port, "3128");
}

#[gpui_kit::test]
fn the_password_row_is_labelled_plainly(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = open_prefs(cx, &s);
    choose(cx, &s, "global-proxy-mode", 2);
    // The search matches row labels, so it shows which label the row carries.
    let search = view.read_with(cx, |v, _| v.search().clone());
    for (query, shown) in [("enter saves", false), ("password", true)] {
        cx.update_window(s.win, |_, w, cx| {
            search.update(cx, |st, cx| st.set_value(query, w, cx))
        })
        .unwrap();
        frame(cx, s.win);
        assert_eq!(present(cx, s.win, "pref-proxy-password"), shown, "{query}");
    }
}

#[gpui_kit::test]
fn the_password_commits_to_the_store_on_blur_and_close(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    choose(cx, &s, "global-proxy-mode", 2);
    let store = cx.update(|cx| cx.global::<AppState>().secrets.clone());
    type_then(cx, &s, "global-proxy-password", "first-pw", "tab");
    assert_eq!(
        store.get(&proxy_password_key()).unwrap().as_deref(),
        Some("first-pw")
    );
    type_then(cx, &s, "global-proxy-password", "second-pw", "");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("close", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        store.get(&proxy_password_key()).unwrap().as_deref(),
        Some("second-pw")
    );
    let file = saved_file(cx, &s);
    assert!(
        !file.contains("first-pw") && !file.contains("second-pw"),
        "{file}"
    );
}

#[gpui_kit::test]
fn pac_rows_edit_the_source_and_refetch(cx: &mut TestAppContext) {
    assert_eq!(
        PAC_HINT,
        "The bypass list and proxy login do not apply to PAC. Requests that cannot use the PAC \
         file connect directly and show a warning."
    );
    let col = two_requests();
    let s = started(cx, &col);
    let view = open_prefs(cx, &s);
    assert!(!present(cx, s.win, "global-proxy-pac"));
    choose(cx, &s, "global-proxy-mode", 3);
    assert_eq!(settings(cx).proxy.mode, ProxyMode::Pac);
    assert!(!present(cx, s.win, "pref-proxy-host"));
    assert!(!present(cx, s.win, "global-proxy-bypass"));
    reveal(cx, &s, "prefs-proxy-pac-hint");
    enter(
        cx,
        &s,
        "global-proxy-pac",
        " https://example.com/proxy.pac ",
    );
    assert_eq!(
        settings(cx).proxy.pac_source,
        "https://example.com/proxy.pac"
    );
    enter(cx, &s, "global-proxy-pac", "ftp://example.com/proxy.pac");
    assert!(present(cx, s.win, "global-proxy-pac-error"));
    assert_eq!(
        settings(cx).proxy.pac_source,
        "https://example.com/proxy.pac"
    );
    enter(cx, &s, "global-proxy-pac", "proxy.pac");
    assert!(present(cx, s.win, "global-proxy-pac-error"));
    enter(
        cx,
        &s,
        "global-proxy-pac",
        "https://u:pw-SEKRIT@example.com/proxy.pac",
    );
    assert!(present(cx, s.win, "global-proxy-pac-error"));
    let json = serde_json::to_string(&settings(cx)).unwrap();
    assert!(!json.contains("SEKRIT"), "{json}");
    assert!(
        view.read_with(cx, |v, _| v.unsaved())
            .iter()
            .any(|w| w == "Not saved: PAC file: the PAC URL must not contain credentials"),
    );
    enter(cx, &s, "global-proxy-pac", "");
    assert!(!present(cx, s.win, "global-proxy-pac-error"));
    assert_eq!(settings(cx).proxy.pac_source, "");
    click(cx, &s, "global-proxy-pac-refetch");
    assert_eq!(
        view.read_with(cx, |v, _| v.status()).map(|s| s.to_string()),
        Some("The PAC file will be fetched again on the next request.".to_owned())
    );
    choose(cx, &s, "global-proxy-mode", 1);
    assert!(!present(cx, s.win, "global-proxy-pac"));
}

#[gpui_kit::test]
fn the_pac_hint_wraps_as_a_few_lines_under_the_source(cx: &mut TestAppContext) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    let _view = open_prefs(cx, &s);
    choose(cx, &s, "global-proxy-mode", 3);
    reveal(cx, &s, "prefs-proxy-pac-hint");
    let (hint, input) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("prefs-proxy-pac-hint").bounds(),
                w.find("global-proxy-pac").bounds(),
            )
        })
        .unwrap();
    // A hint squeezed to a sliver wraps a letter per line and grows hundreds of pixels tall.
    assert!(
        hint.size.width >= input.size.width,
        "hint {hint:?} input {input:?}"
    );
    assert!(
        hint.size.height < input.size.height * 4.0,
        "hint {hint:?} input {input:?}"
    );
}

fn drawn(
    cx: &mut TestAppContext,
    s: &Started,
    id: &'static str,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

// The help on what System reads sits right under the Proxy control in every mode, above the
// rows that follow it, rather than after the cookie settings.
#[gpui_kit::test]
fn the_proxy_help_sits_under_the_proxy_control(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    for (mode, next) in [
        (1, "pref-ca-file"),
        (2, "pref-proxy-protocol"),
        (3, "pref-proxy-pac"),
    ] {
        choose(cx, &s, "global-proxy-mode", mode);
        reveal(cx, &s, "prefs-proxy-note");
        let control = drawn(cx, &s, "pref-proxy-mode");
        let note = drawn(cx, &s, "prefs-proxy-note");
        let next = drawn(cx, &s, next);
        assert!(
            note.top() >= control.bottom() - gpui_kit::px(1.)
                && note.top() <= control.bottom() + gpui_kit::px(16.),
            "mode {mode}: the help {note:?} right under the control {control:?}"
        );
        assert!(
            note.bottom() <= next.top() + gpui_kit::px(1.),
            "mode {mode}: the help {note:?} above the next row {next:?}"
        );
        assert!(
            note.bottom() <= drawn(cx, &s, "pref-store-cookies").top(),
            "mode {mode}: the help comes before the cookie settings"
        );
    }
}

// The manual proxy fields and the PAC source say, after the last of them, that they wait for
// a restart, as the Proxy control does.
#[gpui_kit::test]
fn the_proxy_fields_say_they_take_effect_after_restart(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(
        !present(cx, s.win, "prefs-proxy-restart"),
        "Off has no fields"
    );
    for (mode, last) in [(2, "pref-proxy-bypass"), (3, "pref-proxy-pac")] {
        choose(cx, &s, "global-proxy-mode", mode);
        reveal(cx, &s, "prefs-proxy-restart");
        let note = drawn(cx, &s, "prefs-proxy-restart");
        let last = drawn(cx, &s, last);
        assert!(
            note.top() >= last.bottom() - gpui_kit::px(1.)
                && note.bottom() <= drawn(cx, &s, "pref-ca-file").top() + gpui_kit::px(1.),
            "mode {mode}: the note {note:?} after the last proxy row {last:?}"
        );
        let text = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                w.find("prefs-proxy-restart").label().map(str::to_owned)
            })
            .unwrap();
        assert_eq!(
            text.as_deref(),
            Some("The proxy settings above take effect after restart"),
            "mode {mode}"
        );
    }
}
