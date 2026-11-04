//! Short or narrow zoomed windows keep every control reachable: the request column, the
//! response error, Preferences and the Environments editor.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::{Orientation, Settings};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, ElementId, Pixels, ScrollDelta, TestAppContext, point, px, size,
};
use support::*;

// Resizes the window and sets zoom and response orientation, then draws twice so scroll
// handles report the new extent.
fn room(cx: &mut TestAppContext, s: &Started, w: f32, h: f32, zoom: f32, o: Orientation) {
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |s: &mut Settings| {
                s.zoom = zoom;
                s.response_orientation = o;
            })
        });
    });
    frame(cx, s.win);
    frame(cx, s.win);
}

fn bounds(cx: &mut TestAppContext, s: &Started, id: impl Into<ElementId>) -> Bounds<Pixels> {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn scroll_by(cx: &mut TestAppContext, s: &Started, scroller: &'static str, dy: Pixels) {
    cx.update_window(s.win, |_, w, cx| {
        w.scroll(scroller, ScrollDelta::Pixels(point(px(0.), dy)), cx)
    })
    .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
}

// Scrolls `scroller` down just far enough to bring the bottom of `id` into its box. The
// wheel lands on the scroller's centre; when the section strip there takes it sideways
// instead, one long wheel runs the strip to its end and the next reaches the page.
fn reveal(cx: &mut TestAppContext, s: &Started, scroller: &'static str, id: impl Into<ElementId>) {
    let id = id.into();
    let outer = bounds(cx, s, scroller);
    let inner = bounds(cx, s, id.clone());
    let down = inner.bottom() - outer.bottom();
    if down <= px(0.) {
        return;
    }
    scroll_by(cx, s, scroller, -down);
    if bounds(cx, s, id.clone()) == inner && strip_under(cx, s, outer.center()) {
        scroll_by(cx, s, scroller, px(-10_000.));
        scroll_by(cx, s, scroller, -down);
    }
}

// Whether a strip that turns the vertical wheel sideways lies under `at`.
fn strip_under(cx: &mut TestAppContext, s: &Started, at: gpui_kit::Point<Pixels>) -> bool {
    cx.update_window(s.win, |_, w, _| {
        w.try_find("section-strip")
            .is_some_and(|e| e.bounds().contains(&at))
    })
    .unwrap()
}

// Scrolls `scroller` to its top; the second wheel reaches it past a strip that took the first.
fn to_top(cx: &mut TestAppContext, s: &Started, scroller: &'static str) {
    scroll_by(cx, s, scroller, px(10_000.));
    scroll_by(cx, s, scroller, px(10_000.));
}

fn fits(inner: Bounds<Pixels>, top: Pixels, bottom: Pixels) -> bool {
    inner.top() >= top - px(0.5) && inner.bottom() <= bottom + px(0.5)
}

const PARAMS: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0?a=1\"\n  params:\n    - name: a\n      value: \"1\"\n      type: query\n";

#[gpui_kit::test]
fn the_request_column_scrolls_to_its_tabs_and_each_row_in_short_room(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", PARAMS)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let mut failed = Vec::new();
    for (w, h, z, o) in [
        (1280., 360., 1.0, Orientation::Vertical),
        (1280., 360., 1.5, Orientation::Vertical),
        (1280., 360., 2.0, Orientation::Vertical),
        (1280., 360., 1.0, Orientation::Horizontal),
        (1280., 360., 1.5, Orientation::Horizontal),
        (1280., 360., 2.0, Orientation::Horizontal),
        (1000., 700., 2.0, Orientation::Vertical),
        (1280., 800., 1.0, Orientation::Vertical),
    ] {
        room(cx, &s, w, h, z, o);
        let status = bounds(cx, &s, "status-bar");
        let mut miss = Vec::new();
        // Each target, scrolled to from the column's top by the column alone, shows whole
        // inside the column and the section body, above the status bar.
        for id in [
            ElementId::from("section-strip"),
            ElementId::from("kv-head"),
            ("kv-row", 0usize).into(),
            ("kv-row", 1usize).into(),
        ] {
            to_top(cx, &s, "request-column");
            reveal(cx, &s, "request-column", id.clone());
            let column = bounds(cx, &s, "request-column");
            let mut top = column.top();
            let mut floor = column.bottom().min(status.top());
            if id != ElementId::from("section-strip") {
                let body = bounds(cx, &s, "section-body");
                top = top.max(body.top());
                floor = floor.min(body.bottom());
            }
            if !fits(bounds(cx, &s, id.clone()), top, floor) {
                miss.push(format!("{id:?}"));
            }
        }
        if !miss.is_empty() {
            failed.push(format!("{w}x{h} at {z} {o:?}: {miss:?}"));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[gpui_kit::test]
fn retry_is_reachable_in_a_short_window(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = collection_for("http://127.0.0.1:9", "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("send", cx))
        .unwrap();
    wait_real(cx, |cx| present(cx, s.win, "response-retry"));
    for o in [Orientation::Horizontal, Orientation::Vertical] {
        room(cx, &s, 1280., 360., 2.0, o);
        let pane = bounds(cx, &s, "response");
        let status = bounds(cx, &s, "status-bar");
        let floor = pane.bottom().min(status.top());
        scroll_by(cx, &s, "response-error", px(10_000.));
        let icon = bounds(cx, &s, "response-error-icon");
        assert!(
            fits(icon, pane.top(), floor),
            "{o:?}: icon {icon:?} outside {pane:?}"
        );
        reveal(cx, &s, "response-error", "response-retry");
        let retry = bounds(cx, &s, "response-retry");
        assert!(
            fits(retry, pane.top(), floor),
            "{o:?}: Retry {retry:?} outside {pane:?}"
        );
    }
}

#[gpui_kit::test]
fn the_variable_table_is_reachable_in_a_short_window(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let s = started(cx, &col);
    room(cx, &s, 1280., 360., 2.0, Orientation::Vertical);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    reveal(cx, &s, "env-editor", ("env-name", 0usize));
    let editor = bounds(cx, &s, "env-editor");
    let name = bounds(cx, &s, ("env-name", 0usize));
    let status = bounds(cx, &s, "status-bar");
    assert!(
        fits(name, editor.top(), editor.bottom().min(status.top())),
        "first variable {name:?} outside {editor:?}"
    );
}

#[gpui_kit::test]
fn the_variable_columns_keep_their_minimum_at_200_percent(cx: &mut TestAppContext) {
    use apinata_app::style::{ENV_CELL_MIN, ENV_EDITOR_LIST_MIN, ENV_EDITOR_LIST_WIDTH, at_zoom};
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: host\n    value: x\n  - secret: true\n    name: token\n",
        ),
    ]);
    let s = started(cx, &col);
    room(cx, &s, 1000., 700., 1.0, Orientation::Vertical);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    assert_eq!(
        bounds(cx, &s, "env-list").size.width,
        at_zoom(ENV_EDITOR_LIST_WIDTH, 1.0)
    );
    room(cx, &s, 1000., 700., 2.0, Orientation::Vertical);
    let editor = bounds(cx, &s, "env-editor");
    let list = bounds(cx, &s, "env-list");
    // The list gives up only what the table's minimum needs.
    assert!(
        list.size.width >= at_zoom(ENV_EDITOR_LIST_MIN, 2.0) - px(0.5),
        "{list:?}"
    );
    assert!(
        list.size.width <= at_zoom(ENV_EDITOR_LIST_WIDTH, 2.0) + px(0.5),
        "{list:?}"
    );
    for ix in [0usize, 1] {
        for id in ["env-name", "env-value"] {
            let b = bounds(cx, &s, (id, ix));
            assert!(
                b.size.width >= at_zoom(ENV_CELL_MIN, 2.0) - px(0.5),
                "{id} {ix}: {b:?}"
            );
        }
        let delete = bounds(cx, &s, ("env-delete", ix));
        assert!(
            delete.right() <= editor.right() + px(0.5),
            "{delete:?} past {editor:?}"
        );
    }
}

#[gpui_kit::test]
fn the_header_swatches_and_extends_fit_the_pane_at_200_percent(cx: &mut TestAppContext) {
    use apinata_app::style::{ENV_CELL_MIN, at_zoom};
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "environments/a-rather-long-environment-name.yml",
            "name: a-rather-long-environment-name\ncolor: \"#123456\"\nvariables:\n  - name: host\n    value: x\n",
        ),
    ]);
    let s = started(cx, &col);
    room(cx, &s, 1000., 700., 2.0, Orientation::Vertical);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    let pane = bounds(cx, &s, "env-pane");
    for id in [
        "env-title",
        "env-more",
        "env-save",
        "env-swatches",
        "env-swatch-custom",
        "env-extends",
    ] {
        let b = bounds(cx, &s, id);
        assert!(
            b.left() >= pane.left() - px(0.5) && b.right() <= pane.right() + px(0.5),
            "{id} {b:?} leaves {pane:?}"
        );
    }
    for id in ["env-name", "env-value"] {
        let b = bounds(cx, &s, (id, 0usize));
        assert!(
            b.size.width >= at_zoom(ENV_CELL_MIN, 2.0) - px(0.5),
            "{id}: {b:?}"
        );
    }
}

// Opens the Environments tab of a collection whose one environment is `name`.
fn env_tab_for(cx: &mut TestAppContext, name: &str) -> (tempfile::TempDir, Started) {
    let file = format!("environments/{name}.yml");
    let text = format!("name: {name}\nvariables:\n  - name: host\n    value: x\n");
    let col = write_tree(&[("opencollection.yml", ROOT), (&file, &text)]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    (col, s)
}

// A long name truncates in the header: Save and the menu stay inside the pane, the pane
// inside the editor, and the list keeps the width it has under a short name.
#[gpui_kit::test]
fn a_long_environment_name_truncates_and_keeps_save_in_the_window(cx: &mut TestAppContext) {
    let long = "an-environment-name-long-enough-to-crowd-out-everything-else-in-the-header";
    let mut lists = Vec::new();
    let mut failed = Vec::new();
    for name in ["dev", long] {
        let (_col, s) = env_tab_for(cx, name);
        for (w, h, zoom) in [
            (420., 400., 1.0),
            (640., 700., 1.0),
            (1000., 700., 2.0),
            (1280., 700., 2.0),
        ] {
            room(cx, &s, w, h, zoom, Orientation::Vertical);
            let at = format!("{name:.8} {w}x{h} at {zoom}");
            let editor = bounds(cx, &s, "env-editor");
            let pane = bounds(cx, &s, "env-pane");
            let title = bounds(cx, &s, "env-title");
            let more = bounds(cx, &s, "env-more");
            let save = bounds(cx, &s, "env-save");
            lists.push((at.clone(), bounds(cx, &s, "env-list").size.width));
            if pane.right() > editor.right() + px(0.5) || editor.right() > px(w) + px(0.5) {
                failed.push(format!("{at}: pane {pane:?} leaves editor {editor:?}"));
            }
            // Truncating must leave some of the name visible, not squeeze it to nothing.
            if title.size.width <= px(0.) {
                failed.push(format!("{at}: title {title:?} has no width"));
            }
            if title.right() > more.left() + px(0.5) {
                failed.push(format!(
                    "{at}: title {title:?} runs under the menu {more:?}"
                ));
            }
            for (id, b) in [("env-more", more), ("env-save", save)] {
                if b.left() < pane.left() - px(0.5) || b.right() > pane.right() + px(0.5) {
                    failed.push(format!("{at}: {id} {b:?} leaves {pane:?}"));
                }
            }
        }
    }
    // The long name's list widths match the short name's, size for size.
    let (short, long) = lists.split_at(lists.len() / 2);
    for ((at_s, a), (at_l, b)) in short.iter().zip(long) {
        if (*a - *b).abs() > px(0.5) {
            failed.push(format!("list {at_l} is {b:?}, {at_s} is {a:?}"));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}
