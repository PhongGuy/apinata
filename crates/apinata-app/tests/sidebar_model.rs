use crate::support;

use apinata_app::sidebar::model::{
    EMPTY_LABEL, Expanded, Node, NodeKind, OpenDraft, UNSAVED_LABEL, build, empty_id, invalid_id,
    node_id, to_tree_items, unsaved_id,
};
use apinata_app::snapshot::{Content, DiskState, SnapshotCache};
use apinata_core::{Collection, ItemPath};
use std::collections::HashSet;
use std::path::Path;
use support::{ROOT, folder, ip, request, write_tree};

const SOCKET: &str = "info:\n  name: Socket\n  type: websocket\n  seq: 2\n";

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List users", "http://x/users")),
        ("users/admin/folder.yml", &folder("admin")),
        (
            "users/admin/create.yml",
            &request("Create admin", "http://x/admin"),
        ),
        ("health.yml", &request("Health", "http://x/health")),
        ("ws.yml", SOCKET),
        ("broken.yml", "info: [\n"),
    ])
}

fn content(dir: &tempfile::TempDir) -> Content {
    SnapshotCache::new().build(&Collection::open(dir.path()).unwrap())
}

fn expanded(root: &Path, paths: &[&str]) -> Expanded {
    let set: HashSet<ItemPath> = paths.iter().map(|p| ip(p)).collect();
    Expanded::from([(root.to_path_buf(), set)])
}

fn flat(nodes: &[Node]) -> Vec<(String, NodeKind, bool)> {
    fn walk(n: &Node, out: &mut Vec<(String, NodeKind, bool)>) {
        out.push((n.id.to_string(), n.kind, n.expanded));
        n.children.iter().for_each(|c| walk(c, out));
    }
    let mut out = Vec::new();
    nodes.iter().for_each(|n| walk(n, &mut out));
    out
}

fn find<'a>(nodes: &'a [Node], id: &str) -> &'a Node {
    fn go<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Node> {
        nodes.iter().find_map(|n| {
            if n.id.as_ref() == id {
                Some(n)
            } else {
                go(&n.children, id)
            }
        })
    }
    go(nodes, id).unwrap_or_else(|| panic!("{id}"))
}

fn s(id: &str, kind: NodeKind, expanded: bool) -> (String, NodeKind, bool) {
    (id.to_string(), kind, expanded)
}

#[test]
fn nodes_nest_the_rows_under_their_collection() {
    let dir = tree();
    let c = content(&dir);
    let nodes = build(
        &[(dir.path(), "c", Some(&c))],
        &[],
        &expanded(dir.path(), &["", "users"]),
        "",
    );
    assert_eq!(
        flat(&nodes),
        [
            s("0:", NodeKind::Collection, true),
            s("0:users", NodeKind::Folder, true),
            s("0:users/admin", NodeKind::Folder, false),
            s("0:users/admin/create.yml", NodeKind::Request, false),
            s("0:users/list.yml", NodeKind::Request, false),
            s("0:health.yml", NodeKind::Request, false),
            s("0:ws.yml", NodeKind::Opaque, false),
            s("0::0", NodeKind::Invalid, false),
        ]
    );
    assert_eq!(nodes[0].name, "c");
    assert_eq!(nodes[0].root, dir.path());
    assert_eq!(find(&nodes, "0:users").name, "Users");
    assert_eq!(find(&nodes, "0:health.yml").method.as_deref(), Some("GET"));
    assert_eq!(find(&nodes, "0:health.yml").path, ip("health.yml"));
    assert_eq!(find(&nodes, "0:ws.yml").method, None);
    assert!(find(&nodes, "0::0").name.starts_with("broken.yml"));
}

#[test]
fn an_opaque_item_yields_no_invalid_row() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("ws.yml", SOCKET),
        ("b.yml", "info: [\n"),
        ("a/x.yml", "info: [\n"),
    ]);
    let c = content(&dir);
    // Core reports the opaque file as a load error too; the sidebar must not.
    assert!(c.load_errors.iter().any(|e| e.path == Path::new("ws.yml")));
    let nodes = build(
        &[(dir.path(), "c", Some(&c))],
        &[],
        &expanded(dir.path(), &[""]),
        "",
    );
    let invalid: Vec<&Node> = nodes[0]
        .children
        .iter()
        .filter(|n| n.kind == NodeKind::Invalid)
        .collect();
    assert_eq!(invalid.len(), 2);
    assert_eq!(invalid[0].id, invalid_id(0, 0));
    assert!(invalid[0].name.contains("x.yml"));
    assert!(invalid[1].name.starts_with("b.yml"));
    assert!(!invalid.iter().any(|n| n.name.contains("ws.yml")));
}

#[test]
fn rows_carry_dirty_and_disk_markers() {
    let dir = tree();
    let mut c = content(&dir);
    let mut rows = c.rows.to_vec();
    rows.iter_mut()
        .find(|r| r.path == ip("health.yml"))
        .unwrap()
        .dirty = true;
    rows.iter_mut()
        .find(|r| r.path == ip("users/list.yml"))
        .unwrap()
        .disk = DiskState::Removed;
    c.rows = rows.into();
    let nodes = build(
        &[(dir.path(), "c", Some(&c))],
        &[],
        &expanded(dir.path(), &[""]),
        "",
    );
    assert!(find(&nodes, "0:health.yml").dirty);
    assert_eq!(find(&nodes, "0:health.yml").disk, DiskState::Present);
    assert_eq!(find(&nodes, "0:users/list.yml").disk, DiskState::Removed);
    assert!(!find(&nodes, "0:users/list.yml").dirty);
}

#[test]
fn the_filter_keeps_matching_requests_and_expands_their_folders() {
    let dir = tree();
    let c = content(&dir);
    let input = [(dir.path(), "c", Some(&c))];
    let none = expanded(dir.path(), &[]);
    assert_eq!(
        flat(&build(&input, &[], &none, "ADMIN")),
        [
            s("0:", NodeKind::Collection, true),
            s("0:users", NodeKind::Folder, true),
            s("0:users/admin", NodeKind::Folder, true),
            s("0:users/admin/create.yml", NodeKind::Request, false),
        ]
    );
    // A folder's own name never matches; "Users" finds only the request named "List users".
    assert_eq!(
        flat(&build(&input, &[], &none, "Users")),
        [
            s("0:", NodeKind::Collection, true),
            s("0:users", NodeKind::Folder, true),
            s("0:users/list.yml", NodeKind::Request, false),
        ]
    );
    assert_eq!(
        flat(&build(&input, &[], &none, "sock")),
        [
            s("0:", NodeKind::Collection, true),
            s("0:ws.yml", NodeKind::Opaque, false)
        ]
    );
    assert_eq!(flat(&build(&input, &[], &none, "zzz")), Vec::new());
    assert_eq!(
        build(&input, &[], &none, "  "),
        build(&input, &[], &none, "")
    );
}

#[test]
fn a_second_collection_and_an_opening_one_get_their_own_ids() {
    let dir = tree();
    let c = content(&dir);
    let other = tempfile::tempdir().unwrap();
    let mut exp = expanded(dir.path(), &[""]);
    exp.insert(
        other.path().to_path_buf(),
        HashSet::from([ItemPath::root()]),
    );
    let nodes = build(
        &[(dir.path(), "c", Some(&c)), (other.path(), "other", None)],
        &[],
        &exp,
        "",
    );
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[1].id.as_ref(), "1:");
    assert_eq!(
        (
            nodes[1].collection,
            nodes[1].name.as_str(),
            nodes[1].expanded
        ),
        (1, "other", true)
    );
    assert!(nodes[1].children.is_empty());
    assert_eq!(
        node_id(1, &ip("users/list.yml")).as_ref(),
        "1:users/list.yml"
    );
    assert_eq!(invalid_id(1, 0).as_ref(), "1::0");
}

#[test]
fn tree_items_mirror_the_nodes() {
    let dir = tree();
    let c = content(&dir);
    let nodes = build(
        &[(dir.path(), "c", Some(&c))],
        &[],
        &expanded(dir.path(), &[""]),
        "",
    );
    let items = to_tree_items(&nodes);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id.as_ref(), "0:");
    assert!(items[0].is_expanded());
    assert_eq!(items[0].children[0].label.as_ref(), "Users");
    assert!(!items[0].children[0].is_expanded());
    assert_eq!(items[0].children[0].children.len(), 2);
    assert!(items[0].children.last().unwrap().is_disabled());
}

#[test]
fn a_filter_drops_only_the_collections_without_a_match() {
    let dir = tree();
    let c = content(&dir);
    let other = tree();
    let o = content(&other);
    let input = [(dir.path(), "c", Some(&c)), (other.path(), "d", Some(&o))];
    let none = Expanded::new();
    let ids: Vec<String> = build(&input, &[], &none, "ADMIN")
        .iter()
        .map(|n| n.id.to_string())
        .collect();
    assert_eq!(ids, ["0:", "1:"], "both match");
    let only_first = [(dir.path(), "c", Some(&c)), (other.path(), "d", None)];
    let ids: Vec<String> = build(&only_first, &[], &none, "ADMIN")
        .iter()
        .map(|n| n.id.to_string())
        .collect();
    assert_eq!(ids, ["0:"], "a collection with nothing to show is left out");
}

#[test]
fn a_shortened_method_tag_keeps_the_colour_of_its_full_method() {
    use apinata_app::ui::StatusClass;
    let with = |m: &str| request("R", "http://x/r").replace("method: GET", &format!("method: {m}"));
    let (patch, delete, query) = (with("PATCH"), with("DELETE"), with("QUERY"));
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("p.yml", &patch),
        ("d.yml", &delete),
        ("q.yml", &query),
    ]);
    let c = content(&dir);
    let nodes = build(
        &[(dir.path(), "c", Some(&c))],
        &[],
        &expanded(dir.path(), &[""]),
        "",
    );
    for (id, label, class) in [
        ("0:p.yml", "PTCH", StatusClass::Info),
        ("0:d.yml", "DEL", StatusClass::Danger),
        ("0:q.yml", "QURY", StatusClass::Muted),
    ] {
        let tag = find(&nodes, id).method_tag().expect(id);
        assert_eq!((tag.label().as_ref(), tag.class()), (label, class), "{id}");
    }
    assert!(find(&nodes, "0:").method_tag().is_none());
}

fn draft(root: &Path, path: &str, name: &str) -> OpenDraft {
    OpenDraft {
        root: root.to_path_buf(),
        path: ip(path),
        name: name.into(),
        method: "GET".into(),
    }
}

fn open_root(root: &Path) -> Expanded {
    Expanded::from([(root.to_path_buf(), HashSet::from([ItemPath::root()]))])
}

#[test]
fn open_drafts_head_their_collection_under_unsaved() {
    let dir = tree();
    let c = content(&dir);
    let input = [(dir.path(), "c", Some(&c))];
    let drafts = [
        draft(dir.path(), "untitled-1.draft", "Untitled request 1"),
        draft(dir.path(), "users/untitled-2.draft", "Untitled request 2"),
    ];
    let nodes = build(&input, &drafts, &open_root(dir.path()), "");
    let group = &nodes[0].children[0];
    assert_eq!(UNSAVED_LABEL, "Unsaved");
    assert_eq!(
        (
            group.id.clone(),
            group.kind,
            group.name.as_str(),
            group.expanded
        ),
        (unsaved_id(0), NodeKind::UnsavedGroup, UNSAVED_LABEL, true)
    );
    let rows: Vec<(String, NodeKind, String)> = group
        .children
        .iter()
        .map(|n| (n.id.to_string(), n.kind, n.name.clone()))
        .collect();
    assert_eq!(
        rows,
        [
            (
                "0:untitled-1.draft".to_owned(),
                NodeKind::Draft,
                "Untitled request 1".to_owned()
            ),
            (
                "0:users/untitled-2.draft".to_owned(),
                NodeKind::Draft,
                "Untitled request 2".to_owned()
            ),
        ]
    );
    assert_eq!(group.children[0].method.as_deref(), Some("GET"));
}

#[test]
fn without_drafts_there_is_no_unsaved_group() {
    let dir = tree();
    let c = content(&dir);
    let input = [(dir.path(), "c", Some(&c))];
    let nodes = build(&input, &[], &open_root(dir.path()), "");
    assert!(
        nodes[0]
            .children
            .iter()
            .all(|n| n.kind != NodeKind::UnsavedGroup)
    );
}

#[test]
fn the_filter_matches_draft_names() {
    let dir = tree();
    let c = content(&dir);
    let input = [(dir.path(), "c", Some(&c))];
    let none = expanded(dir.path(), &[]);
    let drafts = [draft(dir.path(), "untitled-1.draft", "Untitled request 1")];
    let ids: Vec<String> = flat(&build(&input, &drafts, &none, "untitled"))
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(
        ids,
        [
            "0:".to_owned(),
            unsaved_id(0).to_string(),
            "0:untitled-1.draft".to_owned()
        ]
    );
    assert!(flat(&build(&input, &drafts, &none, "zzz")).is_empty());
}

#[test]
fn draft_rows_are_neither_dragged_nor_dropped_on() {
    use apinata_app::sidebar::drop::{DropZone, drop_target};
    for kind in [NodeKind::Draft, NodeKind::UnsavedGroup] {
        for zone in [DropZone::Before, DropZone::Inside, DropZone::After] {
            assert_eq!(
                drop_target(&ip("a.yml"), &ip("untitled-1.draft"), kind, zone),
                None,
                "{kind:?}"
            );
        }
    }
}

#[test]
fn an_empty_folder_holds_one_disabled_empty_row() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("empty/folder.yml", &folder("Empty folder")),
        ("full/folder.yml", &folder("Full")),
        ("full/a.yml", &request("a", "http://x/a")),
    ]);
    let c = content(&dir);
    let input = [(dir.path(), "c", Some(&c))];
    let nodes = build(&input, &[], &expanded(dir.path(), &["", "empty"]), "");
    assert_eq!(
        flat(&nodes),
        [
            s("0:", NodeKind::Collection, true),
            s("0:empty", NodeKind::Folder, true),
            s(empty_id(0, &ip("empty")).as_ref(), NodeKind::Empty, false),
            s("0:full", NodeKind::Folder, false),
            s("0:full/a.yml", NodeKind::Request, false),
        ]
    );
    let row = find(&nodes, empty_id(0, &ip("empty")).as_ref());
    assert_eq!(row.name, EMPTY_LABEL);
    let items = to_tree_items(&nodes);
    assert!(items[0].children[0].children[0].is_disabled());
    // A filter never keeps an empty folder, so it never shows the row either.
    assert_eq!(
        flat(&build(&input, &[], &expanded(dir.path(), &[]), "a")).len(),
        3
    );
}
