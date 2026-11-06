//! Schema compilation, `check_query`, `validate_query` and the panic guard for query validation.

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

/// The note added when a `{{placeholder}}` stands for query structure and schema checks are skipped.
pub const SCHEMA_SKIP_NOTE: &str =
    "Schema checks skipped: a {{placeholder}} stands for query structure";

/// How a query check used the schema it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemaUse {
    /// The schema checks ran; a blank query with a schema counts as checked.
    Checked,
    /// No schema was passed, so only syntax was checked.
    NotGiven,
    /// The query is over [`QUERY_VALIDATE_LIMIT`] or [`QUERY_FORMAT_LIMIT`].
    TooLarge,
    /// A `{{placeholder}}` stands for query structure.
    Placeholder,
    /// The compiled schema's build failed.
    Unusable,
    /// The check against the schema panicked.
    Failed,
}

/// A query check's findings and whether the schema checks actually ran.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryCheck {
    diagnostics: Vec<QueryDiagnostic>,
    schema_use: SchemaUse,
}

impl QueryCheck {
    fn new(diagnostics: Vec<QueryDiagnostic>, schema_use: SchemaUse) -> Self {
        Self {
            diagnostics,
            schema_use,
        }
    }

    /// The findings, sorted and capped as [`validate_query`] returns them.
    pub fn diagnostics(&self) -> &[QueryDiagnostic] {
        &self.diagnostics
    }

    /// The findings, by value.
    pub fn into_diagnostics(self) -> Vec<QueryDiagnostic> {
        self.diagnostics
    }

    /// How the schema was used.
    pub fn schema_use(&self) -> SchemaUse {
        self.schema_use
    }

    /// Whether the schema checks ran.
    pub fn schema_checked(&self) -> bool {
        self.schema_use == SchemaUse::Checked
    }
}

/// Checks `text` for syntax and, given a schema, against it. Ranges index the original text.
///
/// Over [`QUERY_FORMAT_LIMIT`] nothing is parsed; over [`QUERY_VALIDATE_LIMIT`], or when a
/// `{{placeholder}}` stands for query structure, only syntax is checked.
pub fn validate_query(text: &str, schema: Option<&CompiledSchema>) -> Vec<QueryDiagnostic> {
    check_query(text, schema).into_diagnostics()
}

/// Like [`validate_query`], and also says whether the schema checks ran or why they did not.
pub fn check_query(text: &str, schema: Option<&CompiledSchema>) -> QueryCheck {
    let check = unsorted(text, schema);
    QueryCheck::new(finish(check.diagnostics), check.schema_use)
}

fn unsorted(text: &str, schema: Option<&CompiledSchema>) -> QueryCheck {
    // A size limit only explains a skipped schema check when there was a schema to skip.
    let too_large = if schema.is_some() {
        SchemaUse::TooLarge
    } else {
        SchemaUse::NotGiven
    };
    if text.len() > QUERY_FORMAT_LIMIT {
        let note = QueryDiagnostic::new(
            0..0,
            QuerySeverity::Warning,
            "The query is larger than 4 MiB and was not checked",
        );
        return QueryCheck::new(vec![note], too_large);
    }
    let masked = mask(text);
    let tree = parse(&masked.text);
    if is_blank(&tree) {
        let schema_use = match schema {
            None => SchemaUse::NotGiven,
            Some(CompiledSchema { schema: None }) => SchemaUse::Unusable,
            Some(_) => SchemaUse::Checked,
        };
        return QueryCheck::new(Vec::new(), schema_use);
    }
    if text.len() > QUERY_VALIDATE_LIMIT {
        let mut out = vec![QueryDiagnostic::new(
            0..0,
            QuerySeverity::Warning,
            "The query is larger than 256 KiB: only syntax is checked",
        )];
        out.extend(syntax(text, &tree));
        return QueryCheck::new(out, too_large);
    }
    let Some(compiled) = schema else {
        return QueryCheck::new(syntax(text, &tree), SchemaUse::NotGiven);
    };
    let Some(apollo) = &compiled.schema else {
        let mut out = vec![QueryDiagnostic::new(
            0..0,
            QuerySeverity::Warning,
            SCHEMA_UNUSABLE,
        )];
        out.extend(syntax(text, &tree));
        return QueryCheck::new(out, SchemaUse::Unusable);
    };
    if any_structural(&tree, &masked.spans) {
        let mut out = syntax(text, &tree);
        out.push(QueryDiagnostic::new(
            0..0,
            QuerySeverity::Info,
            SCHEMA_SKIP_NOTE,
        ));
        return QueryCheck::new(out, SchemaUse::Placeholder);
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

/// Runs the schema `check`, turning a panic into one Warning `the query could not be checked`
/// at 0..0 and [`SchemaUse::Failed`].
pub(crate) fn guarded(check: impl FnOnce() -> Vec<QueryDiagnostic>) -> QueryCheck {
    match catch_unwind(AssertUnwindSafe(check)) {
        Ok(found) => QueryCheck::new(found, SchemaUse::Checked),
        Err(_) => {
            let note = QueryDiagnostic::new(
                0..0,
                QuerySeverity::Warning,
                "the query could not be checked",
            );
            QueryCheck::new(vec![note], SchemaUse::Failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql_query::{QueryDiagnostic, QuerySeverity};

    #[test]
    fn a_panic_becomes_one_warning() {
        let got = guarded(|| panic!("boom"));
        assert_eq!(
            got.diagnostics(),
            [QueryDiagnostic::new(
                0..0,
                QuerySeverity::Warning,
                "the query could not be checked"
            )]
        );
    }

    #[test]
    fn a_panicking_check_reports_failed() {
        let got = guarded(|| panic!("boom"));
        assert_eq!(got.schema_use(), SchemaUse::Failed);
        assert!(!got.schema_checked());
        let fine = guarded(Vec::new);
        assert_eq!(fine.schema_use(), SchemaUse::Checked);
    }

    #[test]
    fn a_schema_whose_build_panicked_reports_unusable() {
        let c = CompiledSchema {
            schema: build(|| panic!("schema boom")),
        };
        for text in ["{ me { nmae } }", "  "] {
            let got = check_query(text, Some(&c));
            assert_eq!(got.schema_use(), SchemaUse::Unusable, "{text:?}");
            assert!(!got.schema_checked());
        }
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
