//! The schema element a name in a GraphQL query refers to, for hover and jump to definition.

use std::ops::Range;

use super::complete::{Context, Scanner};
use super::model::Schema;
use crate::graphql_query::QUERY_VALIDATE_LIMIT;
use crate::graphql_query::mask::mask;

/// A schema element found in query text, with the byte range of its name.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolAt {
    /// The schema element the name refers to.
    pub symbol: SchemaSymbol,
    /// The byte range of the name in the query text.
    pub range: Range<usize>,
}

/// A schema element a query name can refer to.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaSymbol {
    /// A type name, after `on` or in a variable definition's type.
    Type {
        /// The type's name.
        name: String,
    },
    /// A selected field, owned by the type it is selected on.
    Field {
        /// The type the field is selected on.
        owner: String,
        /// The field's name.
        field: String,
    },
    /// An argument name inside a field's parentheses.
    Argument {
        /// The type that owns the field.
        owner: String,
        /// The field the argument belongs to.
        field: String,
        /// The argument's name.
        arg: String,
    },
}

impl Schema {
    /// The schema element named at byte `offset` of `query`, with the byte range of its name.
    /// `None` off a name, inside a placeholder, on an alias, or for a name the schema lacks.
    pub fn symbol_at(&self, query: &str, offset: usize) -> Option<SymbolAt> {
        if query.len() > QUERY_VALIDATE_LIMIT || !query.is_char_boundary(offset) {
            return None;
        }
        // Masked text keeps every offset, and a placeholder reads as one name to the scanner.
        let masked = mask(query);
        if masked
            .spans
            .iter()
            .any(|s| s.start <= offset && offset <= s.end)
        {
            return None;
        }
        let text = masked.text.as_str();
        let is_name = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let start = text[..offset]
            .bytes()
            .rposition(|b| !is_name(b))
            .map_or(0, |i| i + 1);
        let end = text[offset..]
            .bytes()
            .position(|b| !is_name(b))
            .map_or(text.len(), |i| offset + i);
        let name = &text[start..end];
        if name.is_empty() || name.as_bytes()[0].is_ascii_digit() || name.starts_with("__") {
            return None;
        }
        let symbol = match Scanner::new(self).run(&text[..start])? {
            Context::Fields(owner) if !colon_follows(&text[end..]) => {
                self.selectable(&owner)?
                    .fields()
                    .iter()
                    .find(|f| f.name() == name)?;
                SchemaSymbol::Field {
                    owner,
                    field: name.to_owned(),
                }
            }
            Context::Args { parent, field } => {
                let f = self
                    .selectable(&parent)?
                    .fields()
                    .iter()
                    .find(|f| f.name() == field)?;
                f.args().iter().find(|a| a.name() == name)?;
                SchemaSymbol::Argument {
                    owner: parent,
                    field,
                    arg: name.to_owned(),
                }
            }
            Context::Types(kinds) => {
                self.type_named(name)
                    .filter(|t| kinds.contains(&t.kind()))?;
                SchemaSymbol::Type {
                    name: name.to_owned(),
                }
            }
            _ => return None,
        };
        Some(SymbolAt {
            symbol,
            range: start..end,
        })
    }
}

/// True when the next token after `rest` is `:`, so the name before it is an alias.
/// Whitespace, commas and `#` comments are skipped, as the GraphQL lexer ignores them.
fn colon_follows(rest: &str) -> bool {
    let b = rest.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\n' | b'\r' | b',' => i += 1,
            b'#' => {
                while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
                    i += 1;
                }
            }
            c => return c == b':',
        }
    }
    false
}
