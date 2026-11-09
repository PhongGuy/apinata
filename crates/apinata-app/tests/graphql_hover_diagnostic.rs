//! A live-check diagnostic under a hovered name joins the hover instead of a second popover.

use crate::support;

use std::ops::Range;
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

// The hover's Markdown, if one is open, and whether the diagnostic popover is open.
fn popovers(cx: &mut TestAppContext, query: &Entity<EditorState>) -> (Option<String>, bool) {
    query.read_with(cx, |e, _| {
        let hover = e.hover_popover().map(|h| match &h.hover.contents {
            HoverContents::Markup(m) => m.value.clone(),
            other => panic!("not Markdown: {other:?}"),
        });
        (hover, e.diagnostic_popover().is_some())
    })
}

fn squiggle_messages(cx: &mut TestAppContext, query: &Entity<EditorState>) -> Vec<String> {
    query.read_with(cx, |e, _| {
        e.diagnostics()
            .map(|set| {
                set.iter()
                    .map(|d| d.diagnostic.message.to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn span(text: &str, word: &str) -> Range<usize> {
    let start = text.find(word).expect("word in text");
    start..start + word.len()
}

#[gpui_kit::test]
fn a_deprecated_field_folds_its_warning_into_the_hover(cx: &mut TestAppContext) {
    let text = "{ me { legacyName } }";
    let (win, query) = section(cx, text);
    let messages = squiggle_messages(cx, &query);
    assert_eq!(messages, ["`User.legacyName` is deprecated: Use name"]);
    hover_over(cx, win, &query, span(text, "legacyName"));
    let (hover, diagnostic) = popovers(cx, &query);
    let hover = hover.expect("a hover");
    assert!(
        hover.starts_with("`User.legacyName: String`\n\nFormer name\\."),
        "{hover}"
    );
    assert!(
        hover.ends_with("\n\n---\n\n`User.legacyName` is deprecated\\: Use name"),
        "{hover}"
    );
    assert!(!diagnostic, "the diagnostic popover stays closed");
}

#[gpui_kit::test]
fn an_unused_fragment_folds_into_the_hover_of_a_name_inside_it(cx: &mut TestAppContext) {
    let text = "{ me { name } }\nfragment Unused on User { name }";
    let (win, query) = section(cx, text);
    let messages = squiggle_messages(cx, &query);
    assert!(
        messages.iter().any(|m| m.contains("Unused")),
        "{messages:?}"
    );
    let inner = text.rfind("name").unwrap();
    hover_over(cx, win, &query, inner..inner + 4);
    let (hover, diagnostic) = popovers(cx, &query);
    let hover = hover.expect("a hover");
    assert!(hover.contains("Display name\\."), "{hover}");
    assert!(hover.contains("\n\n---\n\n"), "{hover}");
    assert!(hover.contains("Unused"), "{hover}");
    assert!(!diagnostic, "the diagnostic popover stays closed");
}

#[gpui_kit::test]
fn a_squiggle_without_a_hover_keeps_the_diagnostic_popover(cx: &mut TestAppContext) {
    let text = "{ me { nmae } }";
    let (win, query) = section(cx, text);
    assert_eq!(squiggle_messages(cx, &query).len(), 1);
    hover_over(cx, win, &query, span(text, "nmae"));
    assert_eq!(popovers(cx, &query), (None, true));
}

#[gpui_kit::test]
fn a_clean_name_shows_the_hover_alone(cx: &mut TestAppContext) {
    let text = "{ me { name } }";
    let (win, query) = section(cx, text);
    assert!(squiggle_messages(cx, &query).is_empty());
    hover_over(cx, win, &query, span(text, "name"));
    assert_eq!(
        popovers(cx, &query),
        (
            Some("`User.name: String`\n\nDisplay name\\.".to_owned()),
            false
        )
    );
}
