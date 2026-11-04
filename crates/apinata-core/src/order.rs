//! Bruno's writer key order for request files, from `stringifyHttpRequest.ts`.
//! Each slice lists the keys of one mapping in the order Bruno writes them.

/// Top-level keys.
pub(crate) const TOP: &[&str] = &[
    "info", "http", "graphql", "runtime", "settings", "examples", "docs", "app",
];

/// Keys of `info`.
pub(crate) const INFO: &[&str] = &["name", "type", "seq", "tags", "description"];

/// Keys of `http`, and of `graphql`, which shares them.
pub(crate) const HTTP: &[&str] = &["method", "url", "headers", "params", "body", "auth"];

/// Keys of `runtime`.
pub(crate) const RUNTIME: &[&str] = &["variables", "scripts", "assertions", "actions"];

/// Keys of `settings`.
pub(crate) const SETTINGS: &[&str] = &[
    "encodeUrl",
    "timeout",
    "followRedirects",
    "maxRedirects",
    "forwardAuthorizationHeader",
    "omitHeaders",
];

/// Keys of one entry of `examples`.
pub(crate) const EXAMPLE: &[&str] = &["name", "description", "request", "response"];

/// The writer order of the children of top-level `key`; empty when Bruno's order is not modelled.
pub(crate) fn children_of(key: &str) -> &'static [&'static str] {
    match key {
        "info" => INFO,
        "http" | "graphql" => HTTP,
        "runtime" => RUNTIME,
        "settings" => SETTINGS,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_are_looked_up_by_top_level_key() {
        assert_eq!(children_of("runtime")[0], "variables");
        assert_eq!(children_of("settings")[5], "omitHeaders");
        assert_eq!(children_of("info"), INFO);
        assert_eq!(children_of("http"), HTTP);
        assert_eq!(children_of("graphql"), HTTP);
        assert!(children_of("docs").is_empty());
    }

    #[test]
    fn every_modelled_key_is_a_top_level_key() {
        for key in ["info", "http", "graphql", "runtime", "settings"] {
            assert!(TOP.contains(&key));
        }
    }
}
