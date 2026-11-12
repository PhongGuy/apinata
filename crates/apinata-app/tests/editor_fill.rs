//! The request section's code editors fill the pane's height down to their minimum, and the
//! section scrolls only once the pane is shorter than that.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::request_editor::Section;
use apinata_app::settings::{Orientation, Settings};
use apinata_app::style::{EDITOR_HEIGHT, EDITOR_MIN_HEIGHT, resolve};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext, px, size};
use support::*;

const JSON: &str = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: http://x/0\n  body:\n    type: json\n    data: '{\"a\": 1}'\ndocs: notes\n";
const GRAPHQL: &str = "info:\n  name: g\n  type: graphql\n  seq: 2\ngraphql:\n  method: POST\n  url: http://x/gql\n  body:\n    query: \"{ me }\"\n";

// Resizes the window and sets the response's side, then draws twice so the layout settles.
fn room(cx: &mut TestAppContext, s: &Started, w: f32, h: f32, o: Orientation) {
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |s: &mut Settings| s.response_orientation = o)
        });
    });
    frame(cx, s.win);
    frame(cx, s.win);
}

fn bounds(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn px_of(cx: &mut TestAppContext, s: &Started, len: gpui_kit::Rems) -> Pixels {
    cx.update_window(s.win, |_, w, _| resolve(len, w)).unwrap()
}

fn opened(cx: &mut TestAppContext, yaml: &str, section: Section) -> (Started, tempfile::TempDir) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", yaml)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    room(cx, &s, 1600., 1200., Orientation::Horizontal);
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", section as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    frame(cx, s.win);
    (s, col)
}

// Each code-editing section, its content wrapper, the control above its editors and its
// editors top to bottom.
fn cases() -> [(
    &'static str,
    Section,
    &'static str,
    &'static str,
    &'static [&'static str],
); 5] {
    [
        (JSON, Section::Body, "body", "body-mode", &["body-code"]),
        (
            GRAPHQL,
            Section::Body,
            "body",
            "graphql-schema-file",
            &["graphql-query", "graphql-variables"],
        ),
        (
            JSON,
            Section::Script,
            "script",
            "script",
            &["script-pre-request-code"],
        ),
        (
            JSON,
            Section::Tests,
            "tests",
            "tests",
            &["script-tests-code"],
        ),
        (JSON, Section::Docs, "docs", "docs-mode", &["docs-preview"]),
    ]
}

#[gpui_kit::test]
fn code_editors_fill_a_tall_pane_without_an_outer_scroll(cx: &mut TestAppContext) {
    let mut failed = Vec::new();
    for (yaml, section, wrapper, control, editors) in cases() {
        let (s, _col) = opened(cx, yaml, section);
        for o in [Orientation::Horizontal, Orientation::Vertical] {
            let h = if o == Orientation::Vertical {
                2000.
            } else {
                1200.
            };
            room(cx, &s, 1600., h, o);
            let pane = bounds(cx, &s, "section-body");
            let content = bounds(cx, &s, wrapper);
            let last = bounds(cx, &s, editors[editors.len() - 1]);
            let tall = px_of(cx, &s, EDITOR_HEIGHT);
            let top = bounds(cx, &s, control);
            let mut miss = Vec::new();
            if content.bottom() > pane.bottom() + px(0.5) {
                miss.push(format!("content {content:?} overflows {pane:?}"));
            }
            if pane.bottom() - last.bottom() > px(24.) {
                miss.push(format!("last editor {last:?} leaves room in {pane:?}"));
            }
            for id in editors {
                let e = bounds(cx, &s, id);
                if e.size.height <= tall {
                    miss.push(format!("{id} {e:?} kept the fixed height {tall:?}"));
                }
            }
            if top.top() < pane.top() - px(0.5) {
                miss.push(format!("{control} {top:?} scrolled above {pane:?}"));
            }
            if !miss.is_empty() {
                failed.push(format!("{section:?} {o:?}: {miss:#?}"));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[gpui_kit::test]
fn code_editors_keep_their_minimum_in_a_short_pane(cx: &mut TestAppContext) {
    let mut failed = Vec::new();
    for (yaml, section, wrapper, _, editors) in cases() {
        let (s, _col) = opened(cx, yaml, section);
        room(cx, &s, 1600., 420., Orientation::Horizontal);
        let pane = bounds(cx, &s, "section-body");
        let content = bounds(cx, &s, wrapper);
        let least = px_of(cx, &s, EDITOR_MIN_HEIGHT);
        let mut miss = Vec::new();
        for id in editors {
            let e = bounds(cx, &s, id);
            if e.size.height < least - px(0.5) {
                miss.push(format!("{id} {e:?} under the minimum {least:?}"));
            }
        }
        // Below the minimum the section scrolls instead of squeezing the editor.
        if content.bottom() <= pane.bottom() {
            miss.push(format!("content {content:?} does not scroll in {pane:?}"));
        }
        if !miss.is_empty() {
            failed.push(format!("{section:?}: {miss:#?}"));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[gpui_kit::test]
fn graphql_editors_fill_beside_an_open_schema_panel(cx: &mut TestAppContext) {
    let (s, _col) = opened(cx, GRAPHQL, Section::Body);
    cx.update_window(s.win, |_, w, cx| w.click("graphql-docs-toggle", cx))
        .unwrap();
    frame(cx, s.win);
    let least = px_of(cx, &s, EDITOR_MIN_HEIGHT);
    for h in [1200., 420.] {
        room(cx, &s, 1600., h, Orientation::Horizontal);
        let pane = bounds(cx, &s, "section-body");
        let content = bounds(cx, &s, "body");
        let query = bounds(cx, &s, "graphql-query");
        let variables = bounds(cx, &s, "graphql-variables");
        assert!(
            query.size.height >= least - px(0.5) && variables.size.height >= least - px(0.5),
            "{h}: {query:?} {variables:?} under {least:?}"
        );
        if h > 1000. {
            assert!(
                content.bottom() <= pane.bottom() + px(0.5)
                    && pane.bottom() - variables.bottom() <= px(24.),
                "{h}: {variables:?} does not fill {pane:?}"
            );
        } else {
            assert!(
                content.bottom() > pane.bottom(),
                "{h}: {content:?} in {pane:?}"
            );
        }
    }
}
