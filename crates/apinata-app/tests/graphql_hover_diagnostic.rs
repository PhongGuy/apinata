//! A live-check diagnostic on a schema name carries that name's hover, so one popover shows.

use crate::support;

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use apinata_app::graphql_check::CHECK_DELAY;
use apinata_app::graphql_schema_store::{SchemaStore, compiled};
use apinata_app::graphql_section::GraphqlSection;
use apinata_core::{BodySpec, Schema};
use gpui_kit::component::input::EditorState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, InputEvent as _, Modifiers, MouseMoveEvent,
    TestAppContext,
};
use lsp_types::HoverContents;
use support::*;

const URL: &str = "http://hover.test/graphql";
const SDL: &str = r#"type Query { me: User }
type User {
  "Display name."
  name: String
  "Former name."
  legacyName: String @deprecated(reason: "Use name")
}
"#;

fn section(cx: &mut TestAppContext, query: &str) -> (AnyWindowHandle, Entity<EditorState>) {
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
        query: query.into(),
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
    cx.executor().advance_clock(CHECK_DELAY);
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let query = s.read_with(cx, |s, _| s.query().clone());
    (win, query)
}

// Rest the pointer on the middle of `range` and wait out the hover delay.
fn hover_over(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    query: &Entity<EditorState>,
    range: Range<usize>,
) {
    let at = query
        .read_with(cx, |e, _| e.range_to_bounds(&range))
        .expect("laid out")
        .center();
    for _ in 0..2 {
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
}

// The hover's Markdown, if one is open, and the diagnostic popover's message, if one is open.
fn popovers(
    cx: &mut TestAppContext,
    query: &Entity<EditorState>,
) -> (Option<String>, Option<String>) {
    query.read_with(cx, |e, _| {
        let hover = e.hover_popover().map(|h| match &h.hover.contents {
            HoverContents::Markup(m) => m.value.clone(),
            other => panic!("not Markdown: {other:?}"),
        });
        let diagnostic = e
            .diagnostic_popover()
            .map(|d| d.diagnostic.message.to_string());
        (hover, diagnostic)
    })
}

fn span(text: &str, word: &str) -> Range<usize> {
    let start = text.find(word).expect("word in text");
    start..start + word.len()
}

#[gpui_kit::test]
fn a_deprecated_field_shows_one_popover_holding_warning_and_description(cx: &mut TestAppContext) {
    let text = "{ me { legacyName } }";
    let (win, query) = section(cx, text);
    hover_over(cx, win, &query, span(text, "legacyName"));
    let (hover, diagnostic) = popovers(cx, &query);
    assert_eq!(
        hover, None,
        "the schema hover stays quiet under the warning"
    );
    let message = diagnostic.expect("the diagnostic popover");
    assert!(
        message.starts_with("`User.legacyName` is deprecated\\: Use name\n\n---\n\n"),
        "{message}"
    );
    assert!(
        message.contains("`User.legacyName: String`\n\nFormer name\\."),
        "{message}"
    );
}

#[gpui_kit::test]
fn moving_across_a_squiggle_keeps_the_popover_without_redrawing(cx: &mut TestAppContext) {
    let text = "{ me { legacyName } }";
    let (win, query) = section(cx, text);
    let name = span(text, "legacyName");
    hover_over(cx, win, &query, name.clone());
    let open = query.read_with(cx, |e, _| e.diagnostic_popover().expect("open"));
    let redraws = Rc::new(Cell::new(0));
    let counter = Rc::clone(&redraws);
    let _watch = cx.update(|cx| cx.observe(&query, move |_, _| counter.set(counter.get() + 1)));
    for offset in name.clone() {
        let at = query
            .read_with(cx, |e, _| e.range_to_bounds(&(offset..offset + 1)))
            .expect("laid out")
            .center();
        cx.update_window(win, |_, w, cx| {
            let event = MouseMoveEvent {
                position: at,
                pressed_button: None,
                modifiers: Modifiers::default(),
            };
            w.dispatch_event(event.to_platform_input(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let now = query.read_with(cx, |e, _| e.diagnostic_popover().expect("still open"));
        assert!(
            Rc::ptr_eq(&open, &now),
            "the popover was closed and reopened"
        );
    }
    assert_eq!(
        redraws.get(),
        0,
        "moves within the squiggle redrew the editor"
    );
}

#[gpui_kit::test]
fn a_wide_warning_keeps_its_own_text_beside_the_name_hover(cx: &mut TestAppContext) {
    let text = "{ me { name } }\nfragment Unused on User { name }";
    let (win, query) = section(cx, text);
    let inner = text.rfind("name").unwrap();
    hover_over(cx, win, &query, inner..inner + 4);
    let (hover, diagnostic) = popovers(cx, &query);
    assert_eq!(
        hover.as_deref(),
        Some("`User.name: String`\n\nDisplay name\\.")
    );
    let message = diagnostic.expect("the diagnostic popover");
    assert!(message.contains("Unused"), "{message}");
    assert!(!message.contains("---"), "{message}");
}

#[gpui_kit::test]
fn a_squiggle_without_a_schema_name_shows_just_the_warning(cx: &mut TestAppContext) {
    let text = "{ me { nmae } }";
    let (win, query) = section(cx, text);
    hover_over(cx, win, &query, span(text, "nmae"));
    assert_eq!(
        popovers(cx, &query),
        (
            None,
            Some("type `User` does not have a field `nmae`".to_owned())
        )
    );
}

#[gpui_kit::test]
fn a_clean_name_shows_the_hover_alone(cx: &mut TestAppContext) {
    let text = "{ me { name } }";
    let (win, query) = section(cx, text);
    hover_over(cx, win, &query, span(text, "name"));
    assert_eq!(
        popovers(cx, &query),
        (
            Some("`User.name: String`\n\nDisplay name\\.".to_owned()),
            None
        )
    );
}
