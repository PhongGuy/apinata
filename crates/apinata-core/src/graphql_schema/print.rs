//! The SDL printer: a [`Schema`] written as a schema document that the SDL reader turns back
//! into the same types. Introspection types and the built-in scalars are left out.

use super::model::{InputValue, Schema, SchemaType, TypeKind};
use super::sdl::{BUILT_IN_SCALARS, DEFAULT_DEPRECATION};

/// The document for `schema`: a `schema` block when the roots differ from the conventional
/// names, then each type in introspection order, separated by blank lines.
pub(super) fn print(schema: &Schema) -> String {
    let mut blocks = Vec::new();
    if let Some(block) = schema_block(schema) {
        blocks.push(block);
    }
    blocks.extend(schema.types.iter().filter(|t| printed(t)).map(type_block));
    let mut out = blocks.join("\n\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn printed(ty: &SchemaType) -> bool {
    !ty.name.starts_with("__")
        && !(ty.kind == TypeKind::Scalar && BUILT_IN_SCALARS.contains(&ty.name.as_str()))
}

/// A `schema` block when reading the document back without one would infer other roots, since
/// the reader defaults each root to the type named Query, Mutation or Subscription.
fn schema_block(schema: &Schema) -> Option<String> {
    let roots = [
        ("query", &schema.query_type, "Query"),
        ("mutation", &schema.mutation_type, "Mutation"),
        ("subscription", &schema.subscription_type, "Subscription"),
    ];
    let inferred = |name: &str| schema.types.iter().any(|t| t.name == name);
    let conventional = roots
        .iter()
        .all(|(_, root, name)| root.as_deref() == inferred(name).then_some(*name));
    let lines: Vec<String> = roots
        .iter()
        .filter_map(|(op, root, _)| root.as_ref().map(|r| format!("  {op}: {r}\n")))
        .collect();
    (!conventional && !lines.is_empty()).then(|| format!("schema {{\n{}}}", lines.concat()))
}

fn type_block(ty: &SchemaType) -> String {
    let mut out = description(ty.description.as_deref(), "");
    let implements = if ty.interfaces.is_empty() {
        String::new()
    } else {
        format!(" implements {}", ty.interfaces.join(" & "))
    };
    let name = &ty.name;
    match ty.kind {
        TypeKind::Scalar => out.push_str(&format!("scalar {name}")),
        TypeKind::Object | TypeKind::Interface => {
            let keyword = if ty.kind == TypeKind::Object {
                "type"
            } else {
                "interface"
            };
            out.push_str(&format!("{keyword} {name}{implements}"));
            let lines = ty.fields.iter().map(|f| {
                format!(
                    "{}  {}{}: {}{}\n",
                    description(f.description.as_deref(), "  "),
                    f.name,
                    arguments(&f.args),
                    f.ty.display,
                    deprecated(f.deprecation.as_deref())
                )
            });
            out.push_str(&braced(lines.collect()));
        }
        TypeKind::Union => {
            out.push_str(&format!("union {name}"));
            if !ty.possible_types.is_empty() {
                out.push_str(&format!(" = {}", ty.possible_types.join(" | ")));
            }
        }
        TypeKind::Enum => {
            out.push_str(&format!("enum {name}"));
            let lines = ty.enum_values.iter().map(|v| {
                format!(
                    "{}  {}{}\n",
                    description(v.description.as_deref(), "  "),
                    v.name,
                    deprecated(v.deprecation.as_deref())
                )
            });
            out.push_str(&braced(lines.collect()));
        }
        TypeKind::InputObject => {
            out.push_str(&format!("input {name}"));
            let lines = ty.input_fields.iter().map(|v| {
                format!(
                    "{}  {}\n",
                    description(v.description.as_deref(), "  "),
                    input_value(v)
                )
            });
            out.push_str(&braced(lines.collect()));
        }
    }
    out
}

/// Member lines in braces; a type without members is written bare, which is still valid SDL.
fn braced(lines: Vec<String>) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!(" {{\n{}}}", lines.concat())
    }
}

/// Arguments inline, or one per line when any carries a description.
fn arguments(args: &[InputValue]) -> String {
    if args.is_empty() {
        return String::new();
    }
    if args.iter().all(|a| a.description.is_none()) {
        let inline: Vec<String> = args.iter().map(input_value).collect();
        return format!("({})", inline.join(", "));
    }
    let lines: String = args
        .iter()
        .map(|a| {
            format!(
                "{}    {}\n",
                description(a.description.as_deref(), "    "),
                input_value(a)
            )
        })
        .collect();
    format!("(\n{lines}  )")
}

fn input_value(v: &InputValue) -> String {
    match &v.default_value {
        Some(default) => format!("{}: {} = {default}", v.name, v.ty.display),
        None => format!("{}: {}", v.name, v.ty.display),
    }
}

fn deprecated(reason: Option<&str>) -> String {
    match reason {
        None => String::new(),
        Some(DEFAULT_DEPRECATION) => " @deprecated".to_owned(),
        Some(reason) => format!(" @deprecated(reason: {})", quoted(reason)),
    }
}

/// A description line at `indent`, or nothing. Multi-line text is a block string only when the
/// reader's dedent and blank-line trimming would give back exactly the same text.
fn description(text: Option<&str>, indent: &str) -> String {
    let Some(text) = text else {
        return String::new();
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let blank = |l: &&str| l.trim().is_empty();
    let indented = |l: &&str| l.starts_with([' ', '\t']);
    let block = lines.len() > 1
        && !text.contains("\"\"\"")
        && !text.contains('\r')
        && !lines.first().is_some_and(blank)
        && !lines.last().is_some_and(blank)
        && !lines.iter().filter(|l| !blank(l)).all(indented);
    if !block {
        return format!("{indent}{}\n", quoted(text));
    }
    let body: String = lines.iter().map(|l| format!("{indent}{l}\n")).collect();
    format!("{indent}\"\"\"\n{body}{indent}\"\"\"\n")
}

/// `text` as a GraphQL string literal; JSON's escapes are all valid GraphQL escapes.
fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}
