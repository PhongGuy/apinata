//! The docs explorer: pages for the root, a type and a field; navigation and the deprecated toggle.

use crate::support;

use apinata_app::graphql_docs::{DocRow, DocsPanel, DocsView, page};
use apinata_app::graphql_schema_store::SchemaStore;
use apinata_core::Schema;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn schema() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}
fn h(s: &str) -> DocRow {
    DocRow::Heading(s.into())
}
fn ty(label: &str, to: &str) -> DocRow {
    DocRow::Link {
        label: label.into(),
        to: DocsView::Type(to.into()),
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
fn plain(s: &str) -> DocRow {
    DocRow::Plain(s.into())
}

#[test]
fn the_root_page_lists_roots_and_search_results() {
    let s = schema();
    let root = page(&s, &DocsView::Root, "", false);
    assert_eq!(root.title, "Schema");
    assert_eq!(
        root.rows,
        [
            h("Root types"),
            ty("query: Query", "Query"),
            ty("mutation: Mutation", "Mutation")
        ]
    );
    let found = page(&s, &DocsView::Root, "US", false);
    assert_eq!(
        found.rows[3..],
        [
            h("Results"),
            ty("User", "User"),
            ty("NewUser", "NewUser"),
            field("Query.user", "Query", "user"),
            field("Query.users", "Query", "users"),
            field("Mutation.addUser", "Mutation", "addUser")
        ]
    );
    let all = page(&s, &DocsView::Root, "us", true);
    assert!(
        all.rows
            .contains(&field("Query.oldUsers (deprecated)", "Query", "oldUsers"))
    );
}

#[test]
fn type_pages_follow_the_kind() {
    let s = schema();
    let user = page(&s, &DocsView::Type("User".into()), "", false);
    assert_eq!(
        (user.title.as_str(), user.description.as_deref()),
        ("User", Some("A person."))
    );
    assert_eq!(
        user.rows,
        [
            h("Fields"),
            field("id: ID!", "User", "id"),
            field("name: String", "User", "name"),
            field("role: Role", "User", "role"),
            field("friends: [User!]", "User", "friends"),
            h("Implements"),
            ty("Node", "Node")
        ]
    );
    let with = page(&s, &DocsView::Type("User".into()), "", true);
    assert_eq!(
        with.rows[3],
        field("legacyName: String (deprecated)", "User", "legacyName")
    );
    let query = page(&s, &DocsView::Type("Query".into()), "", false);
    assert_eq!(
        query.rows[1],
        field("user(id: ID!, active: Boolean): User", "Query", "user")
    );
    assert_eq!(
        page(&s, &DocsView::Type("Role".into()), "", false).rows,
        [h("Values"), plain("ADMIN"), plain("MEMBER")]
    );
    assert_eq!(
        page(&s, &DocsView::Type("Role".into()), "", true).rows[3],
        plain("GUEST (deprecated)")
    );
    assert_eq!(
        page(&s, &DocsView::Type("NewUser".into()), "", false).rows,
        [
            h("Input fields"),
            ty("name: String!", "String"),
            ty("role: Role = MEMBER", "Role")
        ]
    );
    assert_eq!(
        page(&s, &DocsView::Type("SearchResult".into()), "", false).rows,
        [h("Possible types"), ty("User", "User"), ty("Post", "Post")]
    );
    assert_eq!(
        page(&s, &DocsView::Type("Node".into()), "", false).rows,
        [
            h("Fields"),
            field("id: ID!", "Node", "id"),
            h("Possible types"),
            ty("User", "User")
        ]
    );
    assert_eq!(
        page(&s, &DocsView::Type("Nope".into()), "", false).rows,
        [plain("No type named Nope in this schema")]
    );
}

#[test]
fn field_pages_show_arguments_type_and_deprecation() {
    let s = schema();
    let user = page(
        &s,
        &DocsView::Field {
            owner: "Query".into(),
            field: "user".into(),
        },
        "",
        false,
    );
    assert_eq!(
        (user.title.as_str(), user.description.as_deref()),
        ("Query.user", Some("One user by id."))
    );
    assert_eq!(
        user.rows,
        [
            h("Arguments"),
            ty("id: ID!", "ID"),
            ty("active: Boolean = true", "Boolean"),
            h("Type"),
            ty("User", "User")
        ]
    );
    let old = page(
        &s,
        &DocsView::Field {
            owner: "Query".into(),
            field: "oldUsers".into(),
        },
        "",
        true,
    );
    assert_eq!(
        old.rows,
        [
            plain("Deprecated: Use users"),
            h("Type"),
            ty("[User]", "User")
        ]
    );
}

#[gpui_kit::test]
fn the_panel_navigates_goes_back_and_toggles_deprecated(cx: &mut TestAppContext) {
    support::init(cx);
    let url = "http://x.test/g";
    let (win, panel) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| DocsPanel::new(w, cx))
            })
        })
        .unwrap();
    panel.update(cx, |p, cx| p.set_url(url.into(), cx));
    assert!(present(cx, win, "graphql-docs-load"));
    cx.update(|cx| {
        let s = cx.default_global::<SchemaStore>();
        let generation = s.begin(url);
        s.finish(url, generation, Ok(schema()));
    });
    cx.run_until_parked();
    assert!(present(cx, win, "graphql-docs-search"));
    assert!(!present(cx, win, "graphql-docs-back"));
    cx.update_window(win, |_, w, cx| w.click(("graphql-docs-row", 1usize), cx))
        .unwrap();
    assert_eq!(
        panel.read_with(cx, |p, _| p.view().clone()),
        DocsView::Type("Query".into())
    );
    cx.update_window(win, |_, w, cx| w.click("graphql-docs-back", cx))
        .unwrap();
    assert_eq!(panel.read_with(cx, |p, _| p.view().clone()), DocsView::Root);
    cx.update_window(win, |_, w, cx| w.click("graphql-docs-deprecated", cx))
        .unwrap();
    assert!(panel.read_with(cx, |p, _| p.show_deprecated()));
}

#[gpui_kit::test]
fn the_docs_toggle_opens_the_panel_in_the_section(cx: &mut TestAppContext) {
    use apinata_app::graphql_section::GraphqlSection;
    let (win, host) = host_section(cx, GraphqlSection::new);
    assert!(!present(cx, win, "graphql-docs"));
    cx.update_window(win, |_, w, cx| w.click("graphql-docs-toggle", cx))
        .unwrap();
    assert!(present(cx, win, "graphql-docs"));
    let s = host.read_with(cx, |h, _| h.view.clone());
    assert!(s.read_with(cx, |s, _| s.docs_open()));
}
