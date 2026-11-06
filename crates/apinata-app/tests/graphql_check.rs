//! The pure parts of live query checks: job numbering, positions, the status label.

use std::sync::Arc;

use apinata_app::graphql_check::{CheckBasis, Jobs, editor_diagnostics, syntax_message};
use apinata_app::graphql_schema_store::Lookup;
use apinata_core::{
    CompiledSchema, QUERY_FORMAT_LIMIT, QUERY_VALIDATE_LIMIT, SCHEMA_SKIP_NOTE, Schema, SchemaUse,
    check_query, format_query, validate_query,
};
use gpui_kit::base::input::DiagnosticSeverity;
use gpui_kit::component::input::{Position, Rope};

const SDL: &str = "type Query { me(s: String): User }\ntype User { name: String }\n";

fn compiled() -> CompiledSchema {
    CompiledSchema::new(&Schema::from_sdl(SDL).unwrap()).unwrap()
}

#[test]
fn only_the_latest_job_counts() {
    let mut jobs = Jobs::default();
    let a = jobs.start();
    let b = jobs.start();
    assert!(!jobs.is_latest(a));
    assert!(jobs.is_latest(b));
}

#[test]
fn ranges_convert_to_char_positions_on_multi_byte_text() {
    let c = compiled();
    let text = "{ me(s: \"日本\") { nmae } }";
    let found = validate_query(text, Some(&c));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].range(), 20..24);
    let got = editor_diagnostics(&Rope::from(text), &found);
    assert_eq!(got[0].range, Position::new(0, 16)..Position::new(0, 20));
    assert_eq!(got[0].severity, DiagnosticSeverity::Error);
    assert_eq!(
        got[0].message.as_ref(),
        "type `User` does not have a field `nmae`"
    );
}

#[test]
fn an_empty_range_at_the_end_widens_back_over_the_last_char() {
    let text = "# 日本\n{ me { id }";
    let found = validate_query(text, None);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].range(), 20..20);
    let got = editor_diagnostics(&Rope::from(text), &found);
    assert_eq!(got[0].range, Position::new(1, 10)..Position::new(1, 11));
    assert_eq!(
        got[0].message.as_ref(),
        "syntax error: expected R_CURLY, got EOF"
    );
}

#[test]
fn the_summary_entry_is_sorted_in_and_zero_width_ranges_widen() {
    let text = "} ".repeat(300);
    let found = validate_query(&text, None);
    assert_eq!(found.len(), 201);
    assert_eq!(found[200].range(), 0..0);
    let got = editor_diagnostics(&Rope::from(text.as_str()), &found);
    assert_eq!(got.len(), 201);
    let at = |p: Position| (p.line, p.character);
    assert!(
        got.windows(2)
            .all(|w| at(w[0].range.start) <= at(w[1].range.start))
    );
    assert!(got.iter().all(|d| d.range.start != d.range.end));
    assert!(got.iter().any(|d| d.message.as_ref() == "and 100 more"
        && d.severity == DiagnosticSeverity::Info));
    assert!(editor_diagnostics(&Rope::from(""), &[]).is_empty());
}

#[test]
fn the_skip_note_sets_the_basis_and_stays_out_of_the_editor() {
    let c = Arc::new(compiled());
    let text = "{ me { {{f}} } }";
    let check = check_query(text, Some(c.as_ref()));
    let found = check.diagnostics();
    assert!(
        found.iter().any(|d| d.message() == SCHEMA_SKIP_NOTE),
        "{found:?}"
    );
    assert!(editor_diagnostics(&Rope::from(text), found).is_empty());
    let ready = Lookup::Ready {
        generation: 1,
        schema: c.clone(),
    };
    assert_eq!(
        CheckBasis::of(&ready, "f.graphqls", check.schema_use()),
        CheckBasis::Skipped
    );
    let plain = check_query("{ me { name } }", Some(c.as_ref())).schema_use();
    assert_eq!(
        CheckBasis::of(&ready, "f.graphqls", plain),
        CheckBasis::Schema {
            source: "f.graphqls".into()
        }
    );
    assert_eq!(
        CheckBasis::of(&Lookup::NoSchema, "u", SchemaUse::NotGiven),
        CheckBasis::NoSchema
    );
    let bad = Lookup::Unusable {
        generation: 2,
        reason: "bad".into(),
    };
    assert_eq!(
        CheckBasis::of(&bad, "u", SchemaUse::NotGiven),
        CheckBasis::Unusable("bad".into())
    );
}

#[test]
fn a_ready_schema_that_was_not_used_names_why() {
    let c = Arc::new(compiled());
    let ready = Lookup::Ready {
        generation: 1,
        schema: c.clone(),
    };
    let big = "{ me { name } }".to_owned() + &" ".repeat(QUERY_VALIDATE_LIMIT);
    let large = check_query(&big, Some(c.as_ref())).schema_use();
    assert_eq!(
        CheckBasis::of(&ready, "f.graphqls", large),
        CheckBasis::TooLarge
    );
    assert_eq!(
        CheckBasis::of(&ready, "f.graphqls", SchemaUse::Unusable),
        CheckBasis::Unusable("the schema could not be prepared".into())
    );
    assert_eq!(
        CheckBasis::of(&ready, "f.graphqls", SchemaUse::Failed),
        CheckBasis::Failed
    );
}

#[test]
fn labels_name_what_the_check_used() {
    assert_eq!(
        CheckBasis::Schema {
            source: "http://x/graphql".into()
        }
        .label(),
        "Checked against the schema from http://x/graphql"
    );
    assert_eq!(
        CheckBasis::NoSchema.label(),
        "Syntax only: no schema loaded"
    );
    assert_eq!(
        CheckBasis::Unusable("bad".into()).label(),
        "Syntax only: the schema cannot be used for checking (bad)"
    );
    assert_eq!(CheckBasis::Skipped.label(), SCHEMA_SKIP_NOTE);
    assert_eq!(
        CheckBasis::TooLarge.label(),
        "Syntax only: the query is too large to check against the schema"
    );
    assert_eq!(
        CheckBasis::Unusable("the schema could not be prepared".into()).label(),
        "Syntax only: the schema cannot be used for checking (the schema could not be prepared)"
    );
    assert_eq!(
        CheckBasis::Failed.label(),
        "Syntax only: the query could not be checked against the schema"
    );
}

#[test]
fn a_format_error_reads_as_line_and_column() {
    let err = format_query("{ me { id }").unwrap_err();
    assert_eq!(
        syntax_message("{ me { id }", &err),
        "Line 1, column 12: expected R_CURLY, got EOF"
    );
    let text = "# 日本\n{ me { id }";
    let err = format_query(text).unwrap_err();
    assert_eq!(
        syntax_message(text, &err),
        "Line 2, column 12: expected R_CURLY, got EOF"
    );
    let big = "x".repeat(QUERY_FORMAT_LIMIT + 1);
    let err = format_query(&big).unwrap_err();
    assert_eq!(
        syntax_message(&big, &err),
        "the query is larger than the 4 MiB format limit"
    );
}
