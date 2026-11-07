//! Completion and hover for the GraphQL variables editor, from the query's variable
//! definitions and, when loaded, the schema's input types.

// Unused outside tests until the completion entry points call it.
#[cfg_attr(not(test), allow(dead_code))]
mod locate;

use std::ops::Range;

use apollo_parser::cst::{self, CstNode};

use crate::QUERY_VALIDATE_LIMIT;
use crate::graphql_query::mask::{mask, parse};

/// One `$name: Type` from the query's operations, in source order, first definition per name.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableDefinition {
    /// The name without `$`.
    pub name: String,
    /// The type as written, whitespace removed: `[Int!]`.
    pub ty: String,
    /// The default value's source text from the original query, placeholders kept.
    pub default: Option<String>,
}

/// The variable definitions of every operation in `query`, read from apollo-parser's CST so a
/// half-written query still has them. Empty over `QUERY_VALIDATE_LIMIT`.
pub fn variable_definitions(query: &str) -> Vec<VariableDefinition> {
    if query.len() > QUERY_VALIDATE_LIMIT {
        return Vec::new();
    }
    let masked = mask(query);
    let tree = parse(&masked.text);
    let touches = |r: Range<usize>| {
        masked
            .spans
            .iter()
            .any(|s| r.start < s.end && s.start < r.end)
    };
    let range = |n: &apollo_parser::SyntaxNode| {
        let r = n.text_range();
        usize::from(r.start())..usize::from(r.end())
    };

    let mut out: Vec<VariableDefinition> = Vec::new();
    for def in tree.document().definitions() {
        let cst::Definition::OperationDefinition(op) = def else {
            continue;
        };
        let Some(list) = op.variable_definitions() else {
            continue;
        };
        for vd in list.variable_definitions() {
            let Some(name) = vd.variable().and_then(|v| v.name()) else {
                continue;
            };
            let Some(ty) = vd.ty() else { continue };
            let (name_range, ty_range) = (range(name.syntax()), range(ty.syntax()));
            if touches(name_range) || touches(ty_range) {
                continue;
            }
            let name = name.text().to_string();
            if out.iter().any(|d| d.name == name) {
                continue;
            }
            let ty: String = ty
                .syntax()
                .text()
                .to_string()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let default = vd
                .default_value()
                .and_then(|v| v.value())
                .map(|v| query[range(v.syntax())].to_owned());
            out.push(VariableDefinition { name, ty, default });
        }
    }
    out
}
