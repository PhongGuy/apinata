//! Directive definitions shared by both schema readers: the rule that admits a definition and
//! the built-in directives every GraphQL schema has.

use super::model::{Directive, InputValue, TypeRef};
use super::print::is_name;
use super::sdl::BUILT_IN_SCALARS;

/// The GraphQL spec's directive locations.
pub(super) const LOCATIONS: [&str; 19] = [
    "QUERY",
    "MUTATION",
    "SUBSCRIPTION",
    "FIELD",
    "FRAGMENT_DEFINITION",
    "FRAGMENT_SPREAD",
    "INLINE_FRAGMENT",
    "VARIABLE_DEFINITION",
    "SCHEMA",
    "SCALAR",
    "OBJECT",
    "FIELD_DEFINITION",
    "ARGUMENT_DEFINITION",
    "INTERFACE",
    "UNION",
    "ENUM",
    "ENUM_VALUE",
    "INPUT_OBJECT",
    "INPUT_FIELD_DEFINITION",
];

/// Keeps each candidate whose name and argument names are GraphQL names, whose locations
/// are non-empty and all in LOCATIONS, whose argument types satisfy `has_type` or are
/// built-in scalars, and whose name no earlier kept candidate has.
pub(super) fn admit(candidates: Vec<Directive>, has_type: impl Fn(&str) -> bool) -> Vec<Directive> {
    let mut kept: Vec<Directive> = Vec::new();
    for d in candidates {
        // A `__` name is reserved for introspection, and an empty location list cannot print.
        let named = is_name(&d.name) && !d.name.starts_with("__");
        let placed =
            !d.locations.is_empty() && d.locations.iter().all(|l| LOCATIONS.contains(&l.as_str()));
        let args_ok = d.args.iter().all(|a| {
            is_name(&a.name)
                && (BUILT_IN_SCALARS.contains(&a.ty.named.as_str()) || has_type(&a.ty.named))
        });
        if named && placed && args_ok && !kept.iter().any(|k| k.name == d.name) {
            kept.push(d);
        }
    }
    kept
}

/// `own`, then each built-in whose name `own` lacks; the second value is `own.len()`.
pub(super) fn with_built_ins(own: Vec<Directive>) -> (Vec<Directive>, usize) {
    let count = own.len();
    let mut all = own;
    for built_in in built_ins() {
        if !all[..count].iter().any(|d| d.name == built_in.name) {
            all.push(built_in);
        }
    }
    (all, count)
}

/// Whether `d` is a built-in exactly as core would add it: the same arguments (names, types,
/// defaults), locations in any order, and repeatability. Descriptions are not compared, since
/// servers word them differently.
pub(super) fn is_unchanged_built_in(d: &Directive) -> bool {
    let sorted = |l: &[String]| {
        let mut v = l.to_vec();
        v.sort();
        v
    };
    built_ins().iter().any(|b| {
        b.name == d.name
            && b.repeatable == d.repeatable
            && sorted(&b.locations) == sorted(&d.locations)
            && b.args.len() == d.args.len()
            && b.args.iter().zip(&d.args).all(|(x, y)| {
                x.name == y.name
                    && x.ty.display == y.ty.display
                    && x.default_value == y.default_value
            })
    })
}

const REASON: &str = "Explains why this element was deprecated, usually also including a suggestion for how to access supported similar data. Formatted using the Markdown syntax, as specified by [CommonMark](https://commonmark.org/).";

/// One built-in with a single argument given as `[name, named type, display, description]`.
fn built_in(
    name: &str,
    description: &str,
    arg: [&str; 4],
    default: Option<&str>,
    locations: &[&str],
) -> Directive {
    let [arg_name, named, display, arg_description] = arg;
    Directive {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        args: vec![InputValue {
            name: arg_name.to_owned(),
            description: Some(arg_description.to_owned()),
            ty: TypeRef {
                named: named.to_owned(),
                display: display.to_owned(),
            },
            default_value: default.map(str::to_owned),
        }],
        locations: locations.iter().map(|l| (*l).to_owned()).collect(),
        repeatable: false,
    }
}

/// The built-ins in apollo-compiler's order, worded as graphql-js words them.
fn built_ins() -> [Directive; 4] {
    const SELECTION: &[&str] = &["FIELD", "FRAGMENT_SPREAD", "INLINE_FRAGMENT"];
    [
        built_in(
            "skip",
            "Directs the executor to skip this field or fragment when the `if` argument is true.",
            ["if", "Boolean", "Boolean!", "Skipped when true."],
            None,
            SELECTION,
        ),
        built_in(
            "include",
            "Directs the executor to include this field or fragment only when the `if` argument is true.",
            ["if", "Boolean", "Boolean!", "Included when true."],
            None,
            SELECTION,
        ),
        built_in(
            "deprecated",
            "Marks an element of a GraphQL schema as no longer supported.",
            ["reason", "String", "String", REASON],
            Some("\"No longer supported\""),
            &[
                "FIELD_DEFINITION",
                "ARGUMENT_DEFINITION",
                "INPUT_FIELD_DEFINITION",
                "ENUM_VALUE",
            ],
        ),
        built_in(
            "specifiedBy",
            "Exposes a URL that specifies the behavior of this scalar.",
            [
                "url",
                "String",
                "String!",
                "The URL that specifies the behavior of this scalar.",
            ],
            None,
            &["SCALAR"],
        ),
    ]
}
