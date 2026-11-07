//! The docs explorer's directive pages, the root page's Directives section and directive search.

use apinata_app::graphql_docs::{DocRow, DocsView, page};
use apinata_core::Schema;

const SDL: &str = "\"Tags a field.\"\ndirective @tag(\"The tag.\" name: String! = \"x\", level: Int) repeatable on FIELD | QUERY\ndirective @once on FIELD\ntype Query { a: Int once: Int }\nscalar Oncer\n";

fn schema() -> Schema {
    Schema::from_sdl(SDL).unwrap()
}
fn h(s: &str) -> DocRow {
    DocRow::Heading(s.into())
}
fn plain(s: &str) -> DocRow {
    DocRow::Plain(s.into())
}
fn ty(label: &str, to: &str) -> DocRow {
    DocRow::Link {
        label: label.into(),
        to: DocsView::Type(to.into()),
    }
}
fn dir(name: &str) -> DocRow {
    DocRow::Link {
        label: format!("@{name}"),
        to: DocsView::Directive(name.into()),
    }
}
fn field(label: &str, owner: &str, f: &str) -> DocRow {
    DocRow::Link {
        label: label.into(),
        to: DocsView::Field {
            owner: owner.into(),
            field: f.into(),
        },
    }
}

#[test]
fn the_root_page_lists_every_directive_after_the_roots() {
    let root = page(&schema(), &DocsView::Root, "", false);
    assert_eq!(
        root.rows,
        [
            h("Root types"),
            ty("query: Query", "Query"),
            h("Directives"),
            dir("tag"),
            dir("once"),
            dir("skip"),
            dir("include"),
            dir("deprecated"),
            dir("specifiedBy")
        ]
    );
}

#[test]
fn search_lists_directives_between_types_and_fields() {
    let found = page(&schema(), &DocsView::Root, "once", false);
    assert_eq!(
        found.rows[9..],
        [
            h("Results"),
            ty("Oncer", "Oncer"),
            dir("once"),
            field("Query.once", "Query", "once")
        ]
    );
    let at = page(&schema(), &DocsView::Root, "@ta", false);
    assert_eq!(at.rows[9..], [h("Results"), dir("tag")]);
}

#[test]
fn a_directive_page_shows_signature_arguments_locations_and_repeatable() {
    let s = schema();
    let tag = page(&s, &DocsView::Directive("tag".into()), "", false);
    assert_eq!(
        (tag.title.as_str(), tag.description.as_deref()),
        ("@tag", Some("Tags a field."))
    );
    assert_eq!(
        tag.rows,
        [
            plain("@tag(name: String! = \"x\", level: Int) repeatable on FIELD | QUERY"),
            plain("Repeatable"),
            h("Arguments"),
            ty("name: String! = \"x\"", "String"),
            ty("level: Int", "Int"),
            h("Locations"),
            plain("FIELD"),
            plain("QUERY")
        ]
    );
    let once = page(&s, &DocsView::Directive("once".into()), "", false);
    assert_eq!(
        once.rows,
        [plain("@once on FIELD"), h("Locations"), plain("FIELD")]
    );
    let missing = page(&s, &DocsView::Directive("nope".into()), "", false);
    assert_eq!(missing.title, "@nope");
    assert_eq!(missing.rows, [plain("No directive @nope in this schema")]);
}
