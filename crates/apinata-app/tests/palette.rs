//! The command palette: its key, rows with key hints, filtering, and dispatch that reaches
//! the views the actions are bound in.

use crate::support;

use apinata_app::keymap::{ToggleOrientation, ToggleSidebar};
use apinata_app::palette::{index_path, palette_items};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action, AppContext as _, TestAppContext};
use support::*;

fn ix_of(action: &dyn Action) -> usize {
    palette_items()
        .iter()
        .position(|e| e.action.partial_eq(action))
        .expect("listed")
}

fn open_palette(cx: &mut TestAppContext, s: &Started) {
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
}

fn type_query(cx: &mut TestAppContext, s: &Started, text: &str) {
    cx.update_window(s.win, |_, w, cx| w.input(text, cx))
        .unwrap();
    frame(cx, s.win);
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

#[gpui_kit::test]
fn the_key_opens_the_palette(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_palette(cx, &s);
    assert!(dialog_open(cx, &s));
    assert!(present(cx, s.win, "command-palette"));
    let first = index_path(&palette_items(), 0);
    let found = cx
        .update_window(s.win, |_, w, _| {
            w.within("command-palette").try_find(first).is_some()
        })
        .unwrap();
    assert!(found, "the first row renders");
}

#[gpui_kit::test]
fn typing_filters_and_rows_show_their_keys(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_palette(cx, &s);
    type_query(cx, &s, "toggle sidebar");
    let items = palette_items();
    let ix = ix_of(&ToggleSidebar);
    let (row, other) = (
        index_path(&items, ix),
        index_path(&items, ix_of(&ToggleOrientation)),
    );
    cx.update_window(s.win, |_, w, _| {
        let palette = w.within("command-palette");
        assert!(palette.try_find(row).is_some(), "Toggle sidebar matches");
        let hint = palette.find(("palette-kbd", ix));
        assert_eq!(
            hint.label(),
            Some(secondary("b").as_str()),
            "its secondary-b key shows"
        );
        assert!(
            palette.try_find(other).is_none(),
            "Toggle layout is filtered out"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_redo_row_shows_the_shift_chord(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    // Redo is listed only with a tab open.
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    open_palette(cx, &s);
    type_query(cx, &s, "redo edit");
    let ix = ix_of(&apinata_app::keymap::RedoActive);
    cx.update_window(s.win, |_, w, _| {
        let hint = w.within("command-palette").find(("palette-kbd", ix));
        assert_eq!(hint.label(), Some(secondary("shift-z").as_str()));
    })
    .unwrap();
}

use apinata_app::app::AppState;
use apinata_app::settings::Orientation;
use gpui_kit::Focusable as _;

fn orientation(cx: &mut TestAppContext) -> Orientation {
    cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .response_orientation
    })
}

#[gpui_kit::test]
fn confirming_with_enter_reaches_the_shell(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_palette(cx, &s);
    type_query(cx, &s, "toggle sidebar");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s));
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));
}

#[gpui_kit::test]
fn a_clicked_row_runs_from_the_focus_it_was_opened_over(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    frame(cx, s.win);
    let url_focused = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, _| w.find("url").focused())
            .unwrap()
    };
    assert_eq!(url_focused(cx), Some(true));
    open_palette(cx, &s);
    type_query(cx, &s, "toggle layout");
    let row = index_path(&palette_items(), ix_of(&ToggleOrientation));
    cx.update_window(s.win, |_, w, cx| w.within("command-palette").click(row, cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(orientation(cx), Orientation::Horizontal);
    assert_eq!(
        url_focused(cx),
        Some(true),
        "focus is back in the URL input"
    );
}

#[gpui_kit::test]
fn with_nothing_focused_the_shell_takes_the_action(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        let fallback = shell.read(cx).focus_handle(cx);
        w.blur(cx);
        apinata_app::palette::open(fallback, w, cx);
    })
    .unwrap();
    frame(cx, s.win);
    type_query(cx, &s, "toggle sidebar");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));
}

#[gpui_kit::test]
fn an_action_nothing_handles_is_harmless(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    // With a request tab open but no caret in a response, the JSON command has no target.
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    open_palette(cx, &s);
    type_query(cx, &s, "copy json path");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s), "the palette closed");
    assert!(
        s.shell.read_with(cx, |v, _| v.sidebar_visible()),
        "nothing else changed"
    );
}

use apinata_app::app_menu::app_menu_index;
use apinata_app::palette::OpenCommandPalette;

#[gpui_kit::test]
fn the_app_menu_opens_the_palette(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let ix = app_menu_index(&OpenCommandPalette).expect("the menu lists the palette");
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("app-menu", cx)
    })
    .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "command-palette"));
}

#[gpui_kit::test]
fn the_empty_tab_area_opens_the_palette(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let hinted = cx
        .update_window(s.win, |_, w, _| {
            w.within("tab-empty")
                .try_find(("empty-kbd", 3usize))
                .is_some()
        })
        .unwrap();
    assert!(hinted, "secondary-shift-p shows beside Command palette");
    cx.update_window(s.win, |_, w, cx| {
        w.within("tab-empty").click(("empty-action", 3usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert!(present(cx, s.win, "command-palette"));
}

#[gpui_kit::test]
fn the_palette_list_ends_on_a_whole_row(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_palette(cx, &s);
    cx.update_window(s.win, |_, w, _| {
        let bottom = w.find("command-palette").bounds().bottom();
        let items = palette_items();
        let mut visible = 0;
        for ix in 0..items.len() {
            if let Some(row) = w.within("command-palette").try_find(index_path(&items, ix)) {
                let b = row.bounds();
                if b.top() < bottom - gpui_kit::px(1.) {
                    assert!(
                        b.bottom() <= bottom + gpui_kit::px(1.),
                        "row {ix} cut: {b:?} past {bottom:?}"
                    );
                    visible += 1;
                }
            }
        }
        assert!(visible >= 5, "only {visible} rows shown");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_palette_toggles_the_theme(cx: &mut TestAppContext) {
    use apinata_app::settings::ThemeChoice;
    let col = two_requests();
    let s = started(cx, &col);
    let dark = |cx: &mut TestAppContext| {
        cx.update(|cx| gpui_kit::component::Theme::global(cx).mode.is_dark())
    };
    let before = dark(cx);
    open_palette(cx, &s);
    type_query(cx, &s, "toggle theme");
    cx.simulate_keystrokes(s.win, "enter");
    frame(cx, s.win);
    cx.run_until_parked();
    assert!(!dialog_open(cx, &s));
    assert_ne!(dark(cx), before);
    let choice = cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().theme);
    assert_eq!(
        choice,
        if before {
            ThemeChoice::Light
        } else {
            ThemeChoice::Dark
        }
    );
}

#[gpui_kit::test]
fn a_filtered_palette_list_ends_on_a_whole_row(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_palette(cx, &s);
    for query in ["t", "o", "a", "s", "r", "l", "e"] {
        type_query(cx, &s, query);
        cx.update_window(s.win, |_, w, _| {
            let bottom = w.find("command-palette").bounds().bottom();
            let items = palette_items();
            let mut visible = 0;
            for ix in 0..items.len() {
                if let Some(row) = w.within("command-palette").try_find(index_path(&items, ix)) {
                    let b = row.bounds();
                    if b.top() < bottom - gpui_kit::px(1.) {
                        assert!(
                            b.bottom() <= bottom + gpui_kit::px(1.),
                            "query {query:?}: row {ix} cut: {b:?} past {bottom:?}"
                        );
                        visible += 1;
                    }
                }
            }
            assert!(visible >= 1, "query {query:?}: nothing shown");
        })
        .unwrap();
        cx.simulate_keystrokes(s.win, "backspace");
        frame(cx, s.win);
    }
}

// Scrolls the palette list by `dy` px and returns where `row` lands.
fn scrolled(
    cx: &mut TestAppContext,
    s: &Started,
    row: gpui_kit::component::IndexPath,
    dy: f32,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    cx.update_window(s.win, |_, w, cx| {
        w.scroll(
            "command-palette",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(dy))),
            cx,
        );
        w.within("command-palette").find(row).bounds()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_single_match_has_nothing_to_scroll(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_palette(cx, &s);
    type_query(cx, &s, "toggle sidebar");
    let row = index_path(&palette_items(), ix_of(&ToggleSidebar));
    let before = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.within("command-palette").find(row).bounds()
        })
        .unwrap();
    // Nothing is scrolled away above the row, and below it only the kit's half-pixel
    // rounding of the list's edge remains.
    assert_eq!(scrolled(cx, &s, row, 100.), before, "scrolled up");
    let down = scrolled(cx, &s, row, -100.);
    assert!(
        before.top() - down.top() <= gpui_kit::px(0.5),
        "the list has nothing to scroll: {before:?} -> {down:?}"
    );
}

// One Escape closes the palette even with a query typed; the kit's own Cancel would only
// clear the query first.
#[gpui_kit::test]
fn escape_closes_the_palette_with_a_query_typed(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_palette(cx, &s);
    type_query(cx, &s, "toggle");
    assert!(dialog_open(cx, &s));
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    assert!(!dialog_open(cx, &s), "one Escape closed the palette");
    assert!(!present(cx, s.win, "command-palette"));
}

// The palette names the one area a pane command works in, beside its key.
#[gpui_kit::test]
fn a_pane_command_row_names_its_area(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    open_palette(cx, &s);
    type_query(cx, &s, "undo");
    let pane = ix_of(&apinata_app::sidebar::UndoDelete);
    let global = ix_of(&apinata_app::keymap::UndoActive);
    cx.update_window(s.win, |_, w, _| {
        let palette = w.within("command-palette");
        assert!(palette.try_find(("palette-kbd", global)).is_some());
        assert!(palette.try_find(("palette-scope", global)).is_none());
        let scope = palette.find(("palette-scope", pane));
        assert_eq!(scope.label(), Some("Sidebar"));
    })
    .unwrap();
}
