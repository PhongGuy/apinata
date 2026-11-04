//! The History palette's search grammar: name, method, scrubbed URL and status terms.

use std::ops::Range;
use std::path::PathBuf;
use std::time::Instant;

use apinata_app::history_palette::{
    HISTORY_MAX, HistoryEntry, HistoryMatch, HistorySearch, UNSAVED_TAG, filter_history,
};
use apinata_core::ItemPath;

fn entry(name: &str, method: &str, url: &str, status: u16) -> HistoryEntry {
    let target = match (method.is_empty(), url.is_empty()) {
        (_, true) => String::new(),
        (true, false) => url.to_owned(),
        (false, false) => format!("{method} {url}"),
    };
    HistoryEntry {
        root: PathBuf::from("/c"),
        path: ItemPath::new("a.yml").unwrap(),
        seq: 0,
        label: name.to_owned(),
        detail: format!("{status} · 1 min ago"),
        target,
        tag: None,
        method: method.to_owned(),
        url: url.to_owned(),
        status,
    }
}

// A draft's send shows its target as the name and has no target column.
fn draft(method: &str, url: &str, status: u16) -> HistoryEntry {
    let mut e = entry("", method, url, status);
    e.label = std::mem::take(&mut e.target);
    e.tag = Some(UNSAVED_TAG);
    e
}

fn rows() -> Vec<HistoryEntry> {
    vec![
        entry("List orders", "GET", "https://api.example.com/orders", 200),
        entry(
            "Create post",
            "PUT",
            "https://api.example.com/articles",
            201,
        ),
        entry("Get item", "GET", "https://api.example.com/posts/7", 500),
        entry("Item", "POST", "https://api.example.com/items/500", 404),
        entry("Health", "HEAD", "https://api.example.com/health", 302),
    ]
}

// A highlight range; a lone `a..b` in a list reads to clippy as a mistaken range literal.
fn span(start: usize, end: usize) -> Range<usize> {
    start..end
}

fn hits(query: &str) -> Vec<usize> {
    filter_history(&rows(), query)
        .into_iter()
        .map(|m| m.index)
        .collect()
}

#[test]
fn a_plain_term_matches_the_name() {
    assert_eq!(hits("orders"), [0]);
}

#[test]
fn a_plain_term_matches_the_url() {
    assert_eq!(hits("/articles"), [1]);
}

#[test]
fn digits_match_the_status_and_the_url() {
    assert_eq!(hits("500"), [2, 3]);
}

#[test]
fn a_status_class_matches_every_status_in_it() {
    assert_eq!(hits("2xx"), [0, 1]);
    assert_eq!(hits("4XX"), [3]);
    assert_eq!(hits("3xx"), [4]);
}

#[test]
fn a_method_word_matches_the_method_and_never_the_url() {
    // "post" is in /posts/7, but only the POST send and the name "Create post" count.
    assert_eq!(hits("post"), [1, 3]);
}

#[test]
fn a_method_word_matches_the_method_exactly() {
    assert_eq!(hits("head"), [4]);
    assert_eq!(hits("get"), [0, 2]);
}

#[test]
fn every_term_must_match() {
    assert_eq!(hits("get 5xx"), [2]);
    assert_eq!(hits("item 404"), [3]);
    assert!(hits("orders 5xx").is_empty());
}

#[test]
fn matching_ignores_case() {
    assert_eq!(hits("LIST Orders"), [0]);
    assert_eq!(hits("API.EXAMPLE.COM/HEALTH"), [4]);
}

#[test]
fn a_draft_method_word_never_matches_its_url() {
    let entries = [draft("GET", "https://h/posts", 200)];
    assert!(filter_history(&entries, "post").is_empty());
    assert_eq!(filter_history(&entries, "posts").len(), 1);
}

#[test]
fn the_empty_query_lists_the_first_rows_without_highlights() {
    let many: Vec<HistoryEntry> = (0..HISTORY_MAX + 20)
        .map(|i| entry("n", "GET", &format!("https://h/{i}"), 200))
        .collect();
    let got = filter_history(&many, "  ");
    assert_eq!(got.len(), HISTORY_MAX);
    assert!(got.iter().enumerate().all(|(i, m)| m.index == i));
    assert!(
        got.iter()
            .all(|m| m.name.is_empty() && m.target.is_empty() && m.detail.is_empty())
    );
}

#[test]
fn the_cap_applies_after_filtering() {
    let mut many: Vec<HistoryEntry> = (0..HISTORY_MAX * 2)
        .map(|i| entry("n", "GET", &format!("https://h/filler/{i}"), 200))
        .collect();
    many.push(entry("n", "GET", "https://h/needle", 200));
    let got = filter_history(&many, "needle");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].index, HISTORY_MAX * 2);
    let all = filter_history(&many, "filler");
    assert_eq!(all.len(), HISTORY_MAX);
    assert_eq!(all.last().unwrap().index, HISTORY_MAX - 1);
}

#[test]
fn a_name_hit_highlights_the_name() {
    let got = filter_history(&rows(), "ORDERS");
    let want = HistoryMatch {
        index: 0,
        name: vec![span(5, 11)],
        // "GET https://api.example.com/orders": the URL starts at byte 4.
        target: vec![span(28, 34)],
        detail: vec![],
    };
    assert_eq!(got, [want]);
}

#[test]
fn a_method_or_status_term_highlights_the_method_and_the_status() {
    let got = filter_history(&rows(), "head 3xx");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].target, [span(0, 4)]);
    assert_eq!(got[0].detail, [span(0, 3)]);
    assert!(got[0].name.is_empty());
}

#[test]
fn a_draft_url_hit_highlights_its_name() {
    let entries = [draft("GET", "https://h/posts", 200)];
    let got = filter_history(&entries, "get posts");
    assert_eq!(got[0].name, [0..3, 14..19]);
    assert!(got[0].target.is_empty());
}

#[test]
fn overlapping_hits_merge_into_one_range() {
    let got = filter_history(&rows(), "order orders");
    assert_eq!(got[0].name, [span(5, 11)]);
}

#[test]
fn a_name_outside_ascii_gives_ranges_that_slice() {
    let entries = [entry("Übersicht café", "GET", "https://h/a", 200)];
    let got = filter_history(&entries, "caf");
    assert_eq!(got.len(), 1);
    let name = &entries[0].label;
    assert_eq!(&name[got[0].name[0].clone()], "caf");
}

#[test]
fn a_name_outside_ascii_matches_in_any_case() {
    let entries = [entry("Übersicht café", "GET", "https://h/a", 200)];
    let got = filter_history(&entries, "übersicht CAFÉ");
    assert_eq!(got.len(), 1);
    let name = &entries[0].label;
    let parts: Vec<&str> = got[0].name.iter().map(|r| &name[r.clone()]).collect();
    assert_eq!(parts, ["Übersicht", "café"]);
}

// A word-final sigma folds the same in the query as in the name, so either case matches.
#[test]
fn a_final_sigma_matches_in_either_case() {
    let entries = [entry("ΟΔΟΣ", "GET", "https://h/a", 200)];
    assert_eq!(filter_history(&entries, "οδος").len(), 1);
    assert_eq!(filter_history(&entries, "ΟΔΟΣ").len(), 1);
}

// The palette's Enter can come before the frame that draws a new query's rows, so the
// search it reads answers the query it is asked, not the one it answered last.
#[test]
fn the_search_answers_the_query_it_is_asked() {
    let search = HistorySearch::new(rows());
    assert_eq!(search.matches("").len(), 5);
    let got = search.matches("post");
    let names: Vec<&str> = got
        .iter()
        .map(|m| search.entries()[m.index].label.as_str())
        .collect();
    assert_eq!(names, ["Create post", "Item"]);
    assert_eq!(search.matches("").len(), 5);
}

// 5000 sends, three terms, every row compared, best of five runs. The bound is generous so
// a loaded machine never flakes; it catches a search gone quadratic.
#[test]
fn filtering_five_thousand_sends_stays_fast() {
    let many: Vec<HistoryEntry> = (0..5000)
        .map(|i| {
            entry(
                &format!("Request number {i}"),
                ["GET", "POST", "PUT"][i % 3],
                &format!("https://api.example.com/v1/things/{i}?page={}", i % 7),
                [200, 404, 500][i % 3],
            )
        })
        .collect();
    let took = (0..5)
        .map(|_| {
            let start = Instant::now();
            assert!(filter_history(&many, "things 5xx post").is_empty());
            start.elapsed()
        })
        .min()
        .unwrap();
    eprintln!("filter over 5000 sends took {took:?}");
    assert!(took.as_millis() < 100, "took {took:?}");
}
