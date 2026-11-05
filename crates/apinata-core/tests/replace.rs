//! Replacing text across a collection: scope, regex expansion, byte stability and failure
//! handling.

use crate::common::write_tree;
use apinata_core::{Collection, ReplaceScope, SearchField, SearchOptions};
use std::fs;
use tempfile::TempDir;

const ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\n";

fn request(name: &str, url: &str, rest: &str) -> String {
    format!("info:\n  name: {name}\n  type: http\nhttp:\n  method: GET\n  url: {url}\n{rest}")
}

fn open(files: &[(&str, &str)]) -> (TempDir, Collection) {
    let mut all = vec![("opencollection.yml", ROOT)];
    all.extend_from_slice(files);
    let dir = write_tree(&all);
    let c = Collection::open(dir.path()).expect("opens");
    (dir, c)
}

fn disk(dir: &TempDir, rel: &str) -> String {
    fs::read_to_string(dir.path().join(rel)).expect("reads")
}

fn lit() -> SearchOptions {
    SearchOptions::default()
}

#[test]
fn literal_replace_all_covers_every_field_and_match() {
    let rest = "  headers:\n    - name: X-Old\n      value: old and Old\n  body:\n    type: text\n    data: old old\n";
    let (d, mut c) = open(&[("a.yml", &request("old name", "https://x.test/old", rest))]);
    let r = c
        .replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    let text = disk(&d, "a.yml");
    assert!(!text.to_lowercase().contains("old"), "{text}");
    assert!(text.contains("name: new name"));
    assert!(text.contains("X-new"));
    assert!(text.contains("new and new"));
    assert!(text.contains("new new"));
    assert_eq!(r.replaced, 7);
    assert_eq!(r.files_changed, 1);
    assert!(r.skipped_unsaved.is_empty() && r.failed.is_empty());
    assert!(!r.scripts_changed);
    assert!(c.search("old").hits.is_empty());
}

#[test]
fn case_sensitive_leaves_other_cases() {
    let (d, mut c) = open(&[("a.yml", &request("Old old", "https://x.test/", ""))]);
    let opts = lit().with_case_sensitive(true);
    let r = c.replace("old", "new", &opts, ReplaceScope::all()).unwrap();
    assert!(disk(&d, "a.yml").contains("name: Old new"));
    assert_eq!(r.replaced, 1);
}

#[test]
fn regex_expands_groups_and_dollar_forms() {
    let (d, mut c) = open(&[("a.yml", &request("v1-v22", "https://x.test/", ""))]);
    let opts = lit().with_regex(true);
    c.replace(
        "v(\\d)(\\d)?",
        "[$2|$1|$&|$$|${1}x|$9|$z]",
        &opts,
        ReplaceScope::all(),
    )
    .unwrap();
    let text = disk(&d, "a.yml");
    assert!(
        text.contains("[|1|v1|$|1x||$z]-[2|2|v22|$|2x||$z]"),
        "{text}"
    );
}

#[test]
fn zero_length_regex_matches_are_not_replaced() {
    let (d, mut c) = open(&[("a.yml", &request("abc", "https://x.test/", ""))]);
    let opts = lit().with_regex(true);
    let r = c.replace("z*", "!", &opts, ReplaceScope::all()).unwrap();
    assert_eq!(r.replaced, 0);
    assert!(disk(&d, "a.yml").contains("name: abc"));
}

#[test]
fn hit_scope_replaces_one_match_in_that_field_only() {
    let (d, mut c) = open(&[("a.yml", &request("old old", "https://x.test/old", ""))]);
    let hit = c
        .search("old")
        .hits
        .into_iter()
        .find(|h| h.field == SearchField::Name)
        .unwrap();
    let r = c
        .replace("old", "new", &lit(), ReplaceScope::hit(hit))
        .unwrap();
    let text = disk(&d, "a.yml");
    assert!(text.contains("name: new old"), "{text}");
    assert!(text.contains("https://x.test/old"));
    assert_eq!(r.replaced, 1);
    assert_eq!(r.files_changed, 1);
}

#[test]
fn hit_scope_uses_the_hits_key() {
    let rest = "  headers:\n    - name: A\n      value: old\n    - name: B\n      value: old\n";
    let (d, mut c) = open(&[("a.yml", &request("r", "https://x.test/", rest))]);
    let hit = c
        .search("old")
        .hits
        .into_iter()
        .find(|h| h.key.as_deref() == Some("B"))
        .unwrap();
    c.replace("old", "new", &lit(), ReplaceScope::hit(hit))
        .unwrap();
    let text = disk(&d, "a.yml");
    assert_eq!(text.matches("old").count(), 1, "{text}");
    assert!(text.contains("value: new"));
}

#[test]
fn untouched_files_and_regions_keep_their_bytes() {
    // Written the way the saver writes it, with a blank line before the `http` block.
    let canon = |n: &str, u: &str| {
        format!("info:\n  name: {n}\n  type: http\n\nhttp:\n  method: GET\n  url: {u}\n")
    };
    let a = canon("a", "https://x.test/old");
    let b = request("b", "https://x.test/none", "");
    let (d, mut c) = open(&[("a.yml", &a), ("b.yml", &b)]);
    c.replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    assert_eq!(disk(&d, "b.yml").as_bytes(), b.as_bytes());
    assert_eq!(disk(&d, "a.yml"), a.replace("old", "new"));
}

#[test]
fn auth_and_secret_values_are_never_touched() {
    let rest = "  auth:\n    type: bearer\n    token: old-token\n";
    let (d, mut c) = open(&[
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: old\n    value: old\n",
        ),
        ("a.yml", &request("a", "https://x.test/", rest)),
    ]);
    fs::write(d.path().join(".env"), "old=old\n").unwrap();
    let env_before = disk(&d, "environments/dev.yml");
    let r = c
        .replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    assert_eq!(r.replaced, 0);
    assert!(disk(&d, "a.yml").contains("token: old-token"));
    assert_eq!(disk(&d, "environments/dev.yml"), env_before);
    assert_eq!(disk(&d, ".env"), "old=old\n");
}

#[test]
fn unsaved_requests_are_skipped_and_named() {
    let (d, mut c) = open(&[
        ("a.yml", &request("a", "https://x.test/old", "")),
        ("b.yml", &request("b", "https://x.test/old", "")),
    ]);
    let before = disk(&d, "a.yml");
    let path = c.search("old").hits[0].path.clone();
    c.request_mut(&path).unwrap().set_name("edited old");
    let r = c
        .replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    assert_eq!(r.skipped_unsaved, ["edited old"]);
    assert_eq!(r.files_changed, 1);
    assert_eq!(disk(&d, "a.yml"), before);
    assert_eq!(c.request(&path).unwrap().name(), "edited old");
    assert!(c.request(&path).unwrap().is_dirty());
    assert!(disk(&d, "b.yml").contains("https://x.test/new"));
}

#[test]
fn a_setter_failure_skips_the_file_and_reverts_memory() {
    let rest = "  headers:\n    - name: X-Old\n      value: ok\n";
    let (d, mut c) = open(&[
        ("a.yml", &request("old", "https://x.test/", rest)),
        ("b.yml", &request("old b", "https://x.test/", "")),
    ]);
    let before = disk(&d, "a.yml");
    let r = c
        .replace("Old", "bad\nname", &lit(), ReplaceScope::all())
        .unwrap();
    assert_eq!(r.failed.len(), 1, "{r:?}");
    assert_eq!(r.failed[0].0, "old");
    assert_eq!(disk(&d, "a.yml"), before);
    let path = c.search("old").hits[0].path.clone();
    let req = c.request(&path).unwrap();
    assert!(!req.is_dirty());
    assert_eq!(req.name(), "old");
    assert_eq!(req.headers()[0].name, "X-Old");
    assert_eq!(r.files_changed, 1);
    assert!(r.summary().contains("1 failed"), "{}", r.summary());
}

#[test]
fn script_changes_are_reported_and_summarised() {
    let rest = "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        bru.setVar(\"old\", 1);\n";
    let (d, mut c) = open(&[("a.yml", &request("a", "https://x.test/", rest))]);
    let r = c
        .replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    assert!(r.scripts_changed);
    assert!(disk(&d, "a.yml").contains("bru.setVar(\"new\", 1);"));
    assert_eq!(
        r.summary(),
        "Replaced 1 in 1 file; scripts changed, they will ask for trust again"
    );
}

#[test]
fn summary_pluralises_and_names_skips() {
    let (_d, mut c) = open(&[
        ("a.yml", &request("a", "https://x.test/old", "")),
        ("b.yml", &request("b", "https://x.test/old", "")),
        ("c.yml", &request("c", "https://x.test/old", "")),
    ]);
    let path = c.search("old").hits[2].path.clone();
    c.request_mut(&path).unwrap().set_name("dirty");
    let r = c
        .replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    assert_eq!(
        r.summary(),
        "Replaced 2 in 2 files; skipped 1 with unsaved edits"
    );
}

#[test]
fn unicode_text_is_replaced_on_char_boundaries() {
    let (d, mut c) = open(&[("a.yml", &request("Ünï ÅÄÖ ünï", "https://x.test/", ""))]);
    let r = c.replace("ÜNÏ", "é", &lit(), ReplaceScope::all()).unwrap();
    assert_eq!(r.replaced, 2);
    assert!(disk(&d, "a.yml").contains("name: é ÅÄÖ é"));
}

#[test]
fn an_invalid_regex_errors_and_changes_nothing() {
    let a = request("a", "https://x.test/(", "");
    let (d, mut c) = open(&[("a.yml", &a)]);
    let opts = lit().with_regex(true);
    assert!(c.replace("(", "x", &opts, ReplaceScope::all()).is_err());
    assert_eq!(disk(&d, "a.yml"), a);
}

/// The hit for `field` whose snippet holds `snippet`, so a test can pick between repeats.
fn hit_with(c: &Collection, field: SearchField, snippet: &str) -> apinata_core::SearchHit {
    c.search("foo")
        .hits
        .into_iter()
        .find(|h| h.field == field && h.snippet == snippet)
        .expect("hit")
}

#[test]
fn hit_targets_the_right_repeated_header() {
    let rest =
        "  headers:\n    - name: X-A\n      value: foo\n    - name: X-A\n      value: foo bar\n";
    let (d, mut c) = open(&[("a.yml", &request("r", "https://x.test/", rest))]);
    let hit = hit_with(&c, SearchField::HeaderValue, "foo bar");
    c.replace("foo", "baz", &lit(), ReplaceScope::hit(hit))
        .unwrap();
    let text = disk(&d, "a.yml");
    assert!(text.contains("value: foo\n"), "{text}");
    assert!(text.contains("value: baz bar"), "{text}");
}

#[test]
fn hit_targets_the_right_repeated_form_field() {
    let rest = "  body:\n    type: form-urlencoded\n    data:\n      - name: k\n        value: foo\n      - name: k\n        value: foo bar\n";
    let (d, mut c) = open(&[("a.yml", &request("r", "https://x.test/", rest))]);
    let hit = hit_with(&c, SearchField::Body, "foo bar");
    c.replace("foo", "baz", &lit(), ReplaceScope::hit(hit))
        .unwrap();
    let text = disk(&d, "a.yml");
    assert!(text.contains("value: foo\n"), "{text}");
    assert!(text.contains("value: baz bar"), "{text}");
}

#[test]
fn hit_targets_the_right_repeated_multipart_field() {
    let rest = "  body:\n    type: multipart-form\n    data:\n      - name: k\n        type: text\n        value: foo\n      - name: k\n        type: text\n        value: foo bar\n";
    let (d, mut c) = open(&[("a.yml", &request("r", "https://x.test/", rest))]);
    let hit = hit_with(&c, SearchField::Body, "foo bar");
    c.replace("foo", "baz", &lit(), ReplaceScope::hit(hit))
        .unwrap();
    let text = disk(&d, "a.yml");
    assert!(text.contains("value: foo\n"), "{text}");
    assert!(text.contains("value: baz bar"), "{text}");
}

#[test]
fn many_literal_matches_are_all_replaced() {
    let name = "ab ".repeat(20_000);
    let (d, mut c) = open(&[("a.yml", &request(name.trim(), "https://x.test/", ""))]);
    let r = c.replace("AB", "c", &lit(), ReplaceScope::all()).unwrap();
    assert_eq!(r.replaced, 20_000);
    assert!(disk(&d, "a.yml").contains(&"c ".repeat(19_999)));
}

#[test]
fn hit_counts_multipart_files_like_search_does() {
    let rest = "  body:\n    type: multipart-form\n    data:\n      - name: k\n        type: file\n        value: [a.png]\n      - name: k\n        type: text\n        value: foo bar\n";
    let (d, mut c) = open(&[("a.yml", &request("r", "https://x.test/", rest))]);
    let hit = hit_with(&c, SearchField::Body, "foo bar");
    let r = c
        .replace("foo", "baz", &lit(), ReplaceScope::hit(hit))
        .unwrap();
    assert_eq!(r.replaced, 1);
    assert!(disk(&d, "a.yml").contains("value: baz bar"));
}

#[test]
fn hit_replaces_graphql_query_and_variables() {
    let rest = "  body:\n    query: \"{ foo }\"\n    variables: '{\"v\": \"foo\"}'\n";
    let text = format!(
        "info:\n  name: g\n  type: graphql\ngraphql:\n  method: POST\n  url: https://x.test/\n{rest}"
    );
    let (d, mut c) = open(&[("g.yml", &text)]);
    for key in ["query", "variables"] {
        let hit = c
            .search("foo")
            .hits
            .into_iter()
            .find(|h| h.key.as_deref() == Some(key))
            .unwrap();
        let r = c
            .replace("foo", "baz", &lit(), ReplaceScope::hit(hit))
            .unwrap();
        assert_eq!(r.replaced, 1, "{key}");
    }
    assert!(!disk(&d, "g.yml").contains("foo"));
}

#[test]
fn a_stale_hit_replaces_nothing() {
    let (d, mut c) = open(&[("a.yml", &request("foo", "https://x.test/", ""))]);
    let hit = c.search("foo").hits.remove(0);
    c.replace("foo", "bar", &lit(), ReplaceScope::all())
        .unwrap();
    let before = disk(&d, "a.yml");
    let r = c
        .replace("foo", "baz", &lit(), ReplaceScope::hit(hit))
        .unwrap();
    assert_eq!(r.replaced, 0);
    assert_eq!(disk(&d, "a.yml"), before);
}

#[test]
fn changed_paths_follow_tree_order_and_exclude_skipped_and_failed() {
    let bad = "  headers:\n    - name: X-Old\n      value: ok\n";
    let (_d, mut c) = open(&[
        ("a.yml", &request("same", "https://x.test/old", "")),
        ("sub/b.yml", &request("same", "https://x.test/old", "")),
        ("sub/c.yml", &request("dirty", "https://x.test/old", "")),
        ("z.yml", &request("last", "https://x.test/old", "")),
        ("y.yml", &request("other", "https://x.test/old", bad)),
    ]);
    let dirty = c
        .search("old")
        .hits
        .iter()
        .find(|h| h.request_name == "dirty")
        .unwrap()
        .path
        .clone();
    c.request_mut(&dirty).unwrap().set_name("dirty old");
    // The replacement is a bad header name only for the request that has the header.
    let r = c
        .replace("old", "x\ny", &lit(), ReplaceScope::all())
        .unwrap();
    assert_eq!(r.skipped_unsaved, ["dirty old"], "{r:?}");
    assert_eq!(r.failed.len(), 1, "{r:?}");
    let paths: Vec<&str> = r.changed_paths.iter().map(|p| p.as_str()).collect();
    assert_eq!(paths.len(), r.files_changed, "{paths:?}");
    assert!(!paths.contains(&dirty.as_str()), "{paths:?}");
    assert!(!paths.iter().any(|p| p.ends_with("y.yml")), "{paths:?}");
    // Both same-named requests are listed, folders before requests as the sidebar shows.
    assert_eq!(paths.len(), 3, "{paths:?}");
    assert!(paths[0].ends_with("b.yml"), "{paths:?}");
    assert!(paths[1].ends_with("a.yml"), "{paths:?}");
    assert!(paths[2].ends_with("z.yml"), "{paths:?}");
}

/// The file's text from its `params:` line to the end, which every request below ends with.
fn params_block(text: &str) -> &str {
    &text[text.find("  params:").expect("params block")..]
}

#[test]
fn a_url_replace_keeps_an_untouched_params_block_byte_identical() {
    // Path param first and a described query param: re-deriving would reorder and drop both.
    let params = "  params:\n    - name: id\n      value: \"7\"\n      type: path\n    - name: q\n      value: \"1\"\n      type: query\n      description: the page\n";
    let a = request("a", "https://old.test/u/:id?q=1", params);
    let (d, mut c) = open(&[("a.yml", &a)]);
    let r = c
        .replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    assert_eq!(r.replaced, 1);
    let text = disk(&d, "a.yml");
    assert!(text.contains("url: https://new.test/u/:id?q=1"), "{text}");
    assert_eq!(params_block(&text), params, "{text}");
}

#[test]
fn a_url_replace_adds_no_params_block_to_a_file_without_one() {
    let (d, mut c) = open(&[("a.yml", &request("a", "https://old.test/?q=1", ""))]);
    c.replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    let text = disk(&d, "a.yml");
    assert!(text.contains("url: https://new.test/?q=1"), "{text}");
    assert!(!text.contains("params:"), "{text}");
}

#[test]
fn a_replace_inside_the_query_updates_the_params_block_to_match() {
    let params = "  params:\n    - name: q\n      value: old\n      type: query\n";
    let a = request("a", "https://x.test/?q=old", params);
    let (d, mut c) = open(&[("a.yml", &a)]);
    c.replace("old", "new", &lit(), ReplaceScope::all())
        .unwrap();
    let text = disk(&d, "a.yml");
    assert!(text.contains("url: https://x.test/?q=new"), "{text}");
    assert_eq!(
        params_block(&text),
        "  params:\n    - name: q\n      value: new\n      type: query\n",
        "{text}"
    );
}

/// A small deterministic generator, so the property cases below are the same on every run.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % n as u64) as usize
    }

    /// A name from a tiny alphabet so repeats are common, holding the needle in some case
    /// about half the time.
    fn name(&mut self) -> String {
        let mut s: String = (0..1 + self.below(2))
            .map(|_| ['a', '-'][self.below(2)])
            .collect();
        if self.below(2) == 0 {
            let at = self.below(s.len() + 1);
            s.insert_str(at, ["zq", "Zq", "zQ", "ZQ"][self.below(4)]);
        }
        s
    }
}

/// The header names and form field names of the request at `path`, in file order.
fn names_of(c: &Collection, path: &apinata_core::ItemPath) -> (Vec<String>, Vec<String>) {
    let r = c.request(path).expect("request");
    let fields = match r.body() {
        apinata_core::BodySpec::FormUrlEncoded(f) => f.into_iter().map(|f| f.name).collect(),
        other => panic!("{other:?}"),
    };
    (r.headers().into_iter().map(|h| h.name).collect(), fields)
}

/// `name` with its first needle, in any case, swapped for `R`.
fn swap_first(name: &str) -> String {
    let at = name.to_lowercase().find("zq").expect("needle");
    format!("{}R{}", &name[..at], &name[at + 2..])
}

#[test]
fn hits_on_header_and_form_field_names_are_found_and_replaced_one_by_one() {
    let (mut total, mut repeated) = (0, 0);
    for seed in 0..32u64 {
        let mut g = Lcg(seed);
        let headers: Vec<String> = (0..1 + g.below(4)).map(|_| g.name()).collect();
        let fields: Vec<String> = (0..1 + g.below(4)).map(|_| g.name()).collect();
        let mut rest = String::from("  headers:\n");
        for h in &headers {
            rest.push_str(&format!("    - name: \"{h}\"\n      value: v\n"));
        }
        rest.push_str("  body:\n    type: form-urlencoded\n    data:\n");
        for f in &fields {
            rest.push_str(&format!("      - name: \"{f}\"\n        value: v\n"));
        }
        let text = request("r", "https://x.test/", &rest);
        let has = |n: &&String| n.to_lowercase().contains("zq");
        let expected: Vec<(SearchField, String)> = headers
            .iter()
            .filter(has)
            .map(|h| (SearchField::HeaderName, h.clone()))
            .chain(
                fields
                    .iter()
                    .filter(has)
                    .map(|f| (SearchField::Body, f.clone())),
            )
            .collect();

        let (_d, c) = open(&[("a.yml", &text)]);
        let hits = c.search("zq").hits;
        let found: Vec<(SearchField, String)> = hits
            .iter()
            .map(|h| (h.field, h.key.clone().unwrap_or_default()))
            .collect();
        assert_eq!(found, expected, "seed {seed}");
        total += expected.len();
        repeated += (1..expected.len())
            .filter(|&i| expected[..i].contains(&expected[i]))
            .count();

        // Each hit, replaced alone in a fresh copy, changes exactly its own name.
        for (j, hit) in hits.into_iter().enumerate() {
            let path = hit.path.clone();
            let (d, mut c) = open(&[("a.yml", &text)]);
            let r = c
                .replace("zq", "R", &lit(), ReplaceScope::hit(hit))
                .unwrap();
            assert_eq!(r.replaced, 1, "seed {seed} hit {j}");
            let (mut want_h, mut want_f) = (headers.clone(), fields.clone());
            let nth = |names: &[String], k: usize| {
                names
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.to_lowercase().contains("zq"))
                    .nth(k)
                    .map(|(i, _)| i)
            };
            let in_headers = headers.iter().filter(has).count();
            if j < in_headers {
                let i = nth(&headers, j).unwrap();
                want_h[i] = swap_first(&headers[i]);
            } else {
                let i = nth(&fields, j - in_headers).unwrap();
                want_f[i] = swap_first(&fields[i]);
            }
            let saved = Collection::open(d.path()).expect("reopens");
            assert_eq!(
                names_of(&saved, &path),
                (want_h, want_f),
                "seed {seed} hit {j}"
            );
        }
    }
    // The seeds must exercise many hits, some on repeated names.
    assert!(
        total >= 20 && repeated >= 2,
        "{total} hits, {repeated} repeats"
    );
}
