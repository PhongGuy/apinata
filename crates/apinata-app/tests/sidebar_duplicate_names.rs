//! A request name shared by two files shows the file name, so the rows can be told apart.

use crate::support;

use apinata_app::sidebar::model::{Expanded, Node, build};
use apinata_app::snapshot::SnapshotCache;
use apinata_core::Collection;
use support::{ROOT, request, write_tree};

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
    )
}

fn file_of(nodes: &[Node], path: &str) -> Option<String> {
    nodes[0]
        .children
        .iter()
        .find(|n| n.path.as_str() == path)
        .unwrap_or_else(|| panic!("{path}"))
        .file
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
