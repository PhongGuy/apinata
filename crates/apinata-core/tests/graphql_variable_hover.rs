//! `variable_hover`: a variables key explained from the query, then the schema.

use apinata_core::{QUERY_VALIDATE_LIMIT, Schema, VariableHover, variable_hover};

const Q_FIX: &str =
    "mutation M($input: NewUser!, $id: ID!, $first: Int = 10) { addUser(input: $input) { id } }";
const Q_SDL: &str = "query Q($filter: UserFilter) { users(filter: $filter) }";
const SDL: &str = r#"
type Query { users(filter: UserFilter): [String] }
input UserFilter {
  "Exact name match."
  name: String
  role: Role = MEMBER
  and: [UserFilter!]
}
enum Role { ADMIN MEMBER }
"#;

fn fixture() -> Schema {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/graphql/schema.json"
    ))
    .unwrap();
    Schema::from_json(&text).unwrap()
}

fn sdl() -> Schema {
    Schema::from_sdl(SDL).unwrap()
}

type Shown = (
    std::ops::Range<usize>,
    String,
    Option<String>,
    Option<String>,
);

fn shown(h: Option<VariableHover>) -> Option<Shown> {
    h.map(|h| (h.range, h.signature, h.description, h.default))
}

#[test]
fn a_top_level_key_shows_its_definition_without_a_schema() {
    assert_eq!(
        shown(variable_hover(Q_FIX, "{\"id\": 1}", 2, None)),
        Some((1..5, "$id: ID!".to_owned(), None, None))
    );
    let first = Some((1..8, "$first: Int".to_owned(), None, Some("10".to_owned())));
    assert_eq!(
        shown(variable_hover(Q_FIX, "{\"first\": 3}", 1, None)),
        first
    );
    assert_eq!(
        shown(variable_hover(Q_FIX, "{\"first\": 3}", 7, None)),
        first
    );
    assert_eq!(variable_hover(Q_FIX, "{\"first\": 3}", 8, None), None);
}

#[test]
fn a_nested_key_shows_its_input_field() {
    let d = sdl();
    assert_eq!(
        shown(variable_hover(
            Q_SDL,
            "{\"filter\": {\"name\": \"x\"}}",
            14,
            Some(&d)
        )),
        Some((
            12..18,
            "UserFilter.name: String".to_owned(),
            Some("Exact name match.".to_owned()),
            None
        ))
    );
    assert_eq!(
        shown(variable_hover(
            Q_SDL,
            "{\"filter\": {\"and\": [{\"name\": null}]}}",
            22,
            Some(&d)
        )),
        Some((
            21..27,
            "UserFilter.name: String".to_owned(),
            Some("Exact name match.".to_owned()),
            None
        ))
    );
    let s = fixture();
    assert_eq!(
        shown(variable_hover(
            Q_FIX,
            "{\"input\": {\"role\": \"ADMIN\"}}",
            12,
            Some(&s)
        )),
        Some((
            11..17,
            "NewUser.role: Role".to_owned(),
            None,
            Some("MEMBER".to_owned())
        ))
    );
    assert_eq!(
        variable_hover(Q_FIX, "{\"input\": {\"role\": \"ADMIN\"}}", 12, None),
        None
    );
}

#[test]
fn nothing_off_known_keys() {
    let d = sdl();
    assert_eq!(variable_hover(Q_FIX, "{\"nope\": 1}", 2, None), None);
    assert_eq!(variable_hover(Q_FIX, "{\"id\": 1}", 7, None), None);
    assert_eq!(
        variable_hover(Q_SDL, "{\"filter\": {\"nope\": 1}}", 14, Some(&d)),
        None
    );
    assert_eq!(variable_hover(Q_FIX, "{\"{{k}}\": 1}", 3, None), None);
    assert_eq!(variable_hover("{ a }", "{\"id\": 1}", 2, None), None);
    let big = format!("{{\"id\": 1}}{}", " ".repeat(QUERY_VALIDATE_LIMIT));
    assert_eq!(variable_hover(Q_FIX, &big, 2, None), None);
    assert_eq!(variable_hover(Q_FIX, "{\"\u{f8}\": 1}", 3, None), None);
}
