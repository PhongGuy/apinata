//! Dialogs keep a usable minimum in a short, zoomed window: the palette, Generate Code and
//! Preferences at 1280x360 and 200%.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::palette::{index_path, palette_items};
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Focusable as _, Pixels, TestAppContext, px, size};
use support::*;

fn short_window(cx: &mut TestAppContext, s: &Started, zoom: f32) {
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1280.), px(360.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s: &mut Settings| s.zoom = zoom));
    });
    frame(cx, s.win);
    frame(cx, s.win);
}

fn bounds(cx: &mut TestAppContext, s: &Started, id: &str) -> Bounds<Pixels> {
    let id = id.to_string();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

// Three whole rows plus the input need about 270 px of room above the status bar, and a
// 360 px window at 200% leaves about 240 px, so the floor is the heading and one whole row;
// the rest of the list scrolls with the arrow keys while the input stays in view.
#[gpui_kit::test]
fn the_palette_keeps_a_row_and_stays_reachable_at_1280x360_and_200_percent(
    cx: &mut TestAppContext,
) {
    let col = two_requests();
    let s = started(cx, &col);
    short_window(cx, &s, 2.0);
    cx.update_window(s.win, |_, w, cx| {
        let fallback = w.focused(cx).unwrap_or_else(|| cx.focus_handle());
        apinata_app::palette::open(fallback, w, cx);
    })
    .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
    let dialog = bounds(cx, &s, "command-palette");
    let status = bounds(cx, &s, "status-bar");
    assert!(
        dialog.bottom() <= status.top(),
        "{dialog:?} over {status:?}"
    );
    // The dialog uses the room: what is left under it would not fit another row (56 px).
    assert!(
        status.top() - dialog.bottom() < px(56.),
        "{dialog:?} leaves a row of room above {status:?}"
    );
    let items = palette_items();
    let whole = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, _| {
            (0..items.len())
                .filter_map(|ix| w.within("command-palette").try_find(index_path(&items, ix)))
                .map(|e| e.bounds())
                .filter(|b| b.bottom() <= dialog.bottom() && b.top() >= dialog.top())
                .count()
        })
        .unwrap()
    };
    assert!(whole(cx) >= 1, "no whole row in {dialog:?}");
    for _ in 0..6 {
        cx.simulate_keystrokes(s.win, "down");
    }
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(whole(cx) >= 1, "no whole row after moving down");
    assert_eq!(bounds(cx, &s, "command-palette"), dialog);
}

fn open_generate_code_short(cx: &mut TestAppContext) -> Started {
    let col = two_requests();
    let s = started(cx, &col);
    std::mem::forget(col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    short_window(cx, &s, 2.0);
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx)
    })
    .unwrap();
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input("generate code", cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
    s
}

// A code line is about 20 px at 100%; the preview keeps at least three, and the dialog
// stays above the status bar with the copy button under the target, off the code.
#[gpui_kit::test]
fn generate_code_keeps_three_lines_at_1280x360_and_200_percent(cx: &mut TestAppContext) {
    let s = open_generate_code_short(cx);
    let preview = bounds(cx, &s, "generate-code-preview");
    let copy = bounds(cx, &s, "generate-code-copy");
    let status = bounds(cx, &s, "status-bar");
    assert!(
        preview.bottom() <= status.top(),
        "{preview:?} over {status:?}"
    );
    assert!(
        preview.size.height >= px(3. * 20. * 2.),
        "{preview:?} holds fewer than three lines"
    );
    let target = bounds(cx, &s, "generate-code-target");
    assert!(
        target.right() <= preview.left(),
        "{target:?} beside {preview:?}"
    );
    assert!(copy.top() >= target.bottom(), "{copy:?} under {target:?}");
    assert!(!copy.intersects(&preview), "{copy:?} off {preview:?}");
}

// At 1000x700 and 200% the Name and Value cells keep a width that reads: at least 90 px at
// 100%, about thirteen characters, not just the layout's own floor.
#[gpui_kit::test]
fn the_environment_cells_read_at_1000x700_and_200_percent(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let s = started(cx, &col);
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(1000.), px(700.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s: &mut Settings| s.zoom = 2.0));
    });
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
    for id in ["env-name", "env-value"] {
        let b = cx
            .update_window(s.win, |_, w, _| w.find((id, 0usize)).bounds())
            .unwrap();
        assert!(
            b.size.width >= px(180.),
            "{id} {b:?} is under 90 px at 100%"
        );
    }
}

// Side by side, the target column is 192 px at 100% and scales with zoom.
#[gpui_kit::test]
fn the_side_by_side_target_column_is_384_px_at_200_percent(cx: &mut TestAppContext) {
    let s = open_generate_code_short(cx);
    let column = bounds(cx, &s, "generate-code-target");
    assert!(
        (column.size.width - px(384.)).abs() < px(1.),
        "{column:?} is not 384 px at 200%"
    );
}
