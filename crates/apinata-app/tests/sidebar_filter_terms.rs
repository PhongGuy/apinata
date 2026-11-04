//! The sidebar filter's `tag:` and `method:` terms, and their agreement with a run's tag filter.

use crate::support;

use apinata_app::sidebar::model::{Node, NodeKind, build};
use apinata_app::sidebar::terms::FilterTerms;
use apinata_app::snapshot::{Content, SnapshotCache};
use apinata_core::{Collection, TagFilter};
use std::collections::{BTreeSet, HashMap};
use support::{ROOT, write_tree};

fn item(name: &str, method: &str, tags: &[&str]) -> String {
    let tags = if tags.is_empty() {
        String::new()
    } else {
        let list: String = tags.iter().map(|t| format!("    - {t}\n")).collect();
        format!("  tags:\n{list}")
    };
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\n{tags}http:\n  method: {method}\n  url: \"http://x/{name}\"\n"
    )
}

fn folder_tagged(name: &str, tags: &[&str]) -> String {
    let list: String = tags.iter().map(|t| format!("    - {t}\n")).collect();
    format!("info:\n  name: {name}\n  type: folder\n  seq: 1\n  tags:\n{list}")
}

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("smoke/folder.yml", &folder_tagged("Smoke", &["x"])),
        ("smoke/inner/folder.yml", &folder_tagged("Inner", &["deep"])),
        ("smoke/inner/deep.yml", &item("Deep", "PUT", &[])),
        ("smoke/plain.yml", &item("Plain", "GET", &[])),
        ("own.yml", &item("Own", "POST", &["x", "Y"])),
        ("other.yml", &item("Other", "POST", &["y"])),
        ("none.yml", &item("None", "GET", &[])),
    ])
}

fn content(dir: &tempfile::TempDir) -> Content {
    SnapshotCache::new().build(&Collection::open(dir.path()).unwrap())
}

fn leaves(nodes: &[Node], out: &mut BTreeSet<String>) {
    for n in nodes {
        if n.kind == NodeKind::Request {
            out.insert(n.path.as_str().to_owned());
        }
        leaves(&n.children, out);
    }
}

fn kept(dir: &tempfile::TempDir, filter: &str) -> Vec<String> {
    let c = content(dir);
    let nodes = build(&[(dir.path(), "c", Some(&c))], &[], &HashMap::new(), filter);
    let mut out = BTreeSet::new();
    leaves(&nodes, &mut out);
    out.into_iter().collect()
}

#[test]
fn tag_term_keeps_the_requests_a_run_with_that_tag_would_run() {
    let dir = tree();
    let col = Collection::open(dir.path()).unwrap();
    let plan = col
        .plan(dir.path())
        .unwrap()
        .filter_tags(&TagFilter::new(vec!["x".into()], vec![]));
    let planned: BTreeSet<String> = plan.items().map(|p| p.as_str().to_owned()).collect();
    assert_eq!(
        planned,
        ["own.yml", "smoke/inner/deep.yml", "smoke/plain.yml"]
            .map(String::from)
            .into()
    );
    let shown: BTreeSet<String> = kept(&dir, "tag:x").into_iter().collect();
    assert_eq!(shown, planned);
}

#[test]
fn tag_matching_is_case_sensitive() {
    let dir = tree();
    assert_eq!(kept(&dir, "tag:y"), ["other.yml"]);
    assert_eq!(kept(&dir, "tag:Y"), ["own.yml"]);
    assert_eq!(kept(&dir, "tag:X"), Vec::<String>::new());
}

#[test]
fn method_term_ignores_case() {
    let dir = tree();
    assert_eq!(kept(&dir, "method:post"), ["other.yml", "own.yml"]);
    assert_eq!(kept(&dir, "METHOD:Put"), ["smoke/inner/deep.yml"]);
}

#[test]
fn tag_and_method_combine_with_and() {
    let dir = tree();
    assert_eq!(kept(&dir, "tag:x method:post"), ["own.yml"]);
    assert_eq!(kept(&dir, "tag:x method:put"), ["smoke/inner/deep.yml"]);
    assert_eq!(kept(&dir, "tag:x method:delete"), Vec::<String>::new());
}

#[test]
fn several_tag_terms_must_all_be_present() {
    let dir = tree();
    assert_eq!(kept(&dir, "tag:x tag:y"), Vec::<String>::new());
    assert_eq!(kept(&dir, "tag:x tag:Y"), ["own.yml"]);
    assert_eq!(kept(&dir, "tag:x tag:deep"), ["smoke/inner/deep.yml"]);
}

#[test]
fn a_plain_word_combines_with_a_tag() {
    let dir = tree();
    assert_eq!(kept(&dir, "tag:x deep"), ["smoke/inner/deep.yml"]);
    assert_eq!(kept(&dir, "pla tag:x"), ["smoke/plain.yml"]);
    assert_eq!(kept(&dir, "own tag:deep"), Vec::<String>::new());
}

#[test]
fn an_empty_value_is_no_constraint() {
    let dir = tree();
    assert_eq!(kept(&dir, "tag:").len(), 5);
    assert_eq!(kept(&dir, "method:").len(), 5);
    assert_eq!(kept(&dir, "tag: method:post"), ["other.yml", "own.yml"]);
    assert_eq!(kept(&dir, "oth tag:"), ["other.yml"]);
}

#[test]
fn text_without_a_prefix_is_the_name_substring_as_before() {
    let dir = tree();
    assert_eq!(kept(&dir, "OWN"), ["own.yml"]);
    assert_eq!(kept(&dir, "tagged"), Vec::<String>::new());
    assert_eq!(kept(&dir, "  "), kept(&dir, ""));
    assert!(FilterTerms::parse("two words").is_some());
    assert!(FilterTerms::parse("   ").is_none());
    assert!(FilterTerms::parse("tag: method:").is_none());
}

#[test]
fn terms_match_on_name_method_and_effective_tags() {
    let t = FilterTerms::parse("ow tag:x method:post").unwrap();
    let tags = vec!["x".to_owned()];
    assert!(t.matches("Own", Some("POST"), Some(&tags)));
    assert!(!t.matches("Own", Some("GET"), Some(&tags)));
    assert!(!t.matches("Own", Some("POST"), Some(&[])));
    // A leaf with no tag data cannot satisfy a tag term.
    assert!(!t.matches("Own", Some("POST"), None));
    let plain = FilterTerms::parse("ow").unwrap();
    assert!(plain.matches("Own", None, None));
}

#[test]
fn a_folder_name_alone_does_not_match_a_term_free_of_it() {
    let dir = tree();
    // A plain word equal to a folder tag matches names only, so nothing is kept.
    assert_eq!(kept(&dir, "smoke"), Vec::<String>::new());
}

#[gpui_kit::test]
fn the_field_shows_the_placeholder_and_its_tooltip_names_the_prefixes(
    cx: &mut gpui_kit::TestAppContext,
) {
    let h = support::open_sidebar(cx);
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    let text = filter.read_with(cx, |i, _| i.presentation().placeholder().to_string());
    assert_eq!(text, apinata_app::sidebar::FILTER_PLACEHOLDER);
    let tip = apinata_app::sidebar::FILTER_TOOLTIP;
    assert!(tip.contains("tag:") && tip.contains("method:"), "{tip}");
}
