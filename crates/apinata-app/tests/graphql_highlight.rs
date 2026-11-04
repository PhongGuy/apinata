//! The vendored GraphQL highlights query is registered, compiles against the grammar and paints.

use apinata_app::graphql_highlight::{HIGHLIGHTS, init};
use gpui_kit::component::Rope;
use gpui_kit::component::highlighter::{HighlightTheme, LanguageRegistry, SyntaxHighlighter};

#[test]
fn the_registry_returns_the_vendored_query() {
    init();
    let config = LanguageRegistry::singleton()
        .language("graphql")
        .expect("graphql is registered");
    assert_eq!(&*config.highlights, HIGHLIGHTS);
    assert!(HIGHLIGHTS.contains("(comment) @comment"));
}

#[test]
fn the_query_compiles_and_paints_a_document() {
    init();
    // A query that fails to compile makes the kit fall back to plain "text".
    let code = "# who\nquery Q($id: ID!) { user(id: $id) { name } }\n";
    let mut h = SyntaxHighlighter::new("graphql");
    assert_eq!(h.language().as_ref(), "graphql");
    h.update(None, &Rope::from(code), None);
    let theme = HighlightTheme::default_dark();
    let styles = h.styles(&(0..code.len()), &*theme);
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "graphql paints nothing: {styles:?}"
    );
}
