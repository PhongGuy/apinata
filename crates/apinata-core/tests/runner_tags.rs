//! Effective tags on a run plan, and the include and exclude filter over them.

use crate::common;

use apinata_core::{Collection, RunPlan, TagFilter};
use common::write_tree;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn req(name: &str, seq: u32, tags: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\n{tags}http:\n  method: GET\n  url: \"http://127.0.0.1:1/{name}\"\n"
    )
}

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "api/folder.yml",
            "info:\n  name: api\n  type: folder\n  tags:\n    - \" api \"\n    - \"\"\n    - api\n    - shared\n",
        ),
        (
            "api/inner/folder.yml",
            "info:\n  name: inner\n  type: folder\n  tags:\n    - inner\n    - shared\n",
        ),
        (
            "api/inner/deep.yml",
            &req(
                "deep",
                1,
                "  tags:\n    - smoke\n    - \" smoke\"\n    - 7\n",
            ),
        ),
        ("api/plain.yml", &req("plain", 1, "")),
        ("top.yml", &req("top", 1, "  tags:\n    - smoke\n")),
        ("bad.yml", "http: [broken\n"),
    ])
}

fn shape(plan: &RunPlan) -> Vec<(String, Vec<String>)> {
    plan.items()
        .zip(plan.tags())
        .map(|(i, t)| (i.as_str().to_owned(), t.to_vec()))
        .collect()
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|t| (*t).to_owned()).collect()
}

#[test]
fn effective_tags_are_own_tags_then_folder_tags_outer_to_inner_normalised() {
    let dir = tree();
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    assert_eq!(
        shape(&plan),
        [
            (
                "api/inner/deep.yml".to_owned(),
                s(&["smoke", "7", "api", "shared", "inner"])
            ),
            ("api/plain.yml".to_owned(), s(&["api", "shared"])),
            ("top.yml".to_owned(), s(&["smoke"])),
        ]
    );
    let single = c
        .plan(&dir.path().join("api/inner/deep.yml"))
        .expect("plan");
    assert_eq!(
        shape(&single)[0].1,
        s(&["smoke", "7", "api", "shared", "inner"])
    );
}

#[test]
fn the_filter_keeps_any_included_tag_drops_any_excluded_one_and_keeps_load_errors() {
    let dir = tree();
    let c = Collection::open(dir.path()).expect("opens");
    let run = |include: &[&str], exclude: &[&str]| {
        let plan = c.plan(dir.path()).expect("plan");
        assert_eq!(plan.load_errors().len(), 1);
        let kept = plan.filter_tags(&TagFilter::new(s(include), s(exclude)));
        assert_eq!(
            kept.load_errors().len(),
            1,
            "load errors survive the filter"
        );
        assert_eq!(kept.requests().count(), kept.tags().count());
        shape(&kept).into_iter().map(|(p, _)| p).collect::<Vec<_>>()
    };
    assert_eq!(
        run(&[], &[]),
        ["api/inner/deep.yml", "api/plain.yml", "top.yml"]
    );
    assert_eq!(run(&["smoke"], &[]), ["api/inner/deep.yml", "top.yml"]);
    assert_eq!(run(&["api"], &["inner"]), ["api/plain.yml"]);
    assert_eq!(run(&[], &["shared"]), ["top.yml"]);
    assert!(
        run(&["Smoke"], &[]).is_empty(),
        "matching is case-sensitive"
    );
}

#[test]
fn parse_list_trims_and_drops_empty_items_and_matches_is_any_of() {
    assert_eq!(TagFilter::parse_list(" a, b,,c ,"), s(&["a", "b", "c"]));
    assert!(TagFilter::parse_list("  ").is_empty());
    assert!(TagFilter::default().is_empty());
    let f = TagFilter::new(s(&["a", "b"]), s(&["x"]));
    assert!(!f.is_empty());
    assert!(f.matches(&s(&["b"])));
    assert!(!f.matches(&s(&["b", "x"])));
    assert!(!f.matches(&[]), "an include list drops untagged requests");
    assert!(TagFilter::default().matches(&[]));
}
