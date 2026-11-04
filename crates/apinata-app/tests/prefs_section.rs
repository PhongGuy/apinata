//! The Preferences section filter: rows match on label, extra terms or the section title.

use apinata_app::prefs_section::{PrefsSection, matches};
use gpui_kit::div;

fn network() -> PrefsSection {
    PrefsSection::new("prefs-network", "Network")
        .row("pref-verify-tls", "Verify TLS certificates", div())
        .row("pref-body-cap", "Body cap (MB, blank for none)", div())
        .note(div())
}

fn ids(section: &PrefsSection, query: &str) -> Vec<String> {
    section
        .matching_rows(query)
        .iter()
        .map(|id| id.to_string())
        .collect()
}

#[test]
fn a_blank_query_matches_every_row() {
    assert!(matches("", "anything"));
    assert!(matches("   ", "anything"));
    assert_eq!(ids(&network(), ""), ["pref-verify-tls", "pref-body-cap"]);
}

#[test]
fn rows_match_their_label_ignoring_case_and_padding() {
    assert_eq!(ids(&network(), "tls"), ["pref-verify-tls"]);
    assert_eq!(ids(&network(), "BODY CAP"), ["pref-body-cap"]);
    assert_eq!(ids(&network(), "  Tls "), ["pref-verify-tls"]);
}

#[test]
fn the_section_title_matches_every_row() {
    assert_eq!(
        ids(&network(), "netw"),
        ["pref-verify-tls", "pref-body-cap"]
    );
}

#[test]
fn a_query_matching_nothing_hides_the_section() {
    assert!(ids(&network(), "theme").is_empty());
    assert!(network().filtered("theme").is_none());
    assert!(network().filtered("tls").is_some());
}

#[test]
fn matching_is_one_substring_not_words() {
    assert!(matches("ver", "Verify TLS"));
    assert!(matches("y t", "Verify TLS"));
    assert!(!matches("tls verify", "Verify TLS"));
}

#[test]
fn extra_terms_match_like_the_label() {
    let keyboard = PrefsSection::new("prefs-keyboard", "Keyboard")
        .row_with_terms("cmd-save", "Save", ["Ctrl+S"], div())
        .row_with_terms("cmd-close", "Close tab", ["Ctrl+W"], div());
    assert_eq!(ids(&keyboard, "ctrl+s"), ["cmd-save"]);
    assert_eq!(ids(&keyboard, "close"), ["cmd-close"]);
    assert!(ids(&keyboard, "ctrl+q").is_empty());
}
