//! Hover text and definition links for names in the GraphQL query editor.

use crate::support;

use std::cell::RefCell;
use std::rc::Rc;

use apinata_app::graphql_docs::DocsView;
use apinata_app::graphql_hover::{SchemaSymbols, definition, docs_view, hover};
use apinata_app::graphql_schema_store::SchemaStore;
use apinata_core::Schema;
use gpui_kit::TestAppContext;
use gpui_kit::component::Rope;
use lsp_types::{HoverContents, MarkupContent, MarkupKind, Position, Range};
use support::*;

// `user` 2..6, `id` 7..9, `name` 16..20, `legacyName` 21..31.
const Q: &str = "{ user(id: 1) { name legacyName } }";

fn schema() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}

fn r(l1: u32, c1: u32, l2: u32, c2: u32) -> Range {
    Range::new(Position::new(l1, c1), Position::new(l2, c2))
}

fn hover_at(text: &str, offset: usize) -> Option<(String, Range)> {
    hover(&schema(), &Rope::from(text), offset).map(|h| {
        let HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }) = h.contents
        else {
            panic!("not Markdown: {:?}", h.contents)
        };
        (value, h.range.expect("a range"))
    })
}

fn link_at(text: &str, offset: usize) -> Option<(String, Range)> {
    definition(&schema(), &Rope::from(text), offset).map(|l| {
        assert_eq!(l.target_range, Range::default());
        assert_eq!(l.target_selection_range, Range::default());
        (
            l.target_uri.as_str().to_owned(),
            l.origin_selection_range.expect("an origin"),
        )
    })
}

#[test]
fn hover_shows_the_signature_description_and_deprecation() {
    assert_eq!(
        hover_at(Q, 3),
        Some((
            "`Query.user(id: ID!, active: Boolean = true): User`\n\nOne user by id\\.".into(),
            r(0, 2, 0, 6)
        ))
    );
    assert_eq!(
        hover_at(Q, 8),
        Some(("`id: ID!`\n\nThe user id\\.".into(), r(0, 7, 0, 9)))
    );
    assert_eq!(
        hover_at(Q, 18),
        Some((
            "`User.name: String`\n\nDisplay name\\.".into(),
            r(0, 16, 0, 20)
        ))
    );
    assert_eq!(
        hover_at(Q, 25),
        Some((
            "`User.legacyName: String`\n\nDeprecated".into(),
            r(0, 21, 0, 31)
        ))
    );
    assert_eq!(
        hover_at("{ oldUsers { id } }", 4),
        Some((
            "`Query.oldUsers: [User]`\n\nDeprecated: Use users".into(),
            r(0, 2, 0, 10)
        ))
    );
    assert_eq!(
        hover_at("{ users(first: 1) { id } }", 9)
            .map(|h| h.0)
            .as_deref(),
        Some("`first: Int = 10`")
    );
}

#[test]
fn hover_names_a_type_by_its_kind() {
    assert_eq!(
        hover_at("{ node(id: 1) { ... on User { id } } }", 24),
        Some(("`type User`\n\nA person\\.".into(), r(0, 23, 0, 27)))
    );
    let v = "query Q($n: NewUser, $r: Role, $i: ID) { user(id: $i) { id } }";
    assert_eq!(
        hover_at(v, 13).map(|h| h.0).as_deref(),
        Some("`input NewUser`")
    );
    assert_eq!(hover_at(v, 26).map(|h| h.0).as_deref(), Some("`enum Role`"));
    assert_eq!(hover_at(v, 36).map(|h| h.0).as_deref(), Some("`scalar ID`"));
    let s = "{ search(term: \"x\") { ... on Node { id } ... on SearchResult { __typename } } }";
    assert_eq!(
        hover_at(s, 30).map(|h| h.0).as_deref(),
        Some("`interface Node`\n\nAnything with an id\\.")
    );
    let union = s.find("SearchResult").unwrap() + 1;
    assert_eq!(
        hover_at(s, union).map(|h| h.0).as_deref(),
        Some("`union SearchResult`")
    );
}

#[test]
fn ranges_count_chars_and_unknown_names_have_no_hover() {
    // `name` is bytes 23..27 but chars 19..23 after the two 3-byte chars.
    let q = "{ user(id: \"日本\") { name } }";
    assert_eq!(hover_at(q, 24).map(|h| h.1), Some(r(0, 19, 0, 23)));
    assert_eq!(hover_at(Q, 1), None);
    assert_eq!(hover_at("{ user(id: 1) { nmae } }", 17), None);
    assert_eq!(hover_at("{ user(id: {{id}}) { name } }", 13), None);
}

#[test]
fn definitions_link_to_docs_pages() {
    assert_eq!(
        link_at(Q, 3),
        Some(("apinata-docs:field/Query/user".into(), r(0, 2, 0, 6)))
    );
    assert_eq!(
        link_at(Q, 8),
        Some(("apinata-docs:field/Query/user".into(), r(0, 7, 0, 9)))
    );
    assert_eq!(
        link_at(Q, 18),
        Some(("apinata-docs:field/User/name".into(), r(0, 16, 0, 20)))
    );
    assert_eq!(
        link_at("{ node(id: 1) { ... on User { id } } }", 24),
        Some(("apinata-docs:type/User".into(), r(0, 23, 0, 27)))
    );
    assert_eq!(link_at(Q, 14), None);
}

#[test]
fn docs_uris_parse_back_to_views() {
    assert_eq!(
        docs_view("apinata-docs:type/User"),
        Some(DocsView::Type("User".into()))
    );
    assert_eq!(
        docs_view("apinata-docs:field/User/name"),
        Some(DocsView::Field {
            owner: "User".into(),
            field: "name".into()
        })
    );
    for bad in [
        "apinata-docs:type/",
        "apinata-docs:field/User",
        "apinata-docs:field/User/",
        "apinata-docs:field/User/name/x",
        "apinata-docs:other/User",
        "https://example.com/type/User",
        "file:///type/User",
    ] {
        assert_eq!(docs_view(bad), None, "{bad}");
    }
}

#[gpui_kit::test]
fn the_providers_answer_only_while_the_schema_is_loaded(cx: &mut TestAppContext) {
    init(cx);
    let url = "http://x.test/g";
    let symbols = SchemaSymbols::new(Rc::new(RefCell::new(url.to_owned())));
    let rope = Rope::from(Q);
    let answer = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            (
                symbols.hover_answer(&rope, 18, cx).is_some(),
                symbols.definition_answer(&rope, 18, cx).len(),
            )
        })
    };
    assert_eq!(answer(cx), (false, 0));
    let generation = cx.update(|cx| cx.default_global::<SchemaStore>().begin(url));
    assert_eq!(answer(cx), (false, 0), "nothing while loading");
    cx.update(|cx| {
        cx.default_global::<SchemaStore>()
            .finish(url, generation, Ok(schema()));
    });
    assert_eq!(answer(cx), (true, 1));
    cx.update(|cx| {
        let store = cx.default_global::<SchemaStore>();
        let g = store.begin(url);
        store.finish(url, g, Err("boom".into()));
    });
    assert_eq!(answer(cx), (false, 0), "nothing after a failed load");
}

// A one-field schema whose texts carry markup, so hover must show them as plain text.
fn hostile() -> Schema {
    let text = r#"{"data":{"__schema":{"queryType":{"name":"Query"},"types":[
      {"kind":"OBJECT","name":"Query","description":null,"interfaces":[],"fields":[
        {"name":"f","description":"![x](https://h/x.png) <img src=x> **bold** [link](u)",
         "args":[
           {"name":"a","description":null,"type":{"kind":"SCALAR","name":"String"},"defaultValue":"a`b\nc"},
           {"name":"b","description":null,"type":{"kind":"SCALAR","name":"String"},"defaultValue":"x`"}],
         "type":{"kind":"SCALAR","name":"String"},"isDeprecated":false,"deprecationReason":null},
        {"name":"g","description":"Line one\nline two\n\n    indented\n\tcode\nlast","args":[],
         "type":{"kind":"SCALAR","name":"String"},"isDeprecated":true,"deprecationReason":"Use <b>f</b>"}]},
      {"kind":"SCALAR","name":"String","description":null}]}}}"#;
    Schema::from_json(text).unwrap()
}

fn hostile_hover(text: &str, offset: usize) -> String {
    let h = hover(&hostile(), &Rope::from(text), offset).expect("a hover");
    let HoverContents::Markup(m) = h.contents else {
        panic!("not markup")
    };
    m.value
}

#[test]
fn schema_text_is_escaped_to_plain_text() {
    let md = hostile_hover("{ f(a: 1, b: 2) }", 3);
    assert!(
        md.ends_with(
            "\n\n\\!\\[x\\]\\(https\\:\\/\\/h\\/x\\.png\\) \\<img src\\=x\\> \\*\\*bold\\*\\* \\[link\\]\\(u\\)"
        ),
        "{md}"
    );
}

#[test]
fn description_comes_before_deprecation_and_breaks_lines_hard_and_drops_leading_whitespace() {
    assert_eq!(
        hostile_hover("{ g }", 2),
        "`Query.g: String`\n\nLine one\\\nline two\n\nindented\\\ncode\\\nlast\n\nDeprecated: Use \\<b\\>f\\<\\/b\\>"
    );
}

#[test]
fn signature_code_spans_survive_backticks_and_newlines() {
    assert_eq!(
        hostile_hover("{ f(a: 1, b: 2) }", 4),
        "``a: String = a`b c``"
    );
    assert_eq!(
        hostile_hover("{ f(a: 1, b: 2) }", 10),
        "`` b: String = x` ``"
    );
}

#[test]
fn a_query_over_the_size_limit_has_no_hover_or_link() {
    let big = format!(
        "{{ user {{ id }} }}{}",
        " ".repeat(apinata_core::QUERY_VALIDATE_LIMIT)
    );
    let rope = Rope::from(big.as_str());
    assert!(hover(&schema(), &rope, 3).is_none());
    assert!(definition(&schema(), &rope, 3).is_none());
}
