//! The request editor's section body and the environment editor show a scrollbar while
//! they scroll, and none while their content fits.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, ElementId, Pixels, TestAppContext, px, size};
use support::*;

// Resizes the window and sets zoom, then draws twice so scroll handles report the new extent.
fn room(cx: &mut TestAppContext, s: &Started, w: f32, h: f32, zoom: f32) {
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s: &mut Settings| s.zoom = zoom));
    });
    for _ in 0..3 {
        frame(cx, s.win);
    }
}

fn find(cx: &mut TestAppContext, s: &Started, id: impl Into<ElementId>) -> Option<Bounds<Pixels>> {
    let id = id.into();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).map(|e| e.bounds())
    })
    .unwrap()
}

fn params(n: usize) -> String {
    let mut out = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"http://x/0?p0=1\"\n  params:\n".to_string();
    for i in 0..n {
        out += &format!("    - name: p{i}\n      value: \"1\"\n      type: query\n");
    }
    out
}

fn variables(n: usize) -> String {
    let mut out = "name: dev\nvariables:\n".to_string();
    for i in 0..n {
        out += &format!("  - name: v{i}\n    value: x\n");
    }
    out
}

// The scrollbar must be drawn inside its scroll box with a real extent.
fn assert_drawn(
    cx: &mut TestAppContext,
    s: &Started,
    id: &'static str,
    within: &'static str,
    content: ElementId,
) {
    let bar = find(cx, s, id).unwrap_or_else(|| panic!("{id} is missing"));
    let outer = find(cx, s, within).expect("scroll box");
    assert!(
        bar.size.width > px(0.) && bar.size.height > px(0.),
        "{id} {bar:?} is empty"
    );
    assert!(
        bar.bottom() <= outer.bottom() + px(0.5) && bar.right() <= outer.right() + px(0.5),
        "{id} {bar:?} leaves {outer:?}"
    );
    // The thumb has its own gutter: the content ends at or left of it.
    let content = find(cx, s, content).expect("content");
    assert!(
        content.right() <= bar.left() + px(0.5),
        "{id} {bar:?} overlaps content {content:?}"
    );
}

fn open_request(cx: &mut TestAppContext, n: usize) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &params(n))]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    (col, s)
}

#[gpui_kit::test]
fn the_section_body_shows_a_scrollbar_only_while_it_overflows(cx: &mut TestAppContext) {
    let (_col, s) = open_request(cx, 8);
    for (h, zoom) in [(360., 2.0), (300., 1.0)] {
        room(cx, &s, 1280., h, zoom);
        // Precondition: the last row is cut off by the body's box.
        let body = find(cx, &s, "section-body").expect("section body");
        let last = find(cx, &s, ("kv-row", 7usize)).expect("last row");
        assert!(last.bottom() > body.bottom(), "rows {last:?} fit {body:?}");
        assert_drawn(
            cx,
            &s,
            "section-body-scrollbar",
            "section-body",
            ("kv-row", 0usize).into(),
        );
    }

    let (_col, s) = open_request(cx, 1);
    room(cx, &s, 1280., 800., 1.0);
    let body = find(cx, &s, "section-body").expect("section body");
    let last = find(cx, &s, ("kv-row", 0usize)).expect("row");
    assert!(
        last.bottom() <= body.bottom(),
        "row {last:?} overflows {body:?}"
    );
    assert!(find(cx, &s, "section-body-scrollbar").is_none());
}

fn open_envs(cx: &mut TestAppContext, n: usize) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", &variables(n)),
    ]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("environments", cx)
    })
    .unwrap();
    frame(cx, s.win);
    (col, s)
}

#[gpui_kit::test]
fn the_environment_editor_shows_a_scrollbar_only_while_it_overflows(cx: &mut TestAppContext) {
    let (_col, s) = open_envs(cx, 8);
    for (h, zoom) in [(360., 2.0), (300., 1.0)] {
        room(cx, &s, 1280., h, zoom);
        // Precondition: the last variable row is cut off by the editor's box.
        let editor = find(cx, &s, "env-editor").expect("editor");
        let last = find(cx, &s, ("env-name", 7usize)).expect("last variable");
        assert!(
            last.bottom() > editor.bottom(),
            "rows {last:?} fit {editor:?}"
        );
        assert_drawn(
            cx,
            &s,
            "env-editor-scrollbar",
            "env-editor",
            "env-pane".into(),
        );
    }

    let (_col, s) = open_envs(cx, 2);
    room(cx, &s, 1280., 800., 1.0);
    let editor = find(cx, &s, "env-editor").expect("editor");
    let last = find(cx, &s, ("env-name", 1usize)).expect("variable");
    assert!(
        last.bottom() <= editor.bottom(),
        "rows {last:?} overflow {editor:?}"
    );
    assert!(find(cx, &s, "env-editor-scrollbar").is_none());
}
