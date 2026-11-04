//! Searching a collection's loaded requests: fields, snippets, the cap and secret hygiene.

use crate::common::write_tree;
use apinata_core::{
    Collection, MAX_PAGE_LIMIT, PageError, SEARCH_HIT_CAP, SearchError, SearchField, SearchOptions,
};
use tempfile::TempDir;

const ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\n";

fn request(name: &str, url: &str, rest: &str) -> String {
    format!("info:\n  name: {name}\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{rest}")
}

fn open(files: &[(&str, &str)]) -> (TempDir, Collection) {
    let mut all = vec![("opencollection.yml", ROOT)];
    all.extend_from_slice(files);
    let dir = write_tree(&all);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

#[test]
fn matches_name_url_header_value_and_body_case_insensitively() {
    let rest = "  headers:\n    - name: X-Trace\n      value: Needle-Header\n  body:\n    type: json\n    data: '{\"k\": \"NEEDLE body\"}'\n";
    let (_d, c) = open(&[
        (
            "a.yml",
            &request("Find the Needle", "https://x.test/needle", rest),
        ),
        ("b.yml", &request("other", "https://x.test/none", "")),
    ]);
    let found = c.search("nEeDlE");
    let fields: Vec<_> = found.hits.iter().map(|h| h.field).collect();
    assert_eq!(
        fields,
        [
            SearchField::Name,
            SearchField::Url,
            SearchField::HeaderValue,
            SearchField::Body
        ]
    );
    assert!(found.hits.iter().all(|h| h.path.as_str() == "a.yml"));
    assert!(
        found
            .hits
            .iter()
            .all(|h| h.request_name == "Find the Needle")
    );
    assert!(!found.capped);
}

#[test]
fn snippet_is_one_line_around_the_match_with_a_valid_range() {
    let body = "line one\\nline two has the NEEDLE here\\nline three";
    let rest = format!("  body:\n    type: text\n    data: \"{body}\"\n");
    let (_d, c) = open(&[("a.yml", &request("a", "https://x.test/", &rest))]);
    let hit = &c.search("needle").hits[0];
    assert_eq!(hit.field, SearchField::Body);
    assert!(!hit.snippet.contains('\n'), "{:?}", hit.snippet);
    assert_eq!(&hit.snippet[hit.match_range.clone()], "NEEDLE");
}

#[test]
fn snippet_window_is_char_boundary_safe() {
    let long = format!("{}needle{}", "é".repeat(200), "ü".repeat(200));
    let (_d, c) = open(&[("a.yml", &request(&long, "https://x.test/", ""))]);
    let hit = &c.search("NEEDLE").hits[0];
    assert!(hit.snippet.len() < long.len());
    assert_eq!(&hit.snippet[hit.match_range.clone()], "needle");
}

#[test]
fn blank_query_finds_nothing() {
    let (_d, c) = open(&[("a.yml", &request("a", "https://x.test/", ""))]);
    assert!(c.search("").hits.is_empty());
    assert!(c.search("  \t").hits.is_empty());
}

#[test]
fn hits_stop_at_the_cap_and_say_so() {
    let files: Vec<(String, String)> = (0..SEARCH_HIT_CAP + 5)
        .map(|i| {
            (
                format!("r{i:04}.yml"),
                request("hit", "https://x.test/", ""),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_d, c) = open(&refs);
    let found = c.search("hit");
    assert_eq!(found.hits.len(), SEARCH_HIT_CAP);
    assert!(found.capped);
    let exact = c.search("https://x.test/");
    assert_eq!(exact.hits.len(), SEARCH_HIT_CAP);
}

#[test]
fn exactly_the_cap_is_not_capped() {
    let files: Vec<(String, String)> = (0..SEARCH_HIT_CAP)
        .map(|i| {
            (
                format!("r{i:04}.yml"),
                request("hit", "https://x.test/", ""),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_d, c) = open(&refs);
    let found = c.search("hit");
    assert_eq!(found.hits.len(), SEARCH_HIT_CAP);
    assert!(!found.capped);
}

#[test]
fn order_is_folders_first_then_requests_then_field_order() {
    let (_d, c) = open(&[
        ("z/inner.yml", &request("x", "https://x.test/", "")),
        ("a.yml", &request("x", "https://x.test/x", "")),
    ]);
    let seen: Vec<_> = c
        .search("x")
        .hits
        .iter()
        .map(|h| (h.path.as_str().to_owned(), h.field))
        .collect();
    let again: Vec<_> = c
        .search("x")
        .hits
        .iter()
        .map(|h| (h.path.as_str().to_owned(), h.field))
        .collect();
    assert_eq!(seen, again);
    assert_eq!(seen[0], ("z/inner.yml".to_owned(), SearchField::Name));
    let names: Vec<_> = seen.iter().map(|(p, _)| p.as_str()).collect();
    assert!(
        names.iter().position(|p| *p == "a.yml") > names.iter().position(|p| *p == "z/inner.yml")
    );
}

#[test]
fn placeholders_match_as_text_and_secret_values_never_do() {
    let (dir, c) = open(&[
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - secret: true\n    name: token\n",
        ),
        ("a.yml", &request("a", "https://x.test/{{token}}", "")),
    ]);
    std::fs::write(dir.path().join(".env"), "token=s3cr3t-from-env\n").unwrap();
    assert!(c.search("s3cr3t-from-env").hits.is_empty());
    assert_eq!(c.search("{{token}}").hits[0].field, SearchField::Url);
}

#[test]
fn auth_secrets_are_not_searched() {
    let rest = "  auth:\n    type: bearer\n    token: hunter2-auth-secret\n";
    let basic = "  auth:\n    type: basic\n    username: ann\n    password: pw-auth-secret\n";
    let (_d, c) = open(&[
        ("a.yml", &request("a", "https://x.test/", rest)),
        ("b.yml", &request("b", "https://x.test/", basic)),
    ]);
    assert!(c.search("auth-secret").hits.is_empty());
}

fn opts(case_sensitive: bool, regex: bool) -> SearchOptions {
    SearchOptions::default()
        .with_case_sensitive(case_sensitive)
        .with_regex(regex)
}

fn sample() -> (TempDir, Collection) {
    open(&[(
        "a.yml",
        &request("Alpha Beta", "https://x.test/alpha-42", ""),
    )])
}

#[test]
fn default_options_equal_plain_search() {
    let (_d, c) = sample();
    assert_eq!(SearchOptions::default(), opts(false, false));
    for q in ["ALPHA", "alpha", "  beta ", "", "zzz", "a.b"] {
        assert_eq!(
            c.search_with(q, &SearchOptions::default()).expect("ok"),
            c.search(q)
        );
    }
}

#[test]
fn case_sensitive_literal_distinguishes_case() {
    let (_d, c) = sample();
    let hit = |q: &str| c.search_with(q, &opts(true, false)).expect("ok").hits;
    assert!(hit("alpha").iter().all(|h| h.field == SearchField::Url));
    assert_eq!(hit("alpha").len(), 1);
    assert_eq!(hit("Alpha").len(), 1);
    assert_eq!(hit("Alpha")[0].field, SearchField::Name);
    assert!(hit("ALPHA").is_empty());
}

#[test]
fn regex_matches_and_reports_first_match_range() {
    let (_d, c) = sample();
    let found = c.search_with(r"alpha-\d+", &opts(true, true)).expect("ok");
    assert_eq!(found.hits.len(), 1);
    let h = &found.hits[0];
    assert_eq!(h.field, SearchField::Url);
    assert_eq!(&h.snippet[h.match_range.clone()], "alpha-42");
    // The same pattern as a literal finds nothing.
    assert!(
        c.search_with(r"alpha-\d+", &opts(true, false))
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn regex_case_flag_controls_case() {
    let (_d, c) = sample();
    let sensitive = c.search_with("^alpha", &opts(true, true)).expect("ok");
    assert!(sensitive.hits.is_empty());
    let folded = c.search_with("^alpha", &opts(false, true)).expect("ok");
    assert_eq!(folded.hits.len(), 1);
    assert_eq!(folded.hits[0].field, SearchField::Name);
}

#[test]
fn invalid_pattern_is_an_error_and_plain_search_stays_empty() {
    let (_d, c) = sample();
    let err = c.search_with("(", &opts(false, true)).unwrap_err();
    assert!(matches!(err, SearchError::InvalidPattern(_)));
    assert!(!err.to_string().is_empty());
    // A literal "(" is fine without regex mode.
    assert!(c.search_with("(", &opts(false, false)).is_ok());
}

#[test]
fn zero_length_regex_matches_produce_no_hits() {
    let (_d, c) = sample();
    for pat in ["q*", "^", "\\b", ""] {
        let found = c.search_with(pat, &opts(false, true)).expect("ok");
        assert!(found.hits.is_empty(), "pattern {pat:?}");
    }
    // A later non-empty match still counts when an earlier one is empty.
    let found = c.search_with("42|z*", &opts(false, true)).expect("ok");
    assert_eq!(found.hits.len(), 1);
}

/// 1200 requests whose URL holds the needle, named so the tree order is the name order.
fn many() -> (TempDir, Collection, Vec<String>) {
    let owned: Vec<(String, String)> = (0..1200)
        .map(|i| {
            (
                format!("r{i:04}.yml"),
                request(&format!("r{i:04}"), "http://x/needle", ""),
            )
        })
        .collect();
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (dir, c) = open(&files);
    let names = owned.iter().map(|(p, _)| p.clone()).collect();
    (dir, c, names)
}

fn page_paths(c: &Collection, offset: usize, limit: usize) -> (Vec<String>, usize, bool) {
    let options = SearchOptions::default()
        .with_offset(offset)
        .with_limit(limit);
    let page = c.search_page("needle", &options).unwrap();
    let paths = page
        .hits
        .iter()
        .map(|h| h.path.as_str().to_owned())
        .collect();
    (paths, page.total, page.has_more)
}

#[test]
fn pages_walk_past_the_cap_and_concatenate_to_the_full_ordered_list() {
    let (_d, c, names) = many();
    let mut all = Vec::new();
    for (offset, len, more) in [(0, 500, true), (500, 500, true), (1000, 200, false)] {
        let (paths, total, has_more) = page_paths(&c, offset, 500);
        assert_eq!(
            (paths.len(), total, has_more),
            (len, 1200, more),
            "{offset}"
        );
        all.extend(paths);
    }
    assert_eq!(all, names);
    // The first page is what the capped search returns today.
    let capped: Vec<String> = c
        .search("needle")
        .hits
        .iter()
        .map(|h| h.path.as_str().to_owned())
        .collect();
    assert_eq!(capped, names[..500]);
}

#[test]
fn a_search_offset_past_the_end_is_an_empty_page() {
    let (_d, c, _) = many();
    let (paths, total, has_more) = page_paths(&c, 5000, 10);
    assert!(paths.is_empty());
    assert_eq!((total, has_more), (1200, false));
}

#[test]
fn a_search_page_refuses_limit_zero_and_clamps_a_large_limit() {
    let (_d, c, _) = open_one();
    let zero = SearchOptions::default().with_limit(0);
    assert_eq!(
        c.search_page("needle", &zero).unwrap_err(),
        SearchError::Page(PageError::ZeroLimit)
    );
    let big = SearchOptions::default().with_limit(9999);
    let page = c.search_page("needle", &big).unwrap();
    assert_eq!(page.limit, MAX_PAGE_LIMIT);
    assert_eq!((page.total, page.has_more), (1, false));
}

fn open_one() -> (TempDir, Collection, ()) {
    let (d, c) = open(&[("a.yml", &request("a", "http://x/needle", ""))]);
    (d, c, ())
}
