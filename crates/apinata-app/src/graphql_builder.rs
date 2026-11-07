//! The GraphQL query builder: schema fields as a tree of rows ticked from the query text.

use std::collections::HashSet;
use std::ops::Range;

use apinata_core::{FieldPath, Lock, QueryOutline, Schema, TypeKind};

/// The lock note on every row while the query text does not parse.
pub const BROKEN_NOTE: &str = "Fix the query to edit it here";

/// What a row stands for, which decides its chevron and checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A root operation type: a chevron, no checkbox.
    Root,
    /// A scalar or enum field, or one whose type is missing.
    Leaf,
    /// An object or interface field with fields of its own.
    Composite,
    /// A union field.
    Union,
}

/// One line of the builder tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderRow {
    /// The root type name and the field names down to this row.
    pub path: FieldPath,
    /// 0 for a root type.
    pub depth: usize,
    /// `query: Query` for a root, else the field name.
    pub label: String,
    /// The field's type as written in SDL; empty for a root.
    pub ty: String,
    /// What the row stands for.
    pub kind: RowKind,
    /// Whether the field is deprecated.
    pub deprecated: bool,
    /// Whether the row's children are listed.
    pub expanded: bool,
    /// Whether the query selects the path; false for roots and broken text.
    pub checked: bool,
    /// Why the row cannot be toggled.
    pub lock: Option<String>,
}

/// Root types in query, mutation, subscription order; children only under expanded paths.
pub fn rows(
    schema: &Schema,
    outline: Option<&QueryOutline>,
    expanded: &HashSet<FieldPath>,
    show_deprecated: bool,
) -> Vec<BuilderRow> {
    let walk = Walk {
        schema,
        outline,
        expanded,
        show_deprecated,
    };
    let roots = [
        ("query", schema.query_type()),
        ("mutation", schema.mutation_type()),
        ("subscription", schema.subscription_type()),
    ];
    let mut out = Vec::new();
    for (op, name) in roots {
        let Some(name) = name else { continue };
        let path = FieldPath::new([name]);
        let open = expanded.contains(&path);
        out.push(BuilderRow {
            lock: walk.lock(&path),
            path: path.clone(),
            depth: 0,
            label: format!("{op}: {name}"),
            ty: String::new(),
            kind: RowKind::Root,
            deprecated: false,
            expanded: open,
            checked: false,
        });
        if open {
            walk.children(&mut out, &path, name, 1);
        }
    }
    out
}

struct Walk<'a, 's> {
    schema: &'a Schema,
    outline: Option<&'a QueryOutline<'s>>,
    expanded: &'a HashSet<FieldPath>,
    show_deprecated: bool,
}

impl Walk<'_, '_> {
    fn lock(&self, path: &FieldPath) -> Option<String> {
        match self.outline {
            None => Some(BROKEN_NOTE.to_owned()),
            Some(o) => o.locked(path).map(|l| lock_text(path, &l)),
        }
    }

    // Recursion follows only expanded paths, so a self-referencing type stays finite.
    fn children(&self, out: &mut Vec<BuilderRow>, parent: &FieldPath, ty: &str, depth: usize) {
        let Some(ty) = self.schema.type_named(ty) else {
            return;
        };
        for field in ty.fields() {
            let deprecated = field.deprecation().is_some();
            if deprecated && !self.show_deprecated {
                continue;
            }
            let named = field.ty().named();
            let kind = match self.schema.type_named(named).map(|t| t.kind()) {
                None | Some(TypeKind::Scalar | TypeKind::Enum) => RowKind::Leaf,
                Some(TypeKind::Union) => RowKind::Union,
                Some(_) => RowKind::Composite,
            };
            let path = FieldPath::new(parent.names().iter().chain([&field.name().to_owned()]));
            let open = self.expanded.contains(&path);
            out.push(BuilderRow {
                lock: self.lock(&path),
                checked: self.outline.is_some_and(|o| o.selects(&path)),
                path: path.clone(),
                depth,
                label: field.name().to_owned(),
                ty: field.ty().display().to_owned(),
                kind,
                deprecated,
                expanded: open,
            });
            if open && kind == RowKind::Composite {
                self.children(out, &path, named, depth + 1);
            }
        }
    }
}

/// The note shown for a row the builder will not toggle.
pub fn lock_text(_path: &FieldPath, lock: &Lock) -> String {
    match lock {
        Lock::OtherRoot => "One operation per request".into(),
        Lock::SubscriptionField => "A subscription selects one field".into(),
        Lock::Union => "Union fields need inline fragments".into(),
        Lock::LastChildOf(n) => format!("Last field of `{n}`; untick `{n}` to remove it"),
        Lock::LastOfOperation(n) => {
            format!("Last field of operation `{n}`; edit the query to remove it")
        }
        _ => "Not editable here".into(),
    }
}

/// Where a caret at `caret` lands after `range` is replaced by `inserted` bytes.
pub fn caret_after(caret: usize, range: Range<usize>, inserted: usize) -> usize {
    if caret <= range.start {
        caret
    } else if caret >= range.end {
        caret - range.len() + inserted
    } else {
        range.start
    }
}
