//! Rows the tree cannot tell apart get a muted hint: sibling requests sharing a name show their
//! file names, and collections sharing a name the part of their folder that differs.

use crate::support;

use apinata_app::sidebar::model::{Expanded, Node, build};
use apinata_app::snapshot::SnapshotCache;
use apinata_core::Collection;
use support::{ROOT, folder, request, write_tree};

fn nodes(files: &[(&str, &str)]) -> Vec<Node> {
    filtered(files, "")
}

fn filtered(files: &[(&str, &str)], filter: &str) -> Vec<Node> {
    let dir = write_tree(files);
    let content = SnapshotCache::new().build(&Collection::open(dir.path()).unwrap());
    build(
        &[(dir.path(), "C", Some(&content))],
        &[],
        &Expanded::new(),
        filter,
        None,
    )
}

fn file_of(nodes: &[Node], path: &str) -> Option<String> {
    nodes[0]
        .children
        .iter()
        .find(|n| n.path.as_str() == path)
        .unwrap_or_else(|| panic!("{path}"))
        .hint
        .clone()
}

#[test]
fn duplicate_names_carry_their_file_names() {
    let n = nodes(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &request("echo post", "http://x/1")),
        ("two.yml", &request("echo post", "http://x/2")),
    ]);
    assert_eq!(file_of(&n, "one.yml").as_deref(), Some("one.yml"));
    assert_eq!(file_of(&n, "two.yml").as_deref(), Some("two.yml"));
}

#[test]
fn unique_names_carry_no_file_name() {
    let n = nodes(&[
        ("opencollection.yml", ROOT),
        ("one.yml", &request("alpha", "http://x/1")),
        ("two.yml", &request("beta", "http://x/2")),
    ]);
    assert_eq!(file_of(&n, "one.yml"), None);
    assert_eq!(file_of(&n, "two.yml"), None);
}

#[test]
fn a_filter_hiding_one_twin_keeps_the_other_file_name() {
    let tagged =
        request("echo post", "http://x/1").replace("http:\n", "  tags:\n    - keep\nhttp:\n");
    let n = filtered(
        &[
            ("opencollection.yml", ROOT),
            ("one.yml", &tagged),
            ("two.yml", &request("echo post", "http://x/2")),
        ],
        "tag:keep",
    );
    assert_eq!(n[0].children.len(), 1, "the filter hides the other twin");
    assert_eq!(file_of(&n, "one.yml").as_deref(), Some("one.yml"));
}

#[test]
fn same_named_requests_in_different_folders_need_no_hint() {
    let n = nodes(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/get.yml", &request("Get user", "http://x/1")),
        ("graphql/folder.yml", &folder("GraphQL")),
        ("graphql/get.yml", &request("Get user", "http://x/2")),
    ]);
    let hints: Vec<_> = (n[0].children.iter())
        .flat_map(|f| f.children.iter())
        .map(|r| r.hint.clone())
        .collect();
    assert_eq!(hints, [None, None], "the folders tell them apart");
}

#[test]
fn same_named_collections_show_where_their_folders_differ() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = (tmp.path().join("work/shop"), tmp.path().join("play/shop"));
    let mut contents = Vec::new();
    for dir in [&a, &b] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("opencollection.yml"), ROOT).unwrap();
        contents.push(SnapshotCache::new().build(&Collection::open(dir).unwrap()));
    }
    let n = build(
        &[
            (a.as_path(), "Shop", Some(&contents[0])),
            (b.as_path(), "Shop", Some(&contents[1])),
        ],
        &[],
        &Expanded::new(),
        "",
        None,
    );
    assert_eq!(n[0].hint.as_deref(), Some("work/shop"));
    assert_eq!(n[1].hint.as_deref(), Some("play/shop"));
}
