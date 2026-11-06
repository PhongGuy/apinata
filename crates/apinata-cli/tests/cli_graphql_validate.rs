//! `apinata graphql validate` with and without a schema, its line format, cap and escaping.

use crate::common;

use common::{ROOT, apinata, write_tree};
use predicates::str::contains;

use crate::cli_graphql_fmt::request;

const SDL: &str = "type Query { me: User users(filter: Filter): [User] }\ntype User { id: ID! name: String old: String @deprecated(reason: \"use name\") }\ninput Filter { name: String }\n";
const JSON: &str = r#"{"data":{"__schema":{"queryType":{"name":"Query"},"mutationType":null,"subscriptionType":null,"types":[{"kind":"OBJECT","name":"Query","fields":[{"name":"me","args":[],"type":{"kind":"SCALAR","name":"String"},"isDeprecated":false}]},{"kind":"SCALAR","name":"String"}]}}}"#;

#[test]
fn validate_without_a_schema_checks_syntax() {
    let dir = write_tree(&[
        ("ok.graphql", "{ me { nmae } }"),
        ("bad.graphql", "{ me { id }"),
    ]);
    apinata()
        .args(["graphql", "validate"])
        .arg(dir.path().join("ok.graphql"))
        .assert()
        .code(0)
        .stdout("");
    let bad = dir.path().join("bad.graphql");
    apinata()
        .args(["graphql", "validate"])
        .arg(&bad)
        .assert()
        .code(1)
        .stdout(format!(
            "{}:1:12: error: syntax error: expected R_CURLY, got EOF\n",
            bad.display()
        ));
}

#[test]
fn validate_with_an_sdl_schema() {
    let dir = write_tree(&[
        ("s.graphql", SDL),
        ("q.graphql", "{ me { nmae old } }"),
        ("w.graphql", "{ me { old } }"),
    ]);
    let (q, w) = (dir.path().join("q.graphql"), dir.path().join("w.graphql"));
    apinata().args(["graphql", "validate"]).arg(&q).arg("--schema-file").arg(dir.path().join("s.graphql")).assert().code(1)
        .stdout(format!(
            "{q}:1:8: error: type `User` does not have a field `nmae`\n{q}:1:13: warning: `User.old` is deprecated: use name\n",
            q = q.display()
        ));
    apinata()
        .args(["graphql", "validate"])
        .arg(&w)
        .arg("--schema-file")
        .arg(dir.path().join("s.graphql"))
        .assert()
        .code(0)
        .stdout(contains("warning: `User.old` is deprecated"));
}

#[test]
fn validate_with_an_introspection_schema() {
    let dir = write_tree(&[
        ("s.json", JSON),
        ("q.graphql", "{ you }"),
        ("ok.graphql", "{ me }"),
    ]);
    let q = dir.path().join("q.graphql");
    apinata()
        .args(["graphql", "validate"])
        .arg(&q)
        .arg("--schema-file")
        .arg(dir.path().join("s.json"))
        .assert()
        .code(1)
        .stdout(format!(
            "{}:1:3: error: type `Query` does not have a field `you`\n",
            q.display()
        ));
    apinata()
        .args(["graphql", "validate"])
        .arg(dir.path().join("ok.graphql"))
        .arg("--schema-file")
        .arg(dir.path().join("s.json"))
        .assert()
        .code(0)
        .stdout("");
}

#[test]
fn validate_a_request_counts_columns_in_chars_of_the_query() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("s.graphql", SDL),
        (
            "g.yml",
            &request("{ users(filter: {name: \"ø\", nmae: \"a\"}) { id } }"),
        ),
    ]);
    let g = dir.path().join("g.yml");
    apinata()
        .args(["graphql", "validate"])
        .arg(&g)
        .arg("--schema-file")
        .arg(dir.path().join("s.graphql"))
        .assert()
        .code(1)
        .stdout(format!(
            "{} (query) 1:35: error: field `nmae` does not exist on `Filter`\n",
            g.display()
        ));
}

#[test]
fn validate_refuses_unusable_inputs_with_exit_3() {
    let dir = write_tree(&[("q.graphql", "{ me }"), ("bad.json", "{ not json")]);
    let q = dir.path().join("q.graphql");
    apinata()
        .args(["graphql", "validate"])
        .arg(&q)
        .arg("--schema-file")
        .arg(dir.path().join("missing.graphql"))
        .assert()
        .code(3);
    apinata()
        .args(["graphql", "validate"])
        .arg(&q)
        .arg("--schema-file")
        .arg(dir.path().join("bad.json"))
        .assert()
        .code(3);
    apinata()
        .args(["graphql", "validate"])
        .arg(dir.path().join("missing.graphql"))
        .assert()
        .code(3);
}

#[test]
fn validate_prints_two_hundred_lines_and_a_summary() {
    let dir = write_tree(&[("q.graphql", &"} ".repeat(300))]);
    let q = dir.path().join("q.graphql");
    let out = apinata()
        .args(["graphql", "validate"])
        .arg(&q)
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 201);
    assert_eq!(
        lines[200],
        format!("{}:1:1: info: and 100 more", q.display())
    );
}

#[test]
fn control_characters_in_messages_are_replaced() {
    let dir = write_tree(&[("q.graphql", "{ me \u{7} }")]);
    let out = apinata()
        .args(["graphql", "validate"])
        .arg(dir.path().join("q.graphql"))
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("\u{FFFD}") && !text.contains('\u{7}'),
        "{text:?}"
    );
}
