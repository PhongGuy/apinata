//! The Import dialog's Link field fills its row and takes focus on a click, in both modes.

use crate::import_update::{Rig, rig};
use crate::support::*;

use apinata_app::import_link::{self, ImportLinkDialog, Mode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Entity, Pixels, TestAppContext};

/// Opens the dialog with an empty link, in `mode`, with focus moved off the field.
fn open(cx: &mut TestAppContext, r: &Rig, mode: Mode) -> Entity<ImportLinkDialog> {
    r.open_v1(cx);
    let d = r.dialog(cx, "");
    if mode == Mode::New {
        r.click(cx, "import-mode-new");
    }
    cx.update_window(r.win, |_, w, cx| w.blur(cx)).unwrap();
    frame(cx, r.win);
    d
}

fn bounds(cx: &mut TestAppContext, r: &Rig, id: &'static str) -> Bounds<Pixels> {
    frame(cx, r.win);
    cx.update_window(r.win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

fn fills_the_dialog(cx: &mut TestAppContext, mode: Mode) {
    let r = rig(cx);
    open(cx, &r, mode);
    let field = bounds(cx, &r, "import-link").size.width;
    let dialog = bounds(cx, &r, "import-link-dialog").size.width;
    assert!(
        field > dialog * 0.4,
        "{mode:?}: the field is {field:?} wide in a {dialog:?} dialog"
    );
}

fn click_then_type(cx: &mut TestAppContext, mode: Mode) {
    let r = rig(cx);
    let d = open(cx, &r, mode);
    click_with(cx, r.win, "import-link", gpui_kit::Modifiers::default());
    cx.simulate_input(r.win, "https://x.test/a");
    assert_eq!(d.read_with(cx, |d, cx| d.link(cx)), "https://x.test/a");
    assert!(cx.read(import_link::current).is_some());
}

#[gpui_kit::test]
fn the_link_field_fills_the_row_in_new_mode(cx: &mut TestAppContext) {
    fills_the_dialog(cx, Mode::New);
}

#[gpui_kit::test]
fn the_link_field_fills_the_row_in_update_mode(cx: &mut TestAppContext) {
    fills_the_dialog(cx, Mode::Update);
}

#[gpui_kit::test]
fn a_click_focuses_the_link_field_in_new_mode(cx: &mut TestAppContext) {
    click_then_type(cx, Mode::New);
}

#[gpui_kit::test]
fn a_click_focuses_the_link_field_in_update_mode(cx: &mut TestAppContext) {
    click_then_type(cx, Mode::Update);
}
