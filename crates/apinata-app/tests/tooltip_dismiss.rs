//! A tooltip goes once a dialog opens over its control, even while the pointer rests there.

use crate::support;

use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, InteractiveElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestAppContext, TestSupportExt as _,
    VisualTestContext, point, px,
};
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

const PROBE_TIP: &str = "probe-tip";

// A collection window with a dialog open whose one control has an element tooltip, hovered
// until the tip shows.
fn hovered_dialog_control(cx: &mut TestAppContext) -> (Started, tempfile::TempDir) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::ui::open_dialog(w, cx, |d, _, _| {
            d.close_button(false).child(
                gpui_kit::div()
                    .id("probe")
                    .test_support()
                    .size_8()
                    .tooltip(apinata_app::ui::text_tooltip(PROBE_TIP, "Probe")),
            )
        });
    })
    .unwrap();
    rest(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.hover("probe", cx))
        .unwrap();
    rest(cx, s.win);
    assert!(
        present(cx, s.win, PROBE_TIP),
        "the dialog's control shows its tip"
    );
    (s, col)
}

// A second dialog opened from the keyboard over the first covers the first's controls too.
#[gpui_kit::test]
fn a_dialog_opened_over_a_dialog_hides_the_tooltip_inside_it(cx: &mut TestAppContext) {
    let (s, _col) = hovered_dialog_control(cx);
    ask(cx, s.win);
    assert!(
        !present(cx, s.win, PROBE_TIP),
        "no tip from the covered dialog"
    );
}

// The dialog the tip was shown in is still the top layer, so its tip stays.
#[gpui_kit::test]
fn a_tooltip_inside_the_top_dialog_stays(cx: &mut TestAppContext) {
    let (s, _col) = hovered_dialog_control(cx);
    rest(cx, s.win);
    assert!(
        present(cx, s.win, PROBE_TIP),
        "the tip stays on the top dialog"
    );
}

// Every dialog the app opens counts, alerts included, in its own window.
#[gpui_kit::test]
fn opening_a_dialog_counts_it(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let opens = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| apinata_app::ui::dialog_opens(w, cx))
            .unwrap()
    };
    let before = opens(cx);
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::ui::open_dialog(w, cx, |d, _, _| d.close_button(false));
    })
    .unwrap();
    assert_eq!(opens(cx), before + 1, "a dialog counts");
    cx.update_window(s.win, |_, w, cx| {
        apinata_app::ui::open_alert_dialog(w, cx, |a, _, _| a.title("Alert"));
    })
    .unwrap();
    assert_eq!(opens(cx), before + 2, "an alert counts");
}
