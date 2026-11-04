//! Focus modality: the ring is the accent only after a key moves focus; a pointer press
//! puts it back to the input border.

use crate::support;

use apinata_app::focus_mode::{self, Modality};
use apinata_app::theme;
use gpui_kit::component::{ActiveTheme as _, ThemeMode, WindowExt as _};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Hsla, TestAppContext};
use support::*;

// What the theme paints focus with: (ring, input border, accent, outer ring on, swaps).
fn ring(cx: &mut TestAppContext) -> (Hsla, Hsla, Hsla, bool, usize) {
    cx.update(|cx| {
        let t = cx.theme();
        (
            t.ring,
            t.input,
            t.primary,
            t.focus_ring,
            focus_mode::swaps(cx),
        )
    })
}

fn shell_on_a(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    (col, s)
}

fn click_url(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    frame(cx, s.win);
}

fn type_text(cx: &mut TestAppContext, s: &Started, text: &str) {
    cx.update_window(s.win, |_, w, cx| w.input(text, cx))
        .unwrap();
    frame(cx, s.win);
}

// Opens Preferences and presses Tab, which moves focus onto its first control.
fn tab_in_preferences(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    let before = cx.update_window(s.win, |_, w, cx| w.focused(cx)).unwrap();
    cx.simulate_keystrokes(s.win, "tab");
    frame(cx, s.win);
    let after = cx.update_window(s.win, |_, w, cx| w.focused(cx)).unwrap();
    assert!(before != after, "the case needs Tab to move focus");
}

#[gpui_kit::test]
fn the_ring_turns_accent_only_when_a_key_moves_focus(cx: &mut TestAppContext) {
    let (_col, s) = shell_on_a(cx);
    let (r, input, accent, on, swaps) = ring(cx);
    assert_ne!(input, accent, "the case needs distinct colours");
    assert_eq!(
        (r, on, swaps),
        (input, false, 0),
        "the app starts in pointer modality"
    );
    click_url(cx, &s);
    type_text(cx, &s, "abc");
    let (r, input, _, on, swaps) = ring(cx);
    assert_eq!(
        (r, on, swaps),
        (input, false, 0),
        "typing into a clicked field keeps the plain border"
    );
    tab_in_preferences(cx, &s);
    let (r, _, accent, on, swaps) = ring(cx);
    assert_eq!((r, on, swaps), (accent, true, 1), "Tab moved focus");
    assert!(cx.update(|cx| focus_mode::is_keyboard(cx)));
    cx.update_window(s.win, |_, w, cx| w.close_dialog(cx))
        .unwrap();
    frame(cx, s.win);
    click_url(cx, &s);
    let (r, input, _, on, swaps) = ring(cx);
    assert_eq!(
        (r, on, swaps),
        (input, false, 2),
        "a click puts the plain border back"
    );
    click_url(cx, &s);
    type_text(cx, &s, "d");
    assert_eq!(ring(cx).4, 2, "repeated clicks and typing swap nothing");
    assert_eq!(cx.update(|cx| focus_mode::modality(cx)), Modality::Pointer);
}

#[gpui_kit::test]
fn a_theme_mode_change_keeps_the_modality_ring(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| theme::set_mode(ThemeMode::Light, None, cx));
    let (r, input, _, on, _) = ring(cx);
    assert_eq!((r, on), (input, false), "pointer, light");
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    cx.update(|cx| theme::set_mode(ThemeMode::Dark, None, cx));
    let (r, input, accent, on, _) = ring(cx);
    assert_ne!(input, accent);
    assert_eq!((r, on), (accent, true), "keyboard, dark");
    cx.update(theme::install);
    let (r, _, accent, on, swaps) = ring(cx);
    assert_eq!(
        (r, on, swaps),
        (accent, true, 1),
        "install keeps the keyboard ring"
    );
    cx.update(|cx| focus_mode::set(Modality::Keyboard, cx));
    assert_eq!(ring(cx).4, 1, "setting the current modality is no swap");
    cx.update(|cx| focus_mode::set(Modality::Pointer, cx));
    let (r, input, _, on, swaps) = ring(cx);
    assert_eq!((r, on, swaps), (input, false, 2));
}
