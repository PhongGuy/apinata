//! Schema compilation and the panic guard for query validation.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use apollo_compiler::validation::Valid;

use super::{QueryDiagnostic, QuerySeverity};
use crate::{Schema, SchemaError};

/// A schema prepared for checking queries; build it once and share it across threads.
#[non_exhaustive]
pub struct CompiledSchema {
    #[allow(dead_code)] // The validator reads it.
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

/// Runs `check`, turning a panic into one Warning `the query could not be checked` at 0..0.
// The validator calls this.
#[allow(dead_code)]
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
