//! The schema hover card closes on typing, scrolling, sending and the pointer leaving.

use crate::support;

use std::ops::Range;
use std::time::Duration;

use apinata_app::graphql_schema_store::{SchemaStore, compiled};
use apinata_app::graphql_section::GraphqlSection;
use apinata_app::request_editor::{Section, SendRequest};
use apinata_core::{BodySpec, Schema};
use gpui_kit::component::input::EditorState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, InputEvent as _, Modifiers, MouseMoveEvent,
    ScrollEvent, TestAppContext, point, px,
};
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};
use support::*;

const URL: &str = "http://hover.test/graphql";
const SDL: &str = "type Query { me: User }\ntype User {\n  \"Display name.\"\n  name: String\n}\n";
const TEXT: &str = "{ me { name } }";

// A section whose query editor holds `TEXT`, checked against the loaded `SDL`.
fn section(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<EditorState>) {
    let (win, host) = host_section(cx, GraphqlSection::new);
    cx.update(|cx| {
        let s = cx.default_global::<SchemaStore>();
        let g = s.begin(URL);
        s.finish(URL, g, Ok(Schema::from_sdl(SDL).unwrap()));
    });
    cx.update(|cx| compiled(URL, cx));
    cx.run_until_parked();
    let s = host.read_with(cx, |h, _| h.view.clone());
    let spec = BodySpec::Graphql {
        query: TEXT.into(),
        variables: String::new(),
    };
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.set_schema_url(URL.into(), cx);
            s.load(&spec, w, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    let query = s.read_with(cx, |s, _| s.query().clone());
    (win, query)
}

fn move_to(cx: &mut TestAppContext, win: AnyWindowHandle, at: gpui_kit::Point<gpui_kit::Pixels>) {
    cx.update_window(win, |_, w, cx| {
        let event = MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Modifiers::default(),
        };
        w.dispatch_event(event.to_platform_input(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
}

// Rests the pointer on `name` until its schema hover opens.
fn hover_name(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    query: &Entity<EditorState>,
) -> gpui_kit::Point<gpui_kit::Pixels> {
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let start = TEXT.find("name").unwrap();
    let range: Range<usize> = start..start + 4;
    let at = query
        .read_with(cx, |e, _| e.range_to_bounds(&range))
        .expect("laid out")
        .center();
    for _ in 0..2 {
        move_to(cx, win, at);
    }
    assert!(open(cx, query), "the hover opened");
    at
}

fn open(cx: &mut TestAppContext, query: &Entity<EditorState>) -> bool {
    query.read_with(cx, |e, _| e.hover_popover().is_some())
}

#[gpui_kit::test]
fn typing_closes_the_hover(cx: &mut TestAppContext) {
    let (win, query) = section(cx);
    cx.update_window(win, |_, w, cx| query.update(cx, |e, cx| e.focus(w, cx)))
        .unwrap();
    hover_name(cx, win, &query);
    cx.update_window(win, |_, w, cx| w.input(" ", cx)).unwrap();
    cx.run_until_parked();
    assert!(!open(cx, &query));
}

#[gpui_kit::test]
fn scrolling_closes_the_hover(cx: &mut TestAppContext) {
    let (win, query) = section(cx);
    // The wheel turns under a resting pointer, so no pointer move closes the card first.
    let at = hover_name(cx, win, &query);
    cx.update_window(win, |_, w, cx| {
        let event = ScrollEvent {
            position: at,
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-40.))),
            ..Default::default()
        };
        w.dispatch_event(event.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!open(cx, &query));
}

#[gpui_kit::test]
fn the_pointer_leaving_closes_the_hover(cx: &mut TestAppContext) {
    let (win, query) = section(cx);
    hover_name(cx, win, &query);
    move_to(cx, win, point(px(1.), px(1.)));
    assert!(!open(cx, &query));
}

#[gpui_kit::test]
fn sending_closes_the_hover(cx: &mut TestAppContext) {
    let gql = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  url: http://127.0.0.1:9/g\n  body:\n    query: \"{ me }\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("g.yml", gql)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, editor) = editor_for(cx, &ws, &root, "g.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    let query = editor.read_with(cx, |e, cx| e.graphql().read(cx).query().clone());
    let card = Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "`Query.me: User`".into(),
        }),
        range: None,
    };
    query.update(cx, |e, cx| e.present_hover(2..4, card, cx));
    frame(cx, win);
    assert!(open(cx, &query));
    cx.update_window(win, |_, w, cx| {
        editor.update(cx, |e, cx| e.send(&SendRequest, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!open(cx, &query));
}
