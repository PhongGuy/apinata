//! The find bar's regex and whole-word matcher, on its own.

use apinata_app::body_find::{FindOptions, Finder, find_in_chunked_with};
use apinata_app::body_text::chunk_lines;
use std::ops::Range;

fn opts(case_sensitive: bool, regex: bool, whole_word: bool) -> FindOptions {
    FindOptions {
        case_sensitive,
        regex,
        whole_word,
    }
}

fn hits(text: &str, query: &str, options: FindOptions) -> Vec<Range<usize>> {
    Finder::new(query, options)
        .expect("valid")
        .expect("non-empty query")
        .find_in_text(text)
}

// The hits as (start, end) pairs, for comparing against a single hit.
fn spans(text: &str, query: &str, options: FindOptions) -> Vec<(usize, usize)> {
    hits(text, query, options)
        .into_iter()
        .map(|r| (r.start, r.end))
        .collect()
}

const REGEX: FindOptions = FindOptions {
    case_sensitive: false,
    regex: true,
    whole_word: false,
};
const WORD: FindOptions = FindOptions {
    case_sensitive: false,
    regex: false,
    whole_word: true,
};

#[test]
fn a_regex_matches_by_pattern() {
    assert_eq!(hits("a1 b22 c", r"\d+", REGEX), [1..2, 4..6]);
    assert_eq!(hits("id: 7, ID: 8", "id", REGEX), [0..2, 7..9]);
}

#[test]
fn a_regex_honours_the_case_toggle() {
    assert_eq!(spans("id ID Id", "id", opts(true, true, false)), [(0, 2)]);
    assert_eq!(hits("Ärger ärger", "ä", REGEX), [0..2, 7..9]);
}

#[test]
fn a_regex_anchors_at_each_line() {
    assert_eq!(hits("ab\nab\nxab", "^ab", REGEX), [0..2, 3..5]);
    assert_eq!(hits("ab\nab\nabx", "ab$", REGEX), [0..2, 3..5]);
}

#[test]
fn an_invalid_regex_is_an_error_not_a_panic() {
    for bad in ["(", "[a-", "a{2,1}", "*", "\\"] {
        assert!(
            Finder::new(bad, REGEX).is_err(),
            "{bad:?} should not compile"
        );
    }
}

#[test]
fn an_empty_query_finds_nothing() {
    assert!(Finder::new("", REGEX).unwrap().is_none());
    assert!(Finder::new("", WORD).unwrap().is_none());
}

#[test]
fn empty_width_matches_are_skipped_and_terminate() {
    assert!(hits("abc\n\ndef", "x*", REGEX).is_empty());
    assert!(hits("abc", "^", REGEX).is_empty());
    assert!(hits("abc", r"\b", REGEX).is_empty());
    // A pattern that can match empty still yields its non-empty hits.
    assert_eq!(hits("aa b aaa", "a*", REGEX), [0..2, 5..8]);
}

#[test]
fn whole_word_needs_non_word_characters_on_both_sides() {
    assert_eq!(hits("cat concat cat_x (cat)", "cat", WORD), [0..3, 18..21]);
    assert_eq!(hits("Cat CAT", "cat", WORD), [0..3, 4..7]);
    assert_eq!(
        spans("Cat CAT cat", "cat", opts(true, false, true)),
        [(8, 11)]
    );
}

// The edges are checked as characters, so a query that starts or ends with punctuation
// still needs non-word neighbours rather than word ones.
#[test]
fn whole_word_checks_the_neighbours_of_a_punctuation_edged_query() {
    assert_eq!(spans("a (x) b(x)c", "(x)", WORD), [(2, 5)]);
}

#[test]
fn whole_word_treats_the_query_literally() {
    assert_eq!(spans("a.b axb", "a.b", WORD), [(0, 3)]);
    assert!(Finder::new("(", WORD).unwrap().is_some());
}

// Word characters are ASCII letters, digits and underscore, as in an ECMAScript `\b`.
#[test]
fn whole_word_uses_ascii_word_characters() {
    assert_eq!(hits("café caf", "caf", WORD), [0..3, 6..9]);
}

// The reference builds its regex from the pattern alone once regex is on.
#[test]
fn whole_word_is_ignored_while_regex_is_on() {
    let both = opts(false, true, true);
    assert_eq!(hits("a1 b22 c3d", r"\w\d+", both), [0..2, 3..6, 7..9]);
}

#[test]
fn hits_are_byte_ranges_on_char_boundaries_in_non_ascii_text() {
    let text = "ü→ \"ünïcode\" 😀ab 😀ab";
    let found = hits(text, "ab", REGEX);
    assert_eq!(found.len(), 2);
    for r in &found {
        assert_eq!(&text[r.clone()], "ab");
    }
    let found = hits(text, "😀.", REGEX);
    assert_eq!(found.len(), 2);
    for r in &found {
        assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
        assert_eq!(&text[r.clone()], "😀a");
    }
    assert_eq!(spans("xün ün", "ün", WORD), [(5, 8)]);
}

#[test]
fn chunked_rows_find_regex_hits_across_a_seam() {
    // Rows of 4: "ab12" "34cd"; the digits run over the seam as one hit.
    let chunked = chunk_lines("ab1234cd", 4);
    let finder = Finder::new(r"\d+", REGEX).unwrap().unwrap();
    let found = find_in_chunked_with(&chunked, &finder);
    assert_eq!(found.matches, [(0, 2..4)]);
    assert_eq!(found.tails, [(0, 1, 0..2)]);
}
