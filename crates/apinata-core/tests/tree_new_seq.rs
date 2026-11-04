//! A new item's `seq` lands past every sibling, so it never ties with a hand-set one.

use crate::tree_support;

use apinata_core::{CheckOptions, DropTarget, FindingKind, ItemPath, check_collection};
use tree_support::{ROOT, disk_seq, folder, ip, item, open, request};

/// A root whose folder `f` was numbered by hand to 7, above the six siblings' count.
fn hand_set_folder() -> Vec<(&'static str, String)> {
    let mut files = vec![
        ("opencollection.yml", ROOT.to_owned()),
        ("f/folder.yml", folder("f", 7)),
    ];
    for (rel, name, seq) in [
        ("a.yml", "a", 1),
        ("b.yml", "b", 2),
        ("c.yml", "c", 3),
        ("d.yml", "d", 4),
        ("e.yml", "e", 5),
        ("g.yml", "g", 6),
    ] {
        files.push((rel, request(name, seq)));
    }
    files
}

fn duplicates(root: &std::path::Path) -> Vec<String> {
    check_collection(root, &CheckOptions::default())
        .expect("checks")
        .findings()
        .iter()
        .filter(|f| f.kind() == FindingKind::DuplicateSequence)
        .map(|f| f.item().to_owned())
        .collect()
}

#[test]
fn clone_paste_and_create_all_land_past_a_hand_set_folder() {
    let files = hand_set_folder();
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let (dir, mut c) = open(&refs);
    let root = ItemPath::root();

    let cloned = c.clone_request(&ip("a.yml"), "a2").expect("clones");
    assert_eq!(disk_seq(&dir, cloned.as_str()), Some(8));
    let source = c.paste_source(&ip("b.yml")).expect("source");
    let pasted = c.paste(&root, &source).expect("pastes");
    assert_eq!(disk_seq(&dir, pasted.as_str()), Some(9));
    let created = c.create_request(&root, "new").expect("creates");
    assert_eq!(disk_seq(&dir, created.as_str()), Some(10));
    let made = c.create_folder(&root, "h").expect("creates");
    assert_eq!(
        disk_seq(&dir, &format!("{}/folder.yml", made.as_str())),
        Some(11)
    );
    let draft = c.create_draft(&root).expect("drafts");
    assert_eq!(c.request(&draft).expect("draft").seq(), Some(12));

    assert_eq!(duplicates(dir.path()), Vec::<String>::new());
}

#[test]
fn cloning_a_request_beside_a_densely_numbered_folder_does_not_tie() {
    // The reference counts only non-folder siblings here, so it would write 3 and tie with b.
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("a.yml", &request("a", 2)),
        ("b.yml", &request("b", 3)),
    ]);
    let cloned = c.clone_request(&ip("a.yml"), "a2").expect("clones");
    assert_eq!(disk_seq(&dir, cloned.as_str()), Some(4));
    let source = c.paste_source(&ip("b.yml")).expect("source");
    let pasted = c.paste(&ItemPath::root(), &source).expect("pastes");
    assert_eq!(disk_seq(&dir, pasted.as_str()), Some(5));
    assert_eq!(duplicates(dir.path()), Vec::<String>::new());
}

#[test]
fn an_app_item_with_the_highest_seq_is_passed_too() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("z.yml", &item("app", "z", 5)),
    ]);
    let created = c.create_request(&ItemPath::root(), "new").expect("creates");
    assert_eq!(disk_seq(&dir, created.as_str()), Some(6));
}

#[test]
fn a_dense_folder_still_gets_the_count_plus_one() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    let created = c.create_request(&ItemPath::root(), "new").expect("creates");
    assert_eq!(disk_seq(&dir, created.as_str()), Some(3));
}

/// A folder and two requests numbered 1, one by its missing `seq`, which counts as 1.
fn repeated_seqs() -> (tempfile::TempDir, apinata_core::Collection) {
    let unnumbered =
        "info:\n  name: b\n  type: http\nhttp:\n  method: GET\n  url: https://x.test/b\n";
    open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("a.yml", &request("a", 1)),
        ("b.yml", unnumbered),
    ])
}

#[test]
fn repeated_or_missing_seqs_still_count_every_sibling() {
    // Three siblings all at seq 1: the count gives 4 where the highest seq alone would give 2.
    let (dir, mut c) = repeated_seqs();
    let created = c.create_request(&ItemPath::root(), "new").expect("creates");
    assert_eq!(disk_seq(&dir, created.as_str()), Some(4));
    let (dir, mut c) = repeated_seqs();
    let cloned = c.clone_request(&ip("a.yml"), "a2").expect("clones");
    assert_eq!(disk_seq(&dir, cloned.as_str()), Some(4));
}

#[test]
fn a_request_moved_into_a_folder_lands_past_a_hand_set_sibling() {
    // The item's own seq equals the folder's highest, so keeping it would tie.
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
        ("a.yml", &request("a", 3)),
        ("f/x.yml", &request("x", 1)),
        ("f/y.yml", &request("y", 3)),
    ]);
    let moved = c
        .move_item(&ip("a.yml"), DropTarget::Inside(ip("f")))
        .expect("moves");
    assert_eq!(moved, ip("f/a.yml"));
    assert_eq!(disk_seq(&dir, "f/a.yml"), Some(4));
    assert_eq!(c.request(&moved).expect("in the tree").seq(), Some(4));
    assert_eq!(duplicates(dir.path()), Vec::<String>::new());
}

#[test]
fn an_item_moved_into_the_root_lands_past_every_root_sibling() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("g/folder.yml", &folder("g", 2)),
        ("b.yml", &request("b", 5)),
        ("g/z.yml", &request("z", 2)),
    ]);
    let moved = c
        .move_item(&ip("g/z.yml"), DropTarget::Inside(ItemPath::root()))
        .expect("moves");
    assert_eq!(moved, ip("z.yml"));
    assert_eq!(disk_seq(&dir, "z.yml"), Some(6));
    assert_eq!(duplicates(dir.path()), Vec::<String>::new());
}
