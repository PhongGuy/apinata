//! Warnings for deprecated fields and enum values a query uses.

use apollo_compiler::ast::{DirectiveList, Type, Value};
use apollo_compiler::executable::{Field, Selection, SelectionSet};
use apollo_compiler::parser::SourceSpan;
use apollo_compiler::schema::ExtendedType;
use apollo_compiler::{ExecutableDocument, Node, Schema};

use super::{QueryDiagnostic, QuerySeverity};

/// One Warning per deprecated field selection or enum value in a field argument, located at
/// the field's name or the enum value. Each fragment is walked once, not once per spread.
pub(crate) fn deprecations(doc: &ExecutableDocument, schema: &Schema) -> Vec<QueryDiagnostic> {
    let mut out = Vec::new();
    for op in doc.operations.iter() {
        selections(&op.selection_set, schema, &mut out);
    }
    for fragment in doc.fragments.values() {
        selections(&fragment.selection_set, schema, &mut out);
    }
    out
}

fn selections(set: &SelectionSet, schema: &Schema, out: &mut Vec<QueryDiagnostic>) {
    for selection in &set.selections {
        match selection {
            Selection::Field(field) => {
                self::field(&set.ty, field, schema, out);
                selections(&field.selection_set, schema, out);
            }
            Selection::InlineFragment(inline) => selections(&inline.selection_set, schema, out),
            Selection::FragmentSpread(_) => {}
        }
    }
}

fn field(parent: &str, field: &Field, schema: &Schema, out: &mut Vec<QueryDiagnostic>) {
    let def = &field.definition;
    warn(
        &def.directives,
        || format!("`{parent}.{}`", def.name),
        field.name.location(),
        out,
    );
    for arg in &field.arguments {
        if let Some(input) = def.argument_by_name(&arg.name) {
            value(&input.ty, &arg.value, schema, out);
        }
    }
}

/// Follows a value through lists and input objects to the enum values it names.
fn value(ty: &Type, value: &Node<Value>, schema: &Schema, out: &mut Vec<QueryDiagnostic>) {
    let named = ty.inner_named_type();
    match (value.as_ref(), schema.types.get(named)) {
        (Value::List(items), _) => {
            for item in items {
                self::value(ty, item, schema, out);
            }
        }
        (Value::Enum(name), Some(ExtendedType::Enum(enum_type))) => {
            if let Some(def) = enum_type.values.get(name) {
                warn(
                    &def.directives,
                    || format!("`{named}.{name}`"),
                    value.location(),
                    out,
                );
            }
        }
        (Value::Object(fields), Some(ExtendedType::InputObject(input))) => {
            for (name, field_value) in fields {
                if let Some(def) = input.fields.get(name) {
                    self::value(&def.ty, field_value, schema, out);
                }
            }
        }
        _ => {}
    }
}

fn warn(
    directives: &DirectiveList,
    subject: impl FnOnce() -> String,
    location: Option<SourceSpan>,
    out: &mut Vec<QueryDiagnostic>,
) {
    let Some(deprecated) = directives.get("deprecated") else {
        return;
    };
    let reason = deprecated
        .specified_argument_by_name("reason")
        .and_then(|r| r.as_str())
        .unwrap_or("No longer supported");
    let range = location.map_or(0..0, |loc| loc.offset()..loc.end_offset());
    out.push(QueryDiagnostic::new(
        range,
        QuerySeverity::Warning,
        format!("{} is deprecated: {reason}", subject()),
    ));
}
