use crate::hostile::hostile_yaml;
use apinata_format::{MAX_EXPANDED_EVENTS, MAX_NESTING, Refusal, bound_expansion, prepass};
use yaml_serde::Value;

#[test]
fn the_bounds_are_the_documented_numbers() {
    assert_eq!(MAX_EXPANDED_EVENTS, 2_000_000);
    assert_eq!(MAX_NESTING, 128);
}

#[test]
fn alias_expansion_counts_against_the_budget() {
    let text = hostile_yaml();
    assert!(text.len() < 50_000);
    assert_eq!(prepass(&text, 2_000_000), Err(Refusal::TooManyEvents));
}

#[test]
fn redefined_and_scalar_anchors_are_sized_by_their_latest_definition() {
    // *s is one event; the second &a is 4 events (start, 2 scalars, end), reused twice.
    let text = "s: &s 1\na: &a [1]\nb: *s\na2: &a [1, 2]\nc: [*a, *a]\n";
    assert_eq!(prepass(text, 40), Ok(()));
    assert_eq!(prepass(text, 8), Err(Refusal::TooManyEvents));
}

#[test]
fn modest_aliases_pass() {
    let text = "base: &b {a: 1, b: 2}\nx: *b\ny: *b\n";
    assert_eq!(prepass(text, 100), Ok(()));
}

#[test]
fn nesting_past_the_limit_is_refused_at_an_offset() {
    let deep = format!(
        "{}{}",
        "[".repeat(MAX_NESTING + 1),
        "]".repeat(MAX_NESTING + 1)
    );
    assert!(matches!(prepass(&deep, u64::MAX), Err(Refusal::At { .. })));
    let ok = format!("{}{}", "[".repeat(MAX_NESTING), "]".repeat(MAX_NESTING));
    assert_eq!(prepass(&ok, u64::MAX), Ok(()));
}

#[test]
fn bound_expansion_names_a_bomb_too_large() {
    assert_eq!(
        bound_expansion(&hostile_yaml()),
        Err("file is too large: more than 2000000 values once aliases expand".to_owned())
    );
}

#[test]
fn bound_expansion_fails_closed_past_a_prepass_error() {
    // A tab-led quoted continuation is a syntax the pre-pass rejects but yaml_serde loads, so
    // the bomb after it would go uncounted.
    let tabbed = "k: \"a\n\tb\"\n";
    assert!(yaml_serde::from_str::<Value>(tabbed).is_ok());
    assert!(bound_expansion(tabbed).is_ok());
    let bomb = format!("{tabbed}{}", hostile_yaml());
    assert_eq!(
        bound_expansion(&bomb),
        Err("file has aliases and a syntax the safety check cannot follow".to_owned())
    );
}
