//! `query_outline`: the selections of a query's first operation, read against a schema.
//!
//! The masked query is walked once into an owned tree of selections with tight byte
//! ranges; the outline answers which field paths are selected and which may not be
//! toggled, and tells the editing code what unticking a path would remove.

use std::ops::Range;

use apollo_parser::cst::{self, CstNode};
use apollo_parser::{SyntaxElement, SyntaxKind, SyntaxNode};

use super::QUERY_VALIDATE_LIMIT;
use super::mask::{is_blank, mask, parse};
use crate::{Field, Schema, TypeKind};

/// A path through the schema: a root type name followed by field names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FieldPath {
    names: Vec<String>,
}

impl FieldPath {
    /// Builds a path from the root type name and the field names below it.
    pub fn new(names: impl IntoIterator<Item = impl Into<String>>) -> FieldPath {
        FieldPath {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// The root type name, then each field name.
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// Why a field path cannot be toggled in the current query.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Lock {
    /// The path starts at a root type other than the operation's.
    OtherRoot,
    /// A subscription already selects its one top-level field.
    SubscriptionField,
    /// Unticking would empty the named field, which has arguments or directives.
    LastChildOf(String),
    /// Unticking would empty the operation, named here or by its keyword.
    LastOfOperation(String),
    /// The field's type is a union, which needs inline fragments.
    Union,
}

/// The selections of a query's first operation, read against a schema.
#[non_exhaustive]
pub struct QueryOutline<'s> {
    schema: &'s Schema,
    op: Option<Op>,
}

/// A selection set: the offsets of its braces and its selections in order.
pub(crate) struct Set {
    pub(crate) open: usize,
    pub(crate) close: usize,
    pub(crate) items: Vec<Sel>,
}

/// One selection of any kind; only an unaliased field carries a name.
pub(crate) struct Sel {
    pub(crate) name: Option<String>,
    pub(crate) protected: bool,
    pub(crate) range: Range<usize>,
    pub(crate) set: Option<Set>,
}

/// The first operation definition of the document.
pub(crate) struct Op {
    pub(crate) root: Option<String>,
    pub(crate) keyword: &'static str,
    pub(crate) label: String,
    pub(crate) protected: bool,
    pub(crate) range: Range<usize>,
    pub(crate) set: Set,
}

/// What unticking a path removes.
pub(crate) enum Cut<'a> {
    Field(&'a Sel),
    Operation(&'a Op),
    Locked(Lock),
}

/// Outlines `query` against `schema`. `None` when the text is larger than
/// [`QUERY_VALIDATE_LIMIT`](crate::QUERY_VALIDATE_LIMIT) or does not parse; blank text
/// and a document without operations give an outline with no operation root.
pub fn query_outline<'s>(query: &str, schema: &'s Schema) -> Option<QueryOutline<'s>> {
    if query.len() > QUERY_VALIDATE_LIMIT {
        return None;
    }
    let masked = mask(query);
    let tree = parse(&masked.text);
    if is_blank(&tree) {
        return Some(QueryOutline { schema, op: None });
    }
    if tree.errors().next().is_some() {
        return None;
    }
    let op = tree.document().definitions().find_map(|def| match def {
        cst::Definition::OperationDefinition(op) => Some(op),
        _ => None,
    });
    Some(QueryOutline {
        schema,
        op: op.and_then(|op| walk_op(&op, schema)),
    })
}

impl<'s> QueryOutline<'s> {
    /// True when some unaliased chain of fields outside fragments matches `path`.
    pub fn selects(&self, path: &FieldPath) -> bool {
        self.chain(path).is_some()
    }

    /// The schema root type of the first operation; `None` without one, or when the
    /// schema has no root type for its operation type.
    pub fn operation_root(&self) -> Option<&str> {
        self.op.as_ref()?.root.as_deref()
    }

    /// Why `path` cannot be toggled, or `None` when it can.
    pub fn locked(&self, path: &FieldPath) -> Option<Lock> {
        let op = self.op.as_ref()?;
        let names = path.names();
        if op
            .root
            .as_deref()
            .is_none_or(|root| names.first().map(String::as_str) != Some(root))
        {
            return Some(Lock::OtherRoot);
        }
        // A subscription takes exactly one top-level field, so any other one is locked.
        if op.keyword == "subscription"
            && let Some(top) = names.get(1)
            && !op.set.items.iter().any(|s| s.name.as_ref() == Some(top))
        {
            return Some(Lock::SubscriptionField);
        }
        if let Some(fields) = resolve(self.schema, path)
            && let Some(last) = fields.last()
            && self
                .schema
                .type_named(last.ty().named())
                .is_some_and(|t| t.kind() == TypeKind::Union)
        {
            return Some(Lock::Union);
        }
        match self.cut(path)? {
            Cut::Locked(lock) => Some(lock),
            Cut::Field(_) | Cut::Operation(_) => None,
        }
    }

    pub(crate) fn schema(&self) -> &'s Schema {
        self.schema
    }

    pub(crate) fn op(&self) -> Option<&Op> {
        self.op.as_ref()
    }

    /// The first chain of unaliased copies matching path[1..], top first.
    pub(crate) fn chain(&self, path: &FieldPath) -> Option<Vec<&Sel>> {
        let op = self.op.as_ref()?;
        let (root, rest) = path.names().split_first()?;
        if op.root.as_ref() != Some(root) {
            return None;
        }
        let mut found = Vec::with_capacity(rest.len());
        descend(&op.set, rest, &mut found).then_some(found)
    }

    /// What unticking `path` removes: the topmost node the removal leaves empty.
    pub(crate) fn cut(&self, path: &FieldPath) -> Option<Cut<'_>> {
        let op = self.op.as_ref()?;
        let chain = self.chain(path)?;
        let mut at = chain.len().checked_sub(1)?;
        // Climb while the removal would leave the parent empty; a protected parent
        // cannot be removed, so the climb ends in a lock there.
        loop {
            let Some(parent_at) = at.checked_sub(1) else {
                return Some(match (op.set.items.len(), op.protected) {
                    (1, true) => Cut::Locked(Lock::LastOfOperation(op.label.clone())),
                    (1, false) => Cut::Operation(op),
                    _ => Cut::Field(chain[at]),
                });
            };
            let parent = chain[parent_at];
            let only_child = parent.set.as_ref().is_some_and(|s| s.items.len() == 1);
            if !only_child {
                return Some(Cut::Field(chain[at]));
            }
            if parent.protected {
                let name = parent.name.clone().unwrap_or_default();
                return Some(Cut::Locked(Lock::LastChildOf(name)));
            }
            at = parent_at;
        }
    }
}

/// Depth-first search for the first chain of unaliased fields named `rest`, in document order.
fn descend<'a>(set: &'a Set, rest: &[String], found: &mut Vec<&'a Sel>) -> bool {
    let Some((name, tail)) = rest.split_first() else {
        return true;
    };
    for sel in set.items.iter().filter(|s| s.name.as_ref() == Some(name)) {
        found.push(sel);
        let matched = match &sel.set {
            _ if tail.is_empty() => true,
            Some(inner) => descend(inner, tail, found),
            None => false,
        };
        if matched {
            return true;
        }
        found.pop();
    }
    false
}

fn walk_op(op: &cst::OperationDefinition, schema: &Schema) -> Option<Op> {
    let keyword = match op.operation_type() {
        Some(ty) if ty.mutation_token().is_some() => "mutation",
        Some(ty) if ty.subscription_token().is_some() => "subscription",
        _ => "query",
    };
    let root = match keyword {
        "mutation" => schema.mutation_type(),
        "subscription" => schema.subscription_type(),
        _ => schema.query_type(),
    };
    let name = op.name().map(|n| n.text().to_string());
    Some(Op {
        root: root.map(str::to_owned),
        keyword,
        label: name.clone().unwrap_or_else(|| keyword.to_owned()),
        protected: name.is_some()
            || op.variable_definitions().is_some()
            || op.directives().is_some(),
        range: tight(op.syntax()),
        set: walk_set(&op.selection_set()?),
    })
}

fn walk_set(set: &cst::SelectionSet) -> Set {
    let start = |tok: Option<apollo_parser::SyntaxToken>| {
        tok.map_or(0, |t| usize::from(t.text_range().start()))
    };
    Set {
        open: start(set.l_curly_token()),
        close: start(set.r_curly_token()),
        items: set.selections().map(|s| walk_sel(&s)).collect(),
    }
}

fn walk_sel(sel: &cst::Selection) -> Sel {
    let (name, protected, set) = match sel {
        cst::Selection::Field(f) => (
            f.alias()
                .is_none()
                .then(|| f.name().map(|n| n.text().to_string()))
                .flatten(),
            f.arguments().is_some() || f.directives().is_some(),
            f.selection_set().map(|s| walk_set(&s)),
        ),
        cst::Selection::FragmentSpread(f) => (None, f.directives().is_some(), None),
        cst::Selection::InlineFragment(f) => (None, f.directives().is_some(), None),
    };
    Sel {
        name,
        protected,
        range: tight(sel.syntax()),
        set,
    }
}

/// The field chain for path[1..] when path[0] is a set root and each name is a field of
/// the previous field's named type; Some(vec![]) for a root-only path.
pub(crate) fn resolve<'a>(schema: &'a Schema, path: &FieldPath) -> Option<Vec<&'a Field>> {
    let (root, rest) = path.names().split_first()?;
    keyword(schema, root)?;
    let mut ty = schema.type_named(root)?;
    let mut fields = Vec::with_capacity(rest.len());
    for (i, name) in rest.iter().enumerate() {
        let field = ty.fields().iter().find(|f| f.name() == name)?;
        fields.push(field);
        if i + 1 < rest.len() {
            ty = schema.type_named(field.ty().named())?;
        }
    }
    Some(fields)
}

/// The operation keyword for a root type name, checked in the order query, mutation, subscription.
pub(crate) fn keyword(schema: &Schema, root: &str) -> Option<&'static str> {
    [
        (schema.query_type(), "query"),
        (schema.mutation_type(), "mutation"),
        (schema.subscription_type(), "subscription"),
    ]
    .into_iter()
    .find_map(|(ty, kw)| (ty == Some(root)).then_some(kw))
}

/// The byte range from a node's first to its last significant token, so trivia the parser
/// attached to either edge never widens it.
pub(crate) fn tight(node: &SyntaxNode) -> Range<usize> {
    let mut tokens = node
        .descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
        .filter(|t| {
            !matches!(
                t.kind(),
                SyntaxKind::WHITESPACE | SyntaxKind::COMMA | SyntaxKind::COMMENT | SyntaxKind::EOF
            )
        });
    let Some(first) = tokens.next() else {
        let r = node.text_range();
        return usize::from(r.start())..usize::from(r.start());
    };
    let last = tokens.last().unwrap_or_else(|| first.clone());
    usize::from(first.text_range().start())..usize::from(last.text_range().end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Schema {
        Schema::from_sdl("type Query { me: User v: Int } type User { id: ID name: String }")
            .unwrap()
    }

    fn path(p: &str) -> FieldPath {
        FieldPath::new(p.split('.'))
    }

    // Ranges must exclude the whitespace, commas and comments the parser hangs on nodes.
    #[test]
    fn ranges_are_tight_around_significant_tokens() {
        let s = schema();
        let q = "# head\n query  { me { id , # c\n name, } ,\n v }  # tail\n";
        let o = query_outline(q, &s).unwrap();
        let op = o.op().unwrap();
        assert_eq!(&q[op.range.clone()], &q[8..q.find("}  #").unwrap() + 1]);
        assert_eq!(&q[op.set.open..=op.set.open], "{");
        assert_eq!(&q[op.set.close..=op.set.close], "}");
        let chain = o.chain(&path("Query.me.id")).unwrap();
        assert_eq!(&q[chain[0].range.clone()], "me { id , # c\n name, }");
        assert_eq!(&q[chain[1].range.clone()], "id");
        let name = o.chain(&path("Query.me.name")).unwrap();
        assert_eq!(&q[name[1].range.clone()], "name");
        assert!(
            matches!(o.cut(&path("Query.me.id")), Some(Cut::Field(f)) if f.range == chain[1].range)
        );
    }

    #[test]
    fn the_cut_climbs_through_unprotected_only_children() {
        let s = schema();
        let q = "{ me { id } v }";
        let o = query_outline(q, &s).unwrap();
        let Some(Cut::Field(f)) = o.cut(&path("Query.me.id")) else {
            panic!("expected a field cut");
        };
        assert_eq!(&q[f.range.clone()], "me { id }");
        let lone = query_outline("{ me { id } }", &s).unwrap();
        assert!(matches!(
            lone.cut(&path("Query.me.id")),
            Some(Cut::Operation(_))
        ));
        assert!(lone.cut(&path("Query")).is_none());
    }
}
