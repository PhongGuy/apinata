//! Schema compilation, `validate_query` and the panic guard for query validation.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use apollo_compiler::ExecutableDocument;
use apollo_compiler::diagnostic::ToCliReport;
use apollo_compiler::validation::Valid;
use apollo_parser::SyntaxTree;

use super::deprecation::deprecations;
use super::filter::{filled_fields, finish, keep};
use super::mask::{Masked, any_structural, is_blank, mask, parse, snap};
use super::{QUERY_FORMAT_LIMIT, QUERY_VALIDATE_LIMIT, QueryDiagnostic, QuerySeverity};
use crate::{Schema, SchemaError};

/// A schema prepared for checking queries; build it once and share it across threads.
#[non_exhaustive]
pub struct CompiledSchema {
    /// `None` when building the schema panicked; queries then get syntax checks only.
    schema: Option<Valid<apollo_compiler::Schema>>,
}

const SCHEMA_UNUSABLE: &str = "The schema could not be prepared: only syntax is checked";

impl CompiledSchema {
    /// Prints `schema` as SDL and builds it leniently; fails only when `to_sdl` fails.
    pub fn new(schema: &Schema) -> Result<CompiledSchema, SchemaError> {
        let sdl = schema.to_sdl()?;
        // A schema the spec rejects (a missing interface field, say) still checks queries.
        let schema =
            build(
                || match apollo_compiler::Schema::parse_and_validate(sdl, "schema.graphql") {
                    Ok(valid) => valid,
                    Err(e) => Valid::assume_valid(e.partial),
                },
            );
        Ok(CompiledSchema { schema })
    }
}

/// Runs a schema build, turning a panic into `None` so it never reaches the caller.
fn build(
    compile: impl FnOnce() -> Valid<apollo_compiler::Schema>,
) -> Option<Valid<apollo_compiler::Schema>> {
    catch_unwind(AssertUnwindSafe(compile)).ok()
}

impl fmt::Debug for CompiledSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CompiledSchema { .. }")
    }
}

/// Checks `text` for syntax and, given a schema, against it. Ranges index the original text.
///
/// Over [`QUERY_FORMAT_LIMIT`] nothing is parsed; over [`QUERY_VALIDATE_LIMIT`], or when a
/// `{{placeholder}}` stands for query structure, only syntax is checked.
pub fn validate_query(text: &str, schema: Option<&CompiledSchema>) -> Vec<QueryDiagnostic> {
    finish(unsorted(text, schema))
}

fn unsorted(text: &str, schema: Option<&CompiledSchema>) -> Vec<QueryDiagnostic> {
    if text.len() > QUERY_FORMAT_LIMIT {
        return vec![QueryDiagnostic::new(
            0..0,
            QuerySeverity::Warning,
            "The query is larger than 4 MiB and was not checked",
        )];
    }
    let masked = mask(text);
    let tree = parse(&masked.text);
    if is_blank(&tree) {
        return Vec::new();
    }
    if text.len() > QUERY_VALIDATE_LIMIT {
        let mut out = vec![QueryDiagnostic::new(
            0..0,
            QuerySeverity::Warning,
            "The query is larger than 256 KiB: only syntax is checked",
        )];
        out.extend(syntax(text, &tree));
        return out;
    }
    let Some(compiled) = schema else {
        return syntax(text, &tree);
    };
    let Some(apollo) = &compiled.schema else {
        let mut out = vec![QueryDiagnostic::new(
            0..0,
            QuerySeverity::Warning,
            SCHEMA_UNUSABLE,
        )];
        out.extend(syntax(text, &tree));
        return out;
    };
    if any_structural(&tree, &masked.spans) {
        let mut out = syntax(text, &tree);
        out.push(QueryDiagnostic::new(
            0..0,
            QuerySeverity::Info,
            "Schema checks skipped: a {{placeholder}} stands for query structure",
        ));
        return out;
    }
    guarded(|| against_schema(text, masked, &tree, apollo))
}

/// The parser's errors, as apollo-compiler words them, located in the original text.
fn syntax(text: &str, tree: &SyntaxTree) -> Vec<QueryDiagnostic> {
    tree.errors()
        .map(|err| {
            let start = err.index();
            QueryDiagnostic::new(
                snap(text, start..start + err.data().len()),
                QuerySeverity::Error,
                format!("syntax error: {}", err.message()),
            )
        })
        .collect()
}

/// Apollo's diagnostics for the masked query, minus the filtered ones and any located in the
/// schema's sources, plus the deprecation warnings.
fn against_schema(
    text: &str,
    masked: Masked,
    tree: &SyntaxTree,
    schema: &Valid<apollo_compiler::Schema>,
) -> Vec<QueryDiagnostic> {
    let (doc, errors) =
        match ExecutableDocument::parse_and_validate(schema, masked.text, "query.graphql") {
            Ok(valid) => (valid.into_inner(), None),
            Err(e) => (e.partial, Some(e.errors)),
        };
    let filled = filled_fields(tree);
    let mut out: Vec<QueryDiagnostic> = errors
        .iter()
        .flat_map(|list| list.iter())
        .filter_map(|d| {
            let location = d.error.location();
            if location.is_some_and(|loc| schema.sources.contains_key(&loc.file_id())) {
                return None;
            }
            let range = location.map_or(0..0, |loc| loc.offset()..loc.end_offset());
            keep(
                d.error.unstable_error_name(),
                &range,
                &masked.spans,
                &filled,
            )
            .then(|| {
                QueryDiagnostic::new(snap(text, range), QuerySeverity::Error, d.error.to_string())
            })
        })
        .collect();
    out.extend(
        deprecations(&doc, schema)
            .into_iter()
            .map(|d| QueryDiagnostic::new(snap(text, d.range()), d.severity(), d.message())),
    );
    out
}

/// Runs `check`, turning a panic into one Warning `the query could not be checked` at 0..0.
pub(crate) fn guarded(check: impl FnOnce() -> Vec<QueryDiagnostic>) -> Vec<QueryDiagnostic> {
    catch_unwind(AssertUnwindSafe(check)).unwrap_or_else(|_| {
        vec![QueryDiagnostic::new(
            0..0,
            QuerySeverity::Warning,
            "the query could not be checked",
        )]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql_query::{QueryDiagnostic, QuerySeverity};

    #[test]
    fn a_panic_becomes_one_warning() {
        let got = guarded(|| panic!("boom"));
        assert_eq!(
            got,
            vec![QueryDiagnostic::new(
                0..0,
                QuerySeverity::Warning,
                "the query could not be checked"
            )]
        );
    }

    #[test]
    fn a_panicking_schema_build_checks_syntax_only_with_one_warning() {
        let c = CompiledSchema {
            schema: build(|| panic!("schema boom")),
        };
        let warning = QueryDiagnostic::new(0..0, QuerySeverity::Warning, SCHEMA_UNUSABLE);
        assert_eq!(
            validate_query("{ me { nmae } }", Some(&c)),
            vec![warning.clone()]
        );
        assert_eq!(
            validate_query("{ me { id } } }", Some(&c)),
            vec![
                warning,
                QueryDiagnostic::new(
                    14..15,
                    QuerySeverity::Error,
                    "syntax error: expected a StringValue, Name or OperationDefinition"
                )
            ]
        );
        assert_eq!(validate_query("  ", Some(&c)), vec![]);
    }
}
