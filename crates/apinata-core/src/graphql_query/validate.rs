//! Schema compilation, `validate_query` and the panic guard for query validation.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use apollo_compiler::ExecutableDocument;
use apollo_compiler::diagnostic::ToCliReport;
use apollo_compiler::validation::Valid;
use apollo_parser::SyntaxTree;

use super::mask::{any_structural, is_blank, mask, parse, snap};
use super::{QUERY_FORMAT_LIMIT, QUERY_VALIDATE_LIMIT, QueryDiagnostic, QuerySeverity};
use crate::{Schema, SchemaError};

/// A schema prepared for checking queries; build it once and share it across threads.
#[non_exhaustive]
pub struct CompiledSchema {
    schema: Valid<apollo_compiler::Schema>,
}

impl CompiledSchema {
    /// Prints `schema` as SDL and builds it leniently; fails only when `to_sdl` fails.
    pub fn new(schema: &Schema) -> Result<CompiledSchema, SchemaError> {
        let sdl = schema.to_sdl()?;
        // A schema the spec rejects (a missing interface field, say) still checks queries.
        let schema = match apollo_compiler::Schema::parse_and_validate(sdl, "schema.graphql") {
            Ok(valid) => valid,
            Err(e) => Valid::assume_valid(e.partial),
        };
        Ok(CompiledSchema { schema })
    }
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
    if any_structural(&tree, &masked.spans) {
        let mut out = syntax(text, &tree);
        out.push(QueryDiagnostic::new(
            0..0,
            QuerySeverity::Info,
            "Schema checks skipped: a {{placeholder}} stands for query structure",
        ));
        return out;
    }
    guarded(|| against_schema(text, masked.text, &compiled.schema))
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

/// Apollo's diagnostics for the masked query, skipping any located in the schema's sources.
fn against_schema(
    text: &str,
    masked: String,
    schema: &Valid<apollo_compiler::Schema>,
) -> Vec<QueryDiagnostic> {
    let Err(with_errors) = ExecutableDocument::parse_and_validate(schema, masked, "query.graphql")
    else {
        return Vec::new();
    };
    with_errors
        .errors
        .iter()
        .filter_map(|d| {
            let location = d.error.location();
            if location.is_some_and(|loc| schema.sources.contains_key(&loc.file_id())) {
                return None;
            }
            let range = location.map_or(0..0, |loc| loc.offset()..loc.end_offset());
            Some(QueryDiagnostic::new(
                snap(text, range),
                QuerySeverity::Error,
                d.error.to_string(),
            ))
        })
        .collect()
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
}
