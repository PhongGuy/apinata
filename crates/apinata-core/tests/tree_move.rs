//! Moving items: Bruno's drop placement, renumbering, collisions and invalid moves.

use crate::tree_support;

use apinata_core::{DropTarget, ItemPath, TreeError};
use tempfile::TempDir;
use tree_support::{
    BROKEN, ROOT, changed_url, disk_seq, exists, folder, ip, item, open, read, reload, request,
    write,
};

fn seqs(dir: &TempDir, rels: &[&str]) -> Vec<Option<u64>> {
    rels.iter().map(|r| disk_seq(dir, r)).collect()
}

#[test]
fn before_and_after_reorder_within_a_directory() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
    ]);
    let moved = c
        .move_item(&ip("c.yml"), DropTarget::Before(ip("a.yml")))
        .expect("moves");
    assert_eq!(moved, ip("c.yml"));
    assert_eq!(
        seqs(&dir, &["a.yml", "b.yml", "c.yml"]),
        [Some(2), Some(3), Some(1)]
    );
    c.move_item(&ip("c.yml"), DropTarget::After(ip("b.yml")))
        .expect("moves");
    assert_eq!(
        seqs(&dir, &["a.yml", "b.yml", "c.yml"]),
        [Some(1), Some(2), Some(3)]
    );
    assert_eq!(c.request(&ip("c.yml")).expect("c").seq(), Some(3));
}

#[test]
fn apps_are_renumbered_by_a_move_but_unparseable_and_unknown_items_are_not() {
    let custom = item("custom", "x", 1);
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("ui.yml", &item("app", "ui", 3)),
        ("x.yml", &custom),
        ("bad.yml", BROKEN),
        ("g/folder.yml", BROKEN),
    ]);
    c.move_item(&ip("ui.yml"), DropTarget::Before(ip("a.yml")))
        .expect("moves");
    assert_eq!(
        seqs(&dir, &["ui.yml", "a.yml", "b.yml"]),
        [Some(1), Some(2), Some(3)]
    );
    assert_eq!(
        (read(&dir, "x.yml"), read(&dir, "bad.yml")),
        (custom, BROKEN.to_owned())
    );
    assert_eq!(read(&dir, "g/folder.yml"), BROKEN);
}

#[test]
fn inside_takes_the_next_seq_and_renumbers_the_source() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
        ("f/folder.yml", &folder("f", 4)),
        ("f/x.yml", &request("x", 1)),
    ]);
    let moved = c
        .move_item(&ip("b.yml"), DropTarget::Inside(ip("f")))
        .expect("moves");
    assert_eq!(moved, ip("f/b.yml"));
    assert!(!exists(&dir, "b.yml"));
    let rels = ["a.yml", "c.yml", "f", "f/x.yml", "f/b.yml"];
    assert_eq!(
        seqs(&dir, &rels),
        [Some(1), Some(2), Some(3), Some(1), Some(2)]
    );
    assert!(c.request(&moved).is_some() && c.request(&ip("b.yml")).is_none());
    assert!(reload(&dir, &mut c, "f/b.yml").is_empty() && reload(&dir, &mut c, "b.yml").is_empty());
}

#[test]
fn after_a_target_in_another_directory_renumbers_both() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/x.yml", &request("x", 1)),
        ("f/y.yml", &request("y", 2)),
    ]);
    let moved = c
        .move_item(&ip("a.yml"), DropTarget::After(ip("f/x.yml")))
        .expect("moves");
    assert_eq!(moved, ip("f/a.yml"));
    assert_eq!(
        seqs(&dir, &["f", "f/x.yml", "f/a.yml", "f/y.yml"]),
        [Some(1), Some(1), Some(2), Some(3)]
    );
}

#[test]
fn a_dragged_item_gets_its_new_seq_even_when_it_matches_the_targets_old_one() {
    // Bruno skips this write (a known divergence, see Decisions); Apinata writes seq 2.
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 7)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/x.yml", &request("x", 1)),
        ("f/y.yml", &request("y", 2)),
    ]);
    c.move_item(&ip("a.yml"), DropTarget::Before(ip("f/y.yml")))
        .expect("moves");
    assert_eq!(
        seqs(&dir, &["f/x.yml", "f/a.yml", "f/y.yml"]),
        [Some(1), Some(2), Some(3)]
    );
    assert_eq!(c.request(&ip("f/a.yml")).expect("a").seq(), Some(2));
}

#[test]
fn a_taken_name_in_the_target_gets_a_suffix() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/a.yml", &request("a", 1)),
    ]);
    let moved = c
        .move_item(&ip("a.yml"), DropTarget::Inside(ip("f")))
        .expect("moves");
    assert_eq!(moved, ip("f/a 1.yml"));
    assert!(exists(&dir, "f/a.yml") && exists(&dir, "f/a 1.yml"));
}

#[test]
fn a_moved_folder_carries_its_tree() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("f/x.yml", &request("x", 1)),
        ("g/folder.yml", &folder("g", 2)),
    ]);
    let moved = c
        .move_item(&ip("f"), DropTarget::Inside(ip("g")))
        .expect("moves");
    assert_eq!(moved, ip("g/f"));
    let x = c
        .request_mut(&ip("g/f/x.yml"))
        .expect("x moved with its folder");
    x.set_method("POST");
    x.save().expect("saves at the new path");
    assert!(read(&dir, "g/f/x.yml").contains("method: POST"));
    assert!(reload(&dir, &mut c, "g/f").is_empty() && reload(&dir, &mut c, "f").is_empty());
}

#[test]
fn invalid_moves_are_rejected_and_a_drop_on_the_own_parent_is_a_no_op() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/g/folder.yml", &folder("g", 1)),
    ]);
    let invalid = [
        (ItemPath::root(), DropTarget::Inside(ip("f"))),
        (ip("f"), DropTarget::Inside(ip("f"))),
        (ip("f"), DropTarget::Inside(ip("f/g"))),
        (ip("a.yml"), DropTarget::Before(ItemPath::root())),
        (ip("a.yml"), DropTarget::Inside(ip("a.yml"))),
        (ip("a.yml"), DropTarget::Before(ip("a.yml"))),
    ];
    for (item, to) in invalid {
        let err = c.move_item(&item, to.clone()).unwrap_err();
        assert!(
            matches!(err, TreeError::InvalidMove { .. }),
            "{item} {to:?}: {err:?}"
        );
    }
    let missing = c.move_item(&ip("a.yml"), DropTarget::Inside(ip("nope")));
    assert!(
        matches!(missing, Err(TreeError::NotFound { .. })),
        "{missing:?}"
    );
    let before = read(&dir, "a.yml");
    assert_eq!(
        c.move_item(&ip("a.yml"), DropTarget::Inside(ItemPath::root()))
            .expect("no-op"),
        ip("a.yml")
    );
    assert_eq!(read(&dir, "a.yml"), before);
}

#[test]
fn a_dragged_file_changed_on_disk_is_a_conflict_before_anything_moves() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 7)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/x.yml", &request("x", 1)),
        ("f/y.yml", &request("y", 2)),
    ]);
    write(&dir, "a.yml", &changed_url("a", 7));
    let err = c
        .move_item(&ip("a.yml"), DropTarget::Before(ip("f/y.yml")))
        .unwrap_err();
    assert!(matches!(err, TreeError::Conflict { .. }), "{err:?}");
    assert!(exists(&dir, "a.yml") && !exists(&dir, "f/a.yml"));
    assert_eq!(read(&dir, "a.yml"), changed_url("a", 7));
}
