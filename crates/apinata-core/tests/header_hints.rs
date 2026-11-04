use std::collections::HashSet;

use apinata_core::{header_names, header_values};

#[test]
fn names_are_unique_and_capitalised_like_a_header() {
    let names = header_names();
    assert!(names.contains(&"Content-Type"));
    assert!(names.contains(&"Authorization"));
    let lower: HashSet<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    assert_eq!(lower.len(), names.len(), "a name is listed twice");
    for name in names {
        assert!(
            name.split('-').all(|w| w
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit())),
            "{name} is not in canonical casing or is not a valid token"
        );
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}

#[test]
fn values_are_found_without_regard_to_the_name_case() {
    assert!(header_values("Content-Type").contains(&"application/json"));
    assert_eq!(header_values("content-type"), header_values("CONTENT-TYPE"));
    assert!(header_values("Authorization").contains(&"Bearer"));
    assert!(header_values("Accept").contains(&"application/json"));
    assert!(header_values("Cache-Control").contains(&"no-cache"));
}

#[test]
fn a_name_without_known_values_has_none() {
    assert!(header_values("X-Custom").is_empty());
    assert!(header_values("").is_empty());
}
