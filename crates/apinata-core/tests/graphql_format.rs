//! `format_query`: the canonical layout, placeholders, strings, limits and the two invariants.

use apinata_core::{QUERY_FORMAT_LIMIT, QuerySyntaxError, format_query, placeholders};
use apollo_parser::cst::CstNode;
use apollo_parser::{Parser, SyntaxKind};

/// `text` with every placeholder overwritten by underscores, as the formatter parses it.
fn masked(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    for (range, _) in placeholders(text) {
        bytes[range].fill(b'_');
    }
    String::from_utf8(bytes).expect("utf-8")
}

/// The significant tokens of `text` as written: commas and whitespace dropped, a comment
/// without its trailing whitespace.
fn significant(text: &str) -> Vec<String> {
    let tree = Parser::new(&masked(text)).parse();
    tree.document()
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| {
            !matches!(
                t.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::COMMA | SyntaxKind::EOF
            )
        })
        .map(|t| {
            let r = t.text_range();
            let s = &text[usize::from(r.start())..usize::from(r.end())];
            if t.kind() == SyntaxKind::COMMENT {
                s.trim_end().to_owned()
            } else {
                s.to_owned()
            }
        })
        .collect()
}

/// `input` formats to `expected`, which is a fixed point, parses cleanly and keeps the tokens.
pub(crate) fn assert_canonical(input: &str, expected: &str) {
    let once = format_query(input).unwrap_or_else(|e| panic!("{e}\n{input}"));
    assert_eq!(once, expected, "input:\n{input}");
    assert_eq!(
        format_query(&once).unwrap(),
        once,
        "not idempotent:\n{once}"
    );
    assert_eq!(
        Parser::new(&masked(&once)).parse().errors().count(),
        0,
        "{once}"
    );
    assert_eq!(
        significant(input),
        significant(&once),
        "tokens changed:\n{once}"
    );
}

#[test]
fn an_operation_gets_one_selection_per_line() {
    assert_canonical(
        "query Q($id: ID!, $n: Int = 10) { user(id: $id) { name, friends(first: $n) { id } } }",
        "query Q($id: ID!, $n: Int = 10) {\n  user(id: $id) {\n    name\n    friends(first: $n) {\n      id\n    }\n  }\n}\n",
    );
}

#[test]
fn arguments_lists_and_objects_stay_on_one_line_with_regular_separators() {
    assert_canonical(
        "{a(x:1 y : [1 2] , o:{k:\"v\",l:[]}) @include(if:true)}",
        "{\n  a(x: 1, y: [1, 2], o: {k: \"v\", l: []}) @include(if: true)\n}\n",
    );
}

#[test]
fn fragments_and_definitions_are_separated_by_one_blank_line() {
    assert_canonical(
        "fragment F on User { id }\n\n\n\nquery { ...F ... on User { name } ... @skip(if: false) { id } }",
        "fragment F on User {\n  id\n}\n\nquery {\n  ...F\n  ... on User {\n    name\n  }\n  ... @skip(if: false) {\n    id\n  }\n}\n",
    );
    assert_canonical("{ a } { b }", "{\n  a\n}\n\n{\n  b\n}\n");
}

#[test]
fn mutations_subscriptions_aliases_and_variable_types() {
    assert_canonical(
        "mutation M($a: [Int!]! @d, $b: In = {x: 1}) { add(a: $a) { ok } }\nsubscription { n: onNew { id } }",
        "mutation M($a: [Int!]! @d, $b: In = {x: 1}) {\n  add(a: $a) {\n    ok\n  }\n}\n\nsubscription {\n  n: onNew {\n    id\n  }\n}\n",
    );
}

#[test]
fn placeholders_come_back_verbatim_in_every_position() {
    assert_canonical(
        "{ user(id: {{id}}, name: \"{{n}}\") { {{fields}} name{{x}} } }",
        "{\n  user(id: {{id}}, name: \"{{n}}\") {\n    {{fields}}\n    name{{x}}\n  }\n}\n",
    );
}

#[test]
fn strings_and_block_strings_are_copied_byte_for_byte() {
    assert_canonical(
        "{ a(t: \"\"\"x\n   y  \n\"\"\", s: \"  # not a comment \") }",
        "{\n  a(t: \"\"\"x\n   y  \n\"\"\", s: \"  # not a comment \")\n}\n",
    );
}

#[test]
fn crlf_input_comes_out_with_lf() {
    assert_canonical("{\r\n  a\r\n}\r\n", "{\n  a\n}\n");
}

#[test]
fn blank_and_comment_only_input_is_returned_unchanged() {
    for text in ["", "  \n", ", ,\n", "# only a comment\n", "  # a\n\n# b"] {
        assert_eq!(format_query(text).unwrap(), text);
    }
}

#[test]
fn a_syntax_error_is_the_first_one_with_its_range() {
    assert_eq!(
        format_query("{ a"),
        Err(QuerySyntaxError::Syntax {
            range: 3..3,
            message: "expected R_CURLY, got EOF".into()
        })
    );
    let err = format_query("query { a(x: 1 }").unwrap_err();
    assert!(
        matches!(&err, QuerySyntaxError::Syntax { range, .. } if *range == (15..16)),
        "{err:?}"
    );
    assert_eq!(err.to_string(), "syntax error: expected R_PAREN, got }");
}

#[test]
fn input_over_the_limit_is_refused_before_parsing() {
    let text = "{ a }".to_owned() + &" ".repeat(QUERY_FORMAT_LIMIT);
    let err = format_query(&text).unwrap_err();
    assert_eq!(
        err,
        QuerySyntaxError::TooLarge {
            limit: QUERY_FORMAT_LIMIT
        }
    );
    assert_eq!(
        err.to_string(),
        "the query is larger than the 4 MiB format limit"
    );
}

#[test]
fn deep_nesting_is_a_syntax_error_without_overflow() {
    let deep = format!("{}{}", "{a".repeat(3000), "}".repeat(3000));
    let got = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || format_query(&deep))
        .unwrap()
        .join()
        .unwrap();
    assert!(
        matches!(&got, Err(QuerySyntaxError::Syntax { message, .. }) if message == "parser recursion limit reached"),
        "{got:?}"
    );
}
