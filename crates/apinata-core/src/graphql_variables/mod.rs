//! Completion and hover for the GraphQL variables editor, from the query's variable
//! definitions and, when loaded, the schema's input types.

mod locate;
mod walk;

use std::ops::Range;

use apollo_parser::cst::{self, CstNode};

use crate::graphql_query::mask::{mask, parse};
use crate::{QUERY_VALIDATE_LIMIT, Schema, TypeKind};
use locate::{Place, Role, locate};

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

/// One completion item for the variables JSON.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableCompletion {
    /// The name shown in the menu: `id`, `ADMIN`, `true`.
    pub label: String,
    /// The text that replaces `range`.
    pub insert: String,
    /// The byte range in the variables text that `insert` replaces.
    pub range: Range<usize>,
    /// What the item names.
    pub kind: VariableCompletionKind,
    /// The GraphQL type, or `deprecated` for a deprecated enum value.
    pub detail: Option<String>,
    /// The schema description.
    pub documentation: Option<String>,
}

/// What a variables completion names.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableCompletionKind {
    /// A top-level key, from the query's variable definitions.
    Variable,
    /// A nested key, from an input object's fields.
    InputField,
    /// An enum value, inserted as a JSON string.
    EnumValue,
    /// `true`, `false` or `null`.
    Literal,
}

/// Completions at byte `offset` of `variables`, filtered by the typed prefix (ASCII
/// case-insensitive). Empty over `QUERY_VALIDATE_LIMIT` or off a char boundary.
pub fn variable_completions(
    query: &str,
    variables: &str,
    offset: usize,
    schema: Option<&Schema>,
) -> Vec<VariableCompletion> {
    if query.len() > QUERY_VALIDATE_LIMIT || variables.len() > QUERY_VALIDATE_LIMIT {
        return Vec::new();
    }
    let Some(place) = locate(variables, offset) else {
        return Vec::new();
    };
    let defs = variable_definitions(query);
    let mut out = match place.role {
        Role::Key => key_candidates(&defs, schema, &place, variables),
        Role::Value => value_candidates(&defs, schema, &place),
    };
    out.retain(|c| starts_with_ignore_case(&c.label, &place.prefix));
    out
}

fn starts_with_ignore_case(label: &str, prefix: &str) -> bool {
    label
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

// Keys come from the query at the root and from the enclosing input object below it; keys
// the object already has are left out.
fn key_candidates(
    defs: &[VariableDefinition],
    schema: Option<&Schema>,
    place: &Place,
    variables: &str,
) -> Vec<VariableCompletion> {
    let names: Vec<(String, String, Option<String>, VariableCompletionKind)> =
        if place.path.is_empty() {
            defs.iter()
                .map(|d| {
                    (
                        d.name.clone(),
                        d.ty.clone(),
                        None,
                        VariableCompletionKind::Variable,
                    )
                })
                .collect()
        } else {
            let Some(object) = walk::type_at(defs, schema, &place.path)
                .zip(schema)
                .and_then(|(display, schema)| walk::input_object(schema, &display))
            else {
                return Vec::new();
            };
            object
                .input_fields()
                .iter()
                .map(|f| {
                    let doc = f.description().map(str::to_owned);
                    let ty = f.ty().display().to_owned();
                    (
                        f.name().to_owned(),
                        ty,
                        doc,
                        VariableCompletionKind::InputField,
                    )
                })
                .collect()
        };
    // A colon already typed after the key stays; otherwise the item adds one.
    let colon_follows = variables[place.token.end..]
        .trim_start_matches([' ', '\t'])
        .starts_with(':');
    let tail = if colon_follows { "" } else { ": " };
    names
        .into_iter()
        .filter(|(name, ..)| !place.present.contains(name))
        .map(|(name, ty, documentation, kind)| VariableCompletion {
            insert: format!("\"{name}\"{tail}"),
            label: name,
            range: place.token.clone(),
            kind,
            detail: Some(ty),
            documentation,
        })
        .collect()
}

// Values follow the type at the path: an enum's values (live ones first), booleans, then
// `null` when the type allows it. A list offers only `null`; its elements are typed inside `[`.
fn value_candidates(
    defs: &[VariableDefinition],
    schema: Option<&Schema>,
    place: &Place,
) -> Vec<VariableCompletion> {
    let Some(display) = walk::type_at(defs, schema, &place.path) else {
        return Vec::new();
    };
    let item =
        |label: &str, insert: String, kind, detail: &str, doc: Option<&str>| VariableCompletion {
            label: label.to_owned(),
            insert,
            range: place.token.clone(),
            kind,
            detail: Some(detail.to_owned()),
            documentation: doc.map(str::to_owned),
        };
    let mut out = Vec::new();
    if walk::element(&display).is_none() {
        let named = walk::named(&display);
        let schema_type = schema.and_then(|s| s.type_named(named));
        // With a schema, a type it does not have is off the schema; built-in scalars stay known.
        if schema.is_some() && schema_type.is_none() && !BUILT_IN_SCALARS.contains(&named) {
            return Vec::new();
        }
        if let Some(t) = schema_type.filter(|t| t.kind() == TypeKind::Enum) {
            let (live, deprecated): (Vec<_>, Vec<_>) = t
                .enum_values()
                .iter()
                .partition(|v| v.deprecation().is_none());
            for v in live {
                let insert = format!("\"{}\"", v.name());
                let kind = VariableCompletionKind::EnumValue;
                out.push(item(v.name(), insert, kind, t.name(), v.description()));
            }
            for v in deprecated {
                let insert = format!("\"{}\"", v.name());
                let kind = VariableCompletionKind::EnumValue;
                out.push(item(v.name(), insert, kind, "deprecated", v.description()));
            }
        }
        if named == "Boolean" {
            for word in ["true", "false"] {
                let kind = VariableCompletionKind::Literal;
                out.push(item(word, word.to_owned(), kind, "Boolean", None));
            }
        }
    }
    if walk::nullable(&display) {
        let kind = VariableCompletionKind::Literal;
        out.push(item("null", "null".to_owned(), kind, &display, None));
    }
    out
}

const BUILT_IN_SCALARS: [&str; 5] = ["Int", "Float", "String", "Boolean", "ID"];

/// A variables key explained for hover.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableHover {
    /// The key, quotes included.
    pub range: Range<usize>,
    /// `$id: ID!` at the root, `UserFilter.name: String` inside an input object.
    pub signature: String,
    /// The schema description of the input field.
    pub description: Option<String>,
    /// The query's or the schema's default, as source text.
    pub default: Option<String>,
}

/// The key under byte `offset` of `variables`, explained. None off a key, for an unknown key,
/// inside a placeholder, over `QUERY_VALIDATE_LIMIT`, or off a char boundary.
pub fn variable_hover(
    query: &str,
    variables: &str,
    offset: usize,
    schema: Option<&Schema>,
) -> Option<VariableHover> {
    if query.len() > QUERY_VALIDATE_LIMIT || variables.len() > QUERY_VALIDATE_LIMIT {
        return None;
    }
    let place = locate(variables, offset)?;
    // Only an offset on the key's own token explains it; a gap or a value has no key here.
    let token = variables.get(place.token.clone())?;
    if place.role != Role::Key || !place.token.contains(&offset) || !token.starts_with('"') {
        return None;
    }
    let body = &token[1..];
    let name = body.strip_suffix('"').unwrap_or(body);
    if name.is_empty() {
        return None;
    }
    let defs = variable_definitions(query);
    if place.path.is_empty() {
        let def = defs.iter().find(|d| d.name == name)?;
        return Some(VariableHover {
            range: place.token,
            signature: format!("${}: {}", def.name, def.ty),
            description: None,
            default: def.default.clone(),
        });
    }
    let display = walk::type_at(&defs, schema, &place.path)?;
    let object = walk::input_object(schema?, &display)?;
    let field = object.input_fields().iter().find(|f| f.name() == name)?;
    Some(VariableHover {
        range: place.token,
        signature: format!(
            "{}.{}: {}",
            object.name(),
            field.name(),
            field.ty().display()
        ),
        description: field.description().map(str::to_owned),
        default: field.default_value().map(str::to_owned),
    })
}
