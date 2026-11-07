//! Walks a variables path through GraphQL type display strings such as `[UserFilter!]!`.

use super::VariableDefinition;
use super::locate::Step;
use crate::{Schema, SchemaType, TypeKind};

/// The type display at `path`: the variable's type for the first `Key`, then the element of a
/// list for an `Index`, or an input object's field type for a `Key`. None off the schema.
pub(super) fn type_at(
    defs: &[VariableDefinition],
    schema: Option<&Schema>,
    path: &[Step],
) -> Option<String> {
    let (Step::Key(first), rest) = path.split_first()? else {
        return None;
    };
    let mut display = defs.iter().find(|d| &d.name == first)?.ty.clone();
    for step in rest {
        display = match step {
            Step::Index(_) => element(&display)?.to_owned(),
            Step::Key(key) => {
                let object = input_object(schema?, &display)?;
                let field = object.input_fields().iter().find(|f| f.name() == key)?;
                field.ty().display().to_owned()
            }
        };
    }
    Some(display)
}

/// The input object a display such as `UserFilter!` names; None for a list or another kind.
pub(super) fn input_object<'a>(schema: &'a Schema, display: &str) -> Option<&'a SchemaType> {
    if display.starts_with('[') {
        return None;
    }
    schema
        .type_named(named(display))
        .filter(|t| t.kind() == TypeKind::InputObject)
}

/// Whether a value of this type may be `null`.
pub(super) fn nullable(display: &str) -> bool {
    !display.ends_with('!')
}

/// The element type of a list display, or None when it is not a list.
pub(super) fn element(display: &str) -> Option<&str> {
    let inner = display.strip_suffix('!').unwrap_or(display);
    inner.strip_prefix('[')?.strip_suffix(']')
}

/// The named type under any list and non-null wrappers.
pub(super) fn named(display: &str) -> &str {
    display.trim_matches(['[', ']', '!'])
}
