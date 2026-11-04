//! Dialogs open below the title bar and end above the status bar at every zoom.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::confirm::{Ask, Choice, ask};
use apinata_app::dialog_frame::{room, top, view_height};
use apinata_app::settings::Settings;
use apinata_app::style::{SPACE_4, STATUS_BAR_HEIGHT, TITLE_BAR_HEIGHT, at_zoom};
use apinata_app::zoom::ui_font_size;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Edges, ElementId, Focusable as _, Pixels, TestAppContext, px, size,
};
use support::*;

fn set(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let s = cx.global::<AppState>().settings.clone();
        s.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

fn bounds(cx: &mut TestAppContext, s: &Started, id: impl Into<ElementId>) -> Bounds<Pixels> {
    frame(cx, s.win);
    let id = id.into();
    cx.update_window(s.win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

fn close_dialogs(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.close_all_dialogs(cx))
        .unwrap();
    frame(cx, s.win);
}

fn assert_in_content(
    cx: &mut TestAppContext,
    s: &Started,
    z: f32,
    first: &'static str,
    last: &'static str,
) {
    let head = bounds(cx, s, first);
    let tail = bounds(cx, s, last);
    let status = bounds(cx, s, "status-bar");
    assert!(
        head.top() >= at_zoom(TITLE_BAR_HEIGHT, z),
        "{z}: {first} {head:?} under the title bar"
    );
    assert!(
        tail.bottom() <= status.top(),
        "{z}: {last} {tail:?} over the status bar {status:?}"
    );
}

#[test]
fn top_clears_the_title_bar_and_room_ends_above_the_status_bar() {
    assert_eq!(top(px(800.), px(13.)), px(80.));
    assert_eq!(
        top(px(800.), px(26.)),
        px(104.),
        "a 72 px title bar plus a 32 px gap is lower"
    );
    assert_eq!(room(px(800.), px(13.)), px(680.));
    assert_eq!(room(px(800.), px(26.)), px(616.));
}

#[test]
fn room_never_goes_negative() {
    assert_eq!(
        room(px(100.), px(26.)),
        px(0.),
        "a window shorter than the bars leaves no room"
    );
}

#[test]
fn view_height_drops_the_window_border_padding() {
    assert_eq!(view_height(px(800.), Edges::all(px(20.))), px(760.));
    assert_eq!(view_height(px(800.), Edges::all(px(0.))), px(800.));
}

#[gpui_kit::test]
fn preferences_stays_in_the_content_area_at_every_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for z in [0.5, 1.0, 1.2, 1.5, 2.0] {
        set(cx, |p| p.zoom = z);
        cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
            .unwrap();
        assert_in_content(cx, &s, z, "prefs-title", "preferences");
        close_dialogs(cx, &s);
    }
}

#[gpui_kit::test]
fn preferences_keeps_its_height_while_filtering(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    let full = bounds(cx, &s, "preferences").size.height;
    for q in ["tls", "no setting is called this"] {
        replace_input(cx, s.win, None, "prefs-search", q);
        let h = bounds(cx, &s, "preferences").size.height;
        assert_eq!(h, full, "height with {q:?}");
    }
}

#[gpui_kit::test]
fn the_confirm_dialog_stays_in_the_content_area(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for z in [0.5, 2.0] {
        set(cx, |p| p.zoom = z);
        let spec = Ask {
            title: "Delete \"a\"?".into(),
            detail: Some("This cannot be undone.".into()),
            choices: vec![
                Choice {
                    id: "confirm-cancel",
                    label: "Cancel".into(),
                    danger: false,
                },
                Choice {
                    id: "confirm-delete",
                    label: "Delete".into(),
                    danger: true,
                },
            ],
            default: 1,
            cancel: 0,
        };
        let _rx = cx
            .update_window(s.win, |_, w, cx| ask(spec, w, cx))
            .unwrap();
        assert_in_content(cx, &s, z, "confirm-dialog", "confirm-dialog");
        close_dialogs(cx, &s);
    }
}

#[gpui_kit::test]
fn preferences_follows_a_zoom_change_while_open(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    set(cx, |p| p.zoom = 2.0);
    assert_in_content(cx, &s, 2.0, "prefs-title", "preferences");
}

fn confirm_spec() -> Ask {
    Ask {
        title: "Delete \"a\"?".into(),
        detail: Some("This cannot be undone.".into()),
        choices: vec![Choice {
            id: "confirm-cancel",
            label: "Cancel".into(),
            danger: false,
        }],
        default: 0,
        cancel: 0,
    }
}

#[gpui_kit::test]
fn a_dialog_without_its_own_width_follows_the_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let width = |cx: &mut TestAppContext, z: f32| {
        set(cx, |p| p.zoom = z);
        let _rx = cx
            .update_window(s.win, |_, w, cx| ask(confirm_spec(), w, cx))
            .unwrap();
        let w = bounds(cx, &s, "confirm-dialog").size.width;
        close_dialogs(cx, &s);
        w
    };
    let one = width(cx, 1.0);
    let two = width(cx, 2.0);
    let ratio = f32::from(two) / f32::from(one);
    assert!((1.9..=2.1).contains(&ratio), "{one:?} -> {two:?}");
}

// Opens the Save dialog for a new draft at zoom `z`; answers the name and folder boxes.
fn save_form_inputs(
    cx: &mut TestAppContext,
    s: &Started,
    z: f32,
) -> (Bounds<Pixels>, Bounds<Pixels>) {
    set(cx, |p| p.zoom = z);
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    cx.update_window(s.win, |_, w, cx| handle.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    let name = bounds(cx, s, "save-draft-name");
    // The picker's drawn box is its inner input.
    let folder = cx
        .update_window(s.win, |_, w, _| {
            w.within("save-draft-folder").find("input").bounds()
        })
        .unwrap();
    close_dialogs(cx, s);
    (name, folder)
}

#[gpui_kit::test]
fn the_save_form_scales_its_inputs_at_200_percent(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let (one, _) = save_form_inputs(cx, &s, 1.0);
    let (name, folder) = save_form_inputs(cx, &s, 2.0);
    assert!(
        name.size.width >= one.size.width * 2. - px(4.),
        "{name:?} at 200% vs {one:?} at 100%"
    );
    assert_eq!(folder.size.width, name.size.width, "{folder:?} vs {name:?}");
}

#[gpui_kit::test]
fn the_palette_stays_in_the_content_area_of_a_short_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.simulate_window_resize(s.win, size(px(1280.), px(800.)));
    for z in [1.0, 2.0] {
        set(cx, |p| p.zoom = z);
        cx.update_window(s.win, |_, w, cx| {
            let fallback = w.focused(cx).unwrap_or_else(|| cx.focus_handle());
            apinata_app::palette::open(fallback, w, cx);
        })
        .unwrap();
        assert_in_content(cx, &s, z, "command-palette", "command-palette");
        close_dialogs(cx, &s);
    }
}

#[gpui_kit::test]
fn preferences_at_half_zoom_leaves_little_empty_room(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    set(cx, |p| p.zoom = 0.5);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    let body = bounds(cx, &s, "preferences");
    let last = bounds(cx, &s, "prefs-keyboard");
    let empty = body.bottom() - last.bottom();
    assert!(
        empty < body.size.height * 0.25,
        "{empty:?} empty below the last section in {body:?}"
    );
    close_dialogs(cx, &s);
}

#[gpui_kit::test]
fn the_save_name_field_fills_its_column(cx: &mut TestAppContext) {
    use apinata_app::style::NAME_INPUT_WIDTH;
    let col = two_requests();
    let s = started(cx, &col);
    let (name, folder) = save_form_inputs(cx, &s, 1.0);
    assert_eq!(name.size.width, folder.size.width, "{name:?} vs {folder:?}");
    assert!(
        name.size.width >= at_zoom(NAME_INPUT_WIDTH, 1.0),
        "{name:?} narrower than a name input"
    );
}

#[gpui_kit::test]
fn the_save_dialog_puts_save_right_of_cancel(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let handle = s.shell.read_with(cx, |v, cx| v.focus_handle(cx));
    cx.update_window(s.win, |_, w, cx| handle.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("n"));
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("s"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let footer = w.within("save-draft-dialog").find("dialog-footer").bounds();
        let cancel = w.find("save-draft-cancel").bounds();
        let save = w.find("save-draft-save").bounds();
        assert!(cancel.right() < save.left());
        assert_eq!(save.right(), footer.right());
    })
    .unwrap();
}

// The gap `room` keeps under a dialog, read back from the public frame functions.
fn gap_at(viewport: Pixels, rem: Pixels) -> Pixels {
    viewport - room(viewport, rem) - top(viewport, rem) - STATUS_BAR_HEIGHT.to_pixels(rem)
}

fn assert_gap(viewport: f32, rem: Pixels, quarter: bool) {
    let full = SPACE_4.to_pixels(rem);
    let want = if quarter { full / 4. } else { full };
    let got = gap_at(px(viewport), rem);
    assert!(
        (got - want).abs() < px(0.05),
        "{viewport} px at rem {rem:?}: gap {got:?}, wanted {want:?}"
    );
}

#[test]
fn the_gap_is_a_quarter_just_under_24_rems_and_full_at_24_rems() {
    // 200% in the issue's rem 32, and the app's own 100% and 200%.
    for rem in [32., 13., f32::from(ui_font_size(2.0))] {
        let edge = rem * 24.;
        assert_gap(edge - 1., px(rem), true);
        assert_gap(edge, px(rem), false);
    }
}

#[test]
fn a_700_px_view_has_the_small_gap_at_rem_32_and_the_full_gap_at_200_percent() {
    assert_gap(700., px(32.), true);
    // The app's 200% rem is 26, and 24 of those are 624 px.
    assert_gap(700., ui_font_size(2.0), false);
}
