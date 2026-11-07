//! `Schema::symbol_at` over the fixture schema: which schema element each name in a query means.

use apinata_core::{QUERY_VALIDATE_LIMIT, Schema, SchemaSymbol};

fn schema() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}

fn ty(name: &str) -> SchemaSymbol {
    SchemaSymbol::Type { name: name.into() }
}

fn field(owner: &str, field: &str) -> SchemaSymbol {
    SchemaSymbol::Field {
        owner: owner.into(),
        field: field.into(),
    }
}

fn arg(owner: &str, field: &str, arg: &str) -> SchemaSymbol {
    SchemaSymbol::Argument {
        owner: owner.into(),
        field: field.into(),
        arg: arg.into(),
    }
}

/// The symbol at the `|` in `marked` (which is removed), with the text its range covers.
fn at(s: &Schema, marked: &str) -> Option<(SchemaSymbol, String)> {
    let offset = marked.find('|').expect("a cursor");
    let query = marked.replacen('|', "", 1);
    s.symbol_at(&query, offset)
        .map(|found| (found.symbol, query[found.range].to_owned()))
}

#[test]
fn names_resolve_to_schema_elements() {
    let s = schema();
    let cases: &[(&str, SchemaSymbol, &str)] = &[
        // Start, middle and end of a name.
        ("{ |user(id: 1) { name } }", field("Query", "user"), "user"),
        ("{ us|er(id: 1) { name } }", field("Query", "user"), "user"),
        ("{ user|(id: 1) { name } }", field("Query", "user"), "user"),
        (
            "{ user(i|d: 1) { name } }",
            arg("Query", "user", "id"),
            "id",
        ),
        (
            "{ user(id: 1, |active: true) { name } }",
            arg("Query", "user", "active"),
            "active",
        ),
        (
            "{ users(role: ADMIN fir|st: 5) { id } }",
            arg("Query", "users", "first"),
            "first",
        ),
        ("{ user(id: 1) { na|me } }", field("User", "name"), "name"),
        // Three levels deep, and back on the outer type after the nested sets close.
        (
            "{ user(id: 1) { friends { friends { ro|le } } } }",
            field("User", "role"),
            "role",
        ),
        (
            "{ user(id: 1) { friends { friends { name } } leg|acyName } }",
            field("User", "legacyName"),
            "legacyName",
        ),
        // A field selected on an interface is owned by the interface.
        ("{ node(id: 1) { i|d } }", field("Node", "id"), "id"),
        // Inline and named fragments.
        (
            "{ search(term: \"x\") { ... on Us|er { name } } }",
            ty("User"),
            "User",
        ),
        (
            "{ search(term: \"x\") { ... on Post { tit|le } } }",
            field("Post", "title"),
            "title",
        ),
        (
            "{ search(term: \"x\") { ... on User { friends { ro|le } } } }",
            field("User", "role"),
            "role",
        ),
        ("fragment F on Us|er { name }", ty("User"), "User"),
        (
            "fragment F on User { friends { na|me } }",
            field("User", "name"),
            "name",
        ),
        // The field behind an alias.
        (
            "{ me: us|er(id: 1) { name } }",
            field("Query", "user"),
            "user",
        ),
        // Variable definition types.
        (
            "query Q($id: I|D!) { user(id: $id) { name } }",
            ty("ID"),
            "ID",
        ),
        (
            "query Q($ids: [I|D!]) { node(id: 1) { id } }",
            ty("ID"),
            "ID",
        ),
        (
            "mutation M($input: NewUser|!) { addUser(input: $input) { id } }",
            ty("NewUser"),
            "NewUser",
        ),
        (
            "mutation { addU|ser(input: {name: \"x\"}) { id } }",
            field("Mutation", "addUser"),
            "addUser",
        ),
        (
            "{ user(id: 1) @sk|ip(if: true) { name } }",
            SchemaSymbol::Directive {
                name: "skip".into(),
            },
            "skip",
        ),
        (
            "{ user(id: 1) @skip(i|f: true) { name } }",
            SchemaSymbol::DirectiveArgument {
                directive: "skip".into(),
                arg: "if".into(),
            },
            "if",
        ),
        // A second operation starts again at the query root.
        (
            "query Q { user(id: 1) { id } }\nquery R { us|ers { id } }",
            field("Query", "users"),
            "users",
        ),
        // Placeholders elsewhere do not disturb the scan.
        (
            "{ user(id: 1) { {{fields}} na|me } }",
            field("User", "name"),
            "name",
        ),
        (
            "{ user(id: {{id}}) { na|me } }",
            field("User", "name"),
            "name",
        ),
        // Half-written queries.
        ("{ user(id: 1) { na|me", field("User", "name"), "name"),
        (
            "query Q($id: ID!) { user(id: $id) { friends { ro|le",
            field("User", "role"),
            "role",
        ),
        // Multibyte text before the name, and a directive before the selection.
        (
            "# 日本語\n{ us|er(id: 1) { name } }",
            field("Query", "user"),
            "user",
        ),
        (
            "{ user(id: \"日本\") { na|me } }",
            field("User", "name"),
            "name",
        ),
        (
            "{ user(id: 1) @include(if: true) { na|me } }",
            field("User", "name"),
            "name",
        ),
    ];
    for (marked, want, text) in cases {
        assert_eq!(
            at(&s, marked),
            Some((want.clone(), (*text).to_owned())),
            "{marked}"
        );
    }
}

#[test]
fn everything_else_finds_nothing() {
    let s = schema();
    let cases = [
        // Whitespace and punctuation.
        "{| user }",
        "{ user |}",
        "{ user(id: 1)| { name } }",
        "{ user(id: |1) { name } }",
        // Aliases, also across a line break, a comma or a comment.
        "{ m|e: user(id: 1) { name } }",
        "{ m|e : user(id: 1) { id } }",
        "{ m|e\n  : user(id: 1) { id } }",
        "{ m|e, : user(id: 1) { id } }",
        "{ m|e # alias\n : user(id: 1) { id } }",
        // Variables, fragment names, keywords and operation names.
        "query Q($i|d: ID!) { user(id: $id) { name } }",
        "query Q($id: ID!) { user(id: $i|d) { name } }",
        "fragment |F on User { name }",
        "{ user(id: 1) { ...F| } }",
        "{ search(term: \"x\") { ... o|n User { id } } }",
        "query Us|er { user(id: 1) { id } }",
        "que|ry { user(id: 1) { id } }",
        // Argument values, enum values, defaults and input-object fields.
        "{ users(role: ADM|IN) { id } }",
        "{ users(role: [ADM|IN]) { id } }",
        "{ users(first: 1|0) { id } }",
        "query Q($r: Role = ADM|IN) { users(role: $r) { id } }",
        "mutation { addUser(input: {na|me: \"x\"}) { id } }",
        "mutation { addUser(input: {name: \"x\", role: ADM|IN}) { id } }",
        // Strings and comments.
        "{ user(id: \"na|me\") { id } }",
        "{ user(id: \"\"\"na|me\"\"\") { id } }",
        "{ # us|er\n user(id: 1) { id } }",
        // Introspection names.
        "{ user(id: 1) { __type|name } }",
        "{ __sch|ema { types { name } } }",
        "{ ... on __Ty|pe { name } }",
        // Names the schema does not know.
        "{ user(id: 1) { nm|ae } }",
        "{ user(id: 1) { name(fo|o: 1) } }",
        "{ search(term: \"x\") { ti|tle } }",
        "{ search(term: \"x\") { ... on No|pe { id } } }",
        // Inside a placeholder, edges included.
        "{ user(id: {{i|d}}) { name } }",
        "{ user(id: 1) { {{fie|lds}} } }",
        "{ user(id: 1) { |{{fields}} } }",
        "{ user(id: 1) { {{fields}}| } }",
    ];
    for marked in cases {
        assert_eq!(at(&s, marked), None, "{marked}");
    }
}

#[test]
fn ranges_are_bytes_and_bad_offsets_find_nothing() {
    let s = schema();
    // `# 日本\n` is 9 bytes, so `user` sits at 11..15.
    let q = "# 日本\n{ user(id: 1) { name } }";
    assert_eq!(s.symbol_at(q, 13).map(|f| f.range), Some(11..15));
    assert_eq!(s.symbol_at(q, 3), None, "inside 日, not a char boundary");
    assert_eq!(s.symbol_at(q, q.len() + 1), None);
    let end = s.symbol_at("{ user", 6).expect("a name ending the text");
    assert_eq!((end.symbol, end.range), (field("Query", "user"), 2..6));
}

#[test]
fn queries_over_the_validate_limit_find_nothing() {
    let s = schema();
    let at_limit = format!("{{ user {}}}", " ".repeat(QUERY_VALIDATE_LIMIT - 8));
    assert_eq!(at_limit.len(), QUERY_VALIDATE_LIMIT);
    assert!(s.symbol_at(&at_limit, 3).is_some());
    let over = format!("{at_limit} ");
    assert!(s.symbol_at(&over, 3).is_none());
}
