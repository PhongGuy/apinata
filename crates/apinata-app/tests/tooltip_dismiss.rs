//! A tooltip goes once a dialog opens over its control, even while the pointer rests there.

use crate::support;

use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, VisualTestContext, point, px};
use support::*;

const TIP: &str = "env-control-tooltip";

fn rest(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    frame(cx, win);
    frame(cx, win);
}

fn ask(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        apinata_app::confirm::ask(apinata_app::confirm::discard_ask("Discard?"), w, cx);
    })
    .unwrap();
    rest(cx, win);
    assert!(present(cx, win, "confirm-dialog"), "the dialog is open");
}

// The Environments tab of a collection with one environment, its select's tip showing.
fn hovered_select(cx: &mut TestAppContext) -> (Started, tempfile::TempDir) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("environments/dev.yml", "name: dev\n"),
    ]);
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Environments, cx)
    });
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").hover("env-control", cx)
    })
    .unwrap();
    rest(cx, s.win);
    assert!(present(cx, s.win, TIP), "the hovered select shows its tip");
    (s, col)
}

// A shortcut can open a modal while the pointer rests on a control; the modal covers the
// control, so its tip goes too, and stays gone while the pointer moves inside the control.
#[gpui_kit::test]
fn a_dialog_opened_over_a_hovered_control_hides_its_tooltip(cx: &mut TestAppContext) {
    let (s, _col) = hovered_select(cx);
    let at = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("env-control").bounds().center()
        })
        .unwrap();
    ask(cx, s.win);
    assert!(!present(cx, s.win, TIP), "no tip over the dialog");
    VisualTestContext::from_window(s.win, cx).simulate_mouse_move(
        at + point(px(2.), px(0.)),
        None,
        Default::default(),
    );
    rest(cx, s.win);
    assert!(
        !present(cx, s.win, TIP),
        "a move under the dialog keeps it gone"
    );
}

// Closed again, the dialog uncovers the control, and the pointer still on it gets its tip back.
#[gpui_kit::test]
fn the_tooltip_returns_once_the_dialog_closes(cx: &mut TestAppContext) {
    let (s, _col) = hovered_select(cx);
    ask(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("confirm-cancel", cx))
        .unwrap();
    rest(cx, s.win);
    assert!(!present(cx, s.win, "confirm-dialog"), "the dialog closed");
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").hover("env-control", cx)
    })
    .unwrap();
    rest(cx, s.win);
    assert!(present(cx, s.win, TIP), "the tip shows again");
}
