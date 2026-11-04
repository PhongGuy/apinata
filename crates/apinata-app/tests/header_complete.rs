use apinata_app::header_complete::{Hint, accept, offers};

fn names(present: &[&str]) -> Hint {
    Hint::Name {
        present: present.iter().map(|s| s.to_string()).collect(),
    }
}

fn value(of: &str) -> Hint {
    Hint::Value {
        name: of.to_owned(),
    }
}

#[test]
fn a_prefix_ranks_before_a_substring_and_table_order_breaks_ties() {
    let got = offers(&names(&[]), "con");
    let at = |n: &str| got.iter().position(|g| g == n).unwrap();
    assert!(at("Content-Type") < at("Content-Length"));
    assert!(at("Content-Length") < at("Connection"));
    assert!(got.iter().all(|g| g.to_lowercase().contains("con")));
    // "type" is only a substring of Content-Type, which still matches.
    let by_type = offers(&names(&[]), "type");
    assert_eq!(by_type[0], "Content-Type");
}

#[test]
fn matching_ignores_the_case_typed() {
    assert_eq!(offers(&names(&[]), "CONTENT-ty"), ["Content-Type"]);
}

#[test]
fn names_already_in_the_table_rank_last() {
    let got = offers(&names(&["content-type"]), "con");
    assert_eq!(got.last().unwrap(), "Content-Type");
    assert_eq!(got[0], "Content-Length");
}

#[test]
fn the_text_already_typed_in_full_is_not_offered_back() {
    assert!(!offers(&names(&[]), "Accept").contains(&"Accept".to_owned()));
    assert!(
        !offers(&value("Content-Type"), "application/json")
            .contains(&"application/json".to_owned())
    );
}

#[test]
fn nothing_is_offered_for_an_empty_cell() {
    assert!(offers(&names(&[]), "").is_empty());
    assert!(offers(&value("Content-Type"), "").is_empty());
}

#[test]
fn values_follow_the_row_name_without_regard_to_case() {
    let got = offers(&value("content-type"), "app");
    assert_eq!(got[0], "application/json");
    assert!(offers(&value("X-Custom"), "app").is_empty());
}

#[test]
fn accepting_replaces_the_whole_cell_with_the_canonical_text() {
    let (range, text, cursor) = accept("con", "Content-Type");
    assert_eq!(range, 0..3);
    assert_eq!(text, "Content-Type");
    assert_eq!(cursor, "Content-Type".len());
}
