//! `variable_completions`: keys from the query and the schema, values from enums and scalars.

use apinata_core::{
    QUERY_VALIDATE_LIMIT, Schema, VariableCompletion, VariableCompletionKind, variable_completions,
};

const Q_FIX: &str = "mutation M($input: NewUser!, $role: Role, $id: ID!, $on: Boolean, $must: Boolean!) { addUser(input: $input) { id } }";
const Q_SDL: &str =
    "query Q($filter: UserFilter, $roles: [Role!]!, $grid: [[Int]]) { users(filter: $filter) }";
const SDL: &str = r#"
type Query { users(filter: UserFilter): [String] }
"Narrows a user search."
input UserFilter {
  "Exact name match."
  name: String
  role: Role = MEMBER
  roles: [Role!]
  active: Boolean!
  level: Level
  address: Address
  and: [UserFilter!]
}
input Address { city: String! zip: String }
enum Role {
  "Full access."
  ADMIN
  MEMBER
  GUEST @deprecated(reason: "Gone")
}
enum Level { OLD @deprecated LOW HIGH }
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

/// The completions at the `|` in `marked`, which is removed.
fn complete(query: &str, marked: &str, schema: Option<&Schema>) -> Vec<VariableCompletion> {
    let offset = marked.find('|').expect("a cursor");
    variable_completions(query, &marked.replacen('|', "", 1), offset, schema)
}

fn labels(query: &str, marked: &str, schema: Option<&Schema>) -> Vec<String> {
    complete(query, marked, schema)
        .into_iter()
        .map(|c| c.label)
        .collect()
}

#[test]
fn top_level_keys_come_from_the_query_without_a_schema() {
    let all = complete(Q_FIX, "{\"|\"}", None);
    assert_eq!(
        all.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        ["input", "role", "id", "on", "must"]
    );
    let first = &all[0];
    assert_eq!(first.insert, "\"input\": ");
    assert_eq!(first.range, 1..3);
    assert_eq!(first.kind, VariableCompletionKind::Variable);
    assert_eq!(first.detail.as_deref(), Some("NewUser!"));
    assert_eq!(first.documentation, None);
}

#[test]
fn bare_letters_and_case_are_matched_and_a_following_colon_is_kept() {
    let bare = complete(Q_FIX, "{i|", None);
    assert_eq!(
        bare.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        ["input", "id"]
    );
    assert_eq!(
        (bare[0].range.clone(), bare[0].insert.as_str()),
        (1..2, "\"input\": ")
    );
    assert_eq!(labels(Q_FIX, "{\"I|\"}", None), ["input", "id"]);
    let colon = complete(Q_FIX, "{\"i|\": 1}", None);
    assert_eq!(
        (colon[1].range.clone(), colon[1].insert.as_str()),
        (1..4, "\"id\"")
    );
}

#[test]
fn present_keys_are_left_out_but_the_typed_one_stays() {
    assert_eq!(
        labels(Q_FIX, "{\"id\": 1, \"|\"}", None),
        ["input", "role", "on", "must"]
    );
    assert_eq!(
        labels(Q_FIX, "{\"|\", \"on\": true}", None),
        ["input", "role", "id", "must"]
    );
    assert_eq!(labels(Q_FIX, "{\"id|\": 1}", None), ["id"]);
}

#[test]
fn nested_keys_are_input_fields_and_need_the_schema() {
    let s = fixture();
    let all = complete(Q_FIX, "{\"input\": {\"|\"}}", Some(&s));
    assert_eq!(
        all.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        ["name", "role"]
    );
    assert_eq!(all[0].kind, VariableCompletionKind::InputField);
    assert_eq!(all[0].detail.as_deref(), Some("String!"));
    assert_eq!(all[0].insert, "\"name\": ");
    assert!(complete(Q_FIX, "{\"input\": {\"|\"}}", None).is_empty());
    let d = sdl();
    let name = &complete(Q_SDL, "{\"filter\": {\"n|\"}}", Some(&d))[0];
    assert_eq!(name.documentation.as_deref(), Some("Exact name match."));
    assert_eq!(
        labels(Q_SDL, "{\"filter\": {\"address\": {\"|\"}}}", Some(&d)),
        ["city", "zip"]
    );
}

#[test]
fn a_recursive_input_follows_the_text() {
    let d = sdl();
    let all = complete(
        Q_SDL,
        "{\"filter\": {\"and\": [{\"and\": [{\"addr|\"}]}]}}",
        Some(&d),
    );
    assert_eq!(all.len(), 1);
    assert_eq!(
        (all[0].label.as_str(), all[0].detail.as_deref()),
        ("address", Some("Address"))
    );
}

#[test]
fn enum_values_are_strings_with_deprecated_ones_last() {
    let s = fixture();
    let all = complete(Q_FIX, "{\"role\": |}", Some(&s));
    assert_eq!(
        all.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        ["ADMIN", "MEMBER", "GUEST", "null"]
    );
    let admin = &all[0];
    assert_eq!(admin.insert, "\"ADMIN\"");
    assert_eq!(admin.range, 9..9);
    assert_eq!(admin.kind, VariableCompletionKind::EnumValue);
    assert_eq!(admin.detail.as_deref(), Some("Role"));
    assert_eq!(admin.documentation.as_deref(), Some("Full access."));
    assert_eq!(all[2].detail.as_deref(), Some("deprecated"));
    assert_eq!(all[3].kind, VariableCompletionKind::Literal);
    let typed = complete(Q_FIX, "{\"role\": \"me|\"}", Some(&s));
    assert_eq!(typed.len(), 1);
    assert_eq!(
        (typed[0].insert.as_str(), typed[0].range.clone()),
        ("\"MEMBER\"", 9..13)
    );
    let d = sdl();
    assert_eq!(
        labels(Q_SDL, "{\"filter\": {\"level\": |}}", Some(&d)),
        ["LOW", "HIGH", "OLD", "null"]
    );
}

#[test]
fn booleans_and_null_follow_the_type() {
    for schema in [None, Some(fixture())] {
        let s = schema.as_ref();
        assert_eq!(labels(Q_FIX, "{\"on\": |}", s), ["true", "false", "null"]);
        assert_eq!(labels(Q_FIX, "{\"must\": |}", s), ["true", "false"]);
        assert!(labels(Q_FIX, "{\"id\": |}", s).is_empty());
        let t = complete(Q_FIX, "{\"on\": t|}", s);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].label.as_str(), t[0].range.clone()), ("true", 7..8));
    }
    assert_eq!(labels(Q_FIX, "{\"role\": |}", None), ["null"]);
}

#[test]
fn lists_offer_their_element_type_inside_brackets() {
    let d = sdl();
    assert_eq!(
        labels(Q_SDL, "{\"roles\": [|]}", Some(&d)),
        ["ADMIN", "MEMBER", "GUEST"]
    );
    assert!(labels(Q_SDL, "{\"roles\": |}", Some(&d)).is_empty());
    assert_eq!(labels(Q_SDL, "{\"grid\": [|]}", None), ["null"]);
    assert_eq!(labels(Q_SDL, "{\"grid\": [[|]]}", None), ["null"]);
}

#[test]
fn nothing_off_the_query_or_the_schema() {
    let d = sdl();
    for (query, marked) in [
        (Q_SDL, "{\"nope\": |}"),
        (Q_SDL, "{\"filter\": {\"nope\": |}}"),
        (Q_SDL, "{\"filter\": {\"name\": {\"|\"}}}"),
        (Q_SDL, "{\"roles\": {\"|\"}}"),
        (Q_SDL, "[|]"),
        (Q_SDL, "{\"filter\": {{f|}}}"),
        ("query ($x: Mystery) { a }", "{\"x\": |}"),
        ("{ a }", "{\"|\"}"),
        ("", "{\"|\"}"),
    ] {
        assert!(
            complete(query, marked, Some(&d)).is_empty(),
            "{query:?} {marked:?}"
        );
    }
    let big = format!("{{\"\"}}{}", " ".repeat(QUERY_VALIDATE_LIMIT));
    assert!(variable_completions(Q_FIX, &big, 2, None).is_empty());
    let big_query = format!("{Q_FIX}{}", " ".repeat(QUERY_VALIDATE_LIMIT));
    assert!(variable_completions(&big_query, "{\"\"}", 2, None).is_empty());
    assert!(variable_completions(Q_FIX, "{\"\u{f8}\": 1}", 3, None).is_empty());
}
