use apinata_core::COMPARE_BODY_LIMIT;

const NOTE: &str = include_str!("fixtures/reference/2026-10-18-m11b-compare-cap.md");

#[test]
fn the_cap_equals_the_number_in_the_research_note() {
    let bytes: usize = NOTE
        .lines()
        .find_map(|l| l.strip_prefix("COMPARE_BODY_LIMIT = "))
        .and_then(|rest| rest.trim().strip_suffix(" bytes"))
        .expect("a line `COMPARE_BODY_LIMIT = <n> bytes`")
        .parse()
        .unwrap();
    assert_eq!(COMPARE_BODY_LIMIT, bytes);
}

#[test]
fn the_cap_stays_within_the_history_body_limit() {
    const { assert!(COMPARE_BODY_LIMIT <= 5 * 1024 * 1024) };
}
