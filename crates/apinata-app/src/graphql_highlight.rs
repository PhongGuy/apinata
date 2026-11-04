//! GraphQL highlighting: the kit links the grammar with an empty highlights query, so the app
//! registers a vendored one in its place.

use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};

/// The vendored highlights query, attributed in its header.
pub const HIGHLIGHTS: &str = include_str!("graphql_highlights.scm");

/// Registers `graphql` with the vendored query; safe to call more than once.
pub fn init() {
    let graphql = LanguageConfig::new(
        "graphql",
        tree_sitter_graphql::LANGUAGE.into(),
        Vec::new(),
        HIGHLIGHTS,
        "",
        "",
    );
    LanguageRegistry::singleton().register("graphql", &graphql);
}
