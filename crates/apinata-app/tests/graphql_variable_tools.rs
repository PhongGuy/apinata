//! The variables editor's provider: query text through a weak handle, schema from the store.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;

use apinata_app::graphql_schema_store::SchemaStore;
use apinata_app::graphql_section::GraphqlSection;
use apinata_app::graphql_variables::VariableTools;
use apinata_core::{BodySpec, Schema};
use gpui_kit::component::Rope;
use gpui_kit::component::input::CompletionProvider as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use lsp_types::{HoverContents, MarkupContent};
use support::*;

const URL: &str = "http://x.test/g";
const Q: &str = "query Q($id: ID!, $role: Role, $input: NewUser) { user(id: $id) { name } }";

fn fixture() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}

fn section(cx: &mut TestAppContext, query: &str) -> (AnyWindowHandle, Entity<GraphqlSection>) {
    let (win, host) = host_section(cx, GraphqlSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    let spec = BodySpec::Graphql {
        query: query.into(),
        variables: String::new(),
    };
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    cx.run_until_parked();
    (win, s)
}

fn labels(
    tools: &VariableTools,
    cx: &mut TestAppContext,
    text: &str,
    offset: usize,
) -> Vec<String> {
    cx.update(|cx| tools.completion_answer(&Rope::from(text), offset, cx))
        .into_iter()
        .map(|i| i.label)
        .collect()
}

#[gpui_kit::test]
fn the_tools_read_the_query_and_fall_back_while_the_schema_loads(cx: &mut TestAppContext) {
    let (win, s) = section(cx, Q);
    let query = s.read_with(cx, |s, _| s.query().downgrade());
    let tools = VariableTools::new(query, Rc::new(RefCell::new(URL.to_owned())));
    let nested = "{\"input\": {\"\"}}";
    assert_eq!(labels(&tools, cx, "{\"\"}", 2), ["id", "role", "input"]);
    assert!(labels(&tools, cx, nested, 12).is_empty());
    let generation = cx.update(|cx| cx.default_global::<SchemaStore>().begin(URL));
    assert_eq!(labels(&tools, cx, "{\"\"}", 2), ["id", "role", "input"]);
    cx.update(|cx| {
        cx.default_global::<SchemaStore>()
            .finish(URL, generation, Ok(fixture()))
    });
    assert_eq!(labels(&tools, cx, nested, 12), ["name", "role"]);
    let hover = cx
        .update(|cx| tools.hover_answer(&Rope::from("{\"input\": {\"role\": 1}}"), 12, cx))
        .expect("a hover");
    let HoverContents::Markup(MarkupContent { value, .. }) = hover.contents else {
        panic!("not Markdown")
    };
    assert!(value.starts_with("`NewUser.role: Role`"), "{value}");
    let spec = BodySpec::Graphql {
        query: "query Q($other: Int) { a }".into(),
        variables: String::new(),
    };
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    assert_eq!(labels(&tools, cx, "{\"\"}", 2), ["other"]);
}

#[gpui_kit::test]
fn the_variables_editor_has_completion_and_hover(cx: &mut TestAppContext) {
    let (_win, s) = section(cx, Q);
    let variables = s.read_with(cx, |s, _| s.variables().clone());
    assert!(variables.read_with(cx, |v, _| {
        let l = v.lsp();
        l.completion_provider.is_some() && l.hover_provider.is_some()
    }));
}

#[gpui_kit::test]
fn json_punctuation_and_names_trigger_the_menu(cx: &mut TestAppContext) {
    init(cx);
    let tools = VariableTools::new(
        gpui_kit::WeakEntity::new_invalid(),
        Rc::new(RefCell::new(String::new())),
    );
    for t in ["a", "Z", "_", "1", "\"", "{", "[", ",", ":", " "] {
        assert!(
            cx.update(|cx| tools.is_completion_trigger(0, t, cx)),
            "{t:?}"
        );
    }
    for t in ["}", "]", "\n", "-", ""] {
        assert!(
            !cx.update(|cx| tools.is_completion_trigger(0, t, cx)),
            "{t:?}"
        );
    }
}

// Type `text` into the focused editor, then press each of `keys` as a platform keystroke,
// which also carries the key's character (a newline for Enter) when the action propagates.
fn keys(cx: &mut TestAppContext, win: AnyWindowHandle, text: &str, keys: &[&str]) {
    use gpui_kit::test::TestWindowExt as _;
    for ch in text.chars() {
        cx.update_window(win, |_, w, cx| {
            w.input(&ch.to_string(), cx);
            w.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    for k in keys {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.dispatch_keystroke(
                gpui_kit::Keystroke::parse_with_os(k, apinata_app::shortcuts::HOST_OS).unwrap(),
                cx,
            );
            w.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn typing_a_quote_offers_variables_and_enter_accepts_one(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (win, s) = section(cx, Q);
    cx.update_window(win, |_, w, cx| {
        w.click_at("graphql-variables", code_at(), cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    keys(cx, win, "{\"", &["enter"]);
    let variables = s.read_with(cx, |s, _| s.variables().clone());
    assert_eq!(
        variables.read_with(cx, |v, _| v.value().to_string()),
        "{\"id\": }"
    );
}
