//! Completion and hover for the GraphQL variables editor, from the query's variable
//! definitions and, when loaded, the schema's input types.

// The completion entry points that use the locator land next; until then only its tests do.
#[cfg_attr(not(test), allow(dead_code))]
mod locate;
