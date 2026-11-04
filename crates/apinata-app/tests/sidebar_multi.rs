use crate::support;

use apinata_app::sidebar::model::{Expanded, Node, build};
use apinata_app::sidebar::multi::{Multi, SelKey, row_keys, visible_keys};
use apinata_app::snapshot::SnapshotCache;
use apinata_core::{Collection, ItemPath};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use support::{ROOT, SOCKET, folder, ip, request, write_tree};

fn k(p: &str) -> SelKey {
    SelKey::new("/c", ip(p))
}
fn other(p: &str) -> SelKey {
    SelKey::new("/d", ip(p))
}
fn order() -> Vec<SelKey> {
    ["a.yml", "b.yml", "c.yml", "d.yml"].map(k).to_vec()
}
fn paths(m: &Multi) -> Vec<String> {
    m.keys()
        .iter()
        .map(|k| k.path.as_str().to_owned())
        .collect()
}

fn nodes_of(dir: &tempfile::TempDir, open: &[&str]) -> Vec<Node> {
    let c = SnapshotCache::new().build(&Collection::open(dir.path()).unwrap());
    let mut set: HashSet<ItemPath> = open.iter().map(|p| ip(p)).collect();
    set.insert(ItemPath::root());
    let expanded = Expanded::from([(dir.path().to_path_buf(), set)]);
    build(&[(dir.path(), "c", Some(&c))], &[], &expanded, "")
}

#[test]
fn visible_keys_follow_expansion_and_skip_unselectable_rows() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("f/x.yml", &request("X", "http://x/x")),
        ("a.yml", &request("A", "http://x/a")),
        ("w.yml", SOCKET),
        ("broken.yml", "info: [\n"),
    ]);
    let seen = |open: &[&str]| -> BTreeSet<String> {
        visible_keys(&nodes_of(&dir, open))
            .into_iter()
            .map(|k| k.path.as_str().to_owned())
            .collect()
    };
    let top: BTreeSet<String> = ["a.yml", "f", "w.yml"].map(String::from).into();
    assert_eq!(
        seen(&[]),
        top,
        "no collection row, no invalid row, no hidden child"
    );
    let mut open = top.clone();
    open.insert("f/x.yml".into());
    assert_eq!(seen(&["f"]), open);
}

#[test]
fn row_keys_list_every_item_of_a_snapshot() {
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("F")),
        ("f/x.yml", &request("X", "http://x/x")),
        ("a.yml", &request("A", "http://x/a")),
    ]);
    let c = SnapshotCache::new().build(&Collection::open(dir.path()).unwrap());
    let got: BTreeSet<String> = row_keys(dir.path(), &c.rows)
        .into_iter()
        .map(|k| k.path.as_str().to_owned())
        .collect();
    assert_eq!(got, ["a.yml", "f", "f/x.yml"].map(String::from).into());
}

#[test]
fn toggle_seeds_with_the_focus_row_and_sets_the_anchor() {
    let mut m = Multi::default();
    assert_eq!(
        m.toggle(k("c.yml"), Some(&k("a.yml")), &order()),
        Some(k("c.yml"))
    );
    assert_eq!(paths(&m), ["a.yml", "c.yml"]);
    assert_eq!(m.anchor(), Some(&k("c.yml")));
    assert!(m.is_active());
}

#[test]
fn toggling_the_focus_row_off_moves_focus_to_the_next_member_else_the_previous() {
    let mut m = Multi::default();
    m.toggle(k("b.yml"), Some(&k("a.yml")), &order());
    m.toggle(k("d.yml"), Some(&k("b.yml")), &order());
    // Members a, b, d; focus b goes to the next member, d.
    assert_eq!(
        m.toggle(k("b.yml"), Some(&k("b.yml")), &order()),
        Some(k("d.yml"))
    );
    // Focus d has no later member, so the previous one, a.
    assert_eq!(
        m.toggle(k("d.yml"), Some(&k("d.yml")), &order()),
        Some(k("a.yml"))
    );
    assert_eq!(paths(&m), ["a.yml"]);
    assert!(!m.is_active(), "one key is the single selection again");
    // Toggling a non-focus member off leaves focus alone.
    let mut m = Multi::default();
    m.toggle(k("b.yml"), Some(&k("a.yml")), &order());
    assert_eq!(m.toggle(k("a.yml"), Some(&k("b.yml")), &order()), None);
    assert_eq!(paths(&m), ["b.yml"]);
}

#[test]
fn a_toggle_without_a_focus_row_or_in_another_root_replaces_the_set() {
    let mut m = Multi::default();
    assert_eq!(m.toggle(k("b.yml"), None, &order()), Some(k("b.yml")));
    assert_eq!(paths(&m), ["b.yml"]);
    m.toggle(k("c.yml"), Some(&k("b.yml")), &order());
    let mixed: Vec<SelKey> = order().into_iter().chain([other("x.yml")]).collect();
    m.toggle(other("x.yml"), Some(&k("c.yml")), &mixed);
    assert_eq!(m.keys().iter().collect::<Vec<_>>(), [&other("x.yml")]);
}

#[test]
fn a_range_runs_from_the_anchor_and_the_anchor_stays() {
    let mut m = Multi::default();
    // No anchor: the focus row anchors.
    assert_eq!(
        m.select_range(k("d.yml"), Some(&k("b.yml")), &order()),
        k("d.yml")
    );
    assert_eq!(paths(&m), ["b.yml", "c.yml", "d.yml"]);
    assert_eq!(m.anchor(), Some(&k("b.yml")));
    // A second shift-click measures from the same anchor, even upward.
    m.select_range(k("c.yml"), Some(&k("d.yml")), &order());
    assert_eq!(paths(&m), ["b.yml", "c.yml"]);
    m.select_range(k("a.yml"), Some(&k("c.yml")), &order());
    assert_eq!(paths(&m), ["a.yml", "b.yml"]);
}

#[test]
fn a_range_stays_in_the_anchors_root_and_a_foreign_click_replaces_the_set() {
    let mixed: Vec<SelKey> = ["a.yml", "b.yml"]
        .map(k)
        .into_iter()
        .chain([other("x.yml")])
        .collect();
    let mut m = Multi::default();
    m.select_range(k("b.yml"), Some(&k("a.yml")), &mixed);
    assert_eq!(paths(&m), ["a.yml", "b.yml"]);
    m.select_range(other("x.yml"), Some(&k("b.yml")), &mixed);
    assert_eq!(m.keys().iter().collect::<Vec<_>>(), [&other("x.yml")]);
    // A focus row hidden from `order` cannot anchor a range.
    let mut m = Multi::default();
    m.select_range(k("c.yml"), Some(&k("gone.yml")), &order());
    assert_eq!(paths(&m), ["c.yml"]);
}

#[test]
fn extend_moves_focus_one_row_and_stops_at_the_ends() {
    let mut m = Multi::default();
    assert_eq!(m.extend(true, &k("b.yml"), &order()), Some(k("c.yml")));
    assert_eq!(paths(&m), ["b.yml", "c.yml"]);
    assert_eq!(m.extend(true, &k("c.yml"), &order()), Some(k("d.yml")));
    assert_eq!(paths(&m), ["b.yml", "c.yml", "d.yml"]);
    assert_eq!(m.extend(true, &k("d.yml"), &order()), None);
    assert_eq!(m.extend(false, &k("d.yml"), &order()), Some(k("c.yml")));
    assert_eq!(paths(&m), ["b.yml", "c.yml"], "shrinks toward the anchor");
    let mixed: Vec<SelKey> = ["a.yml"]
        .map(k)
        .into_iter()
        .chain([other("x.yml")])
        .collect();
    assert_eq!(
        Multi::default().extend(true, &k("a.yml"), &mixed),
        None,
        "never crosses a root"
    );
}

#[test]
fn normalising_drops_descendants_of_selected_folders_and_sorts_by_row_order() {
    let rows: Vec<SelKey> = [
        "f",
        "f/g",
        "f/g/y.yml",
        "f/x.yml",
        "fx.yml",
        "a.yml",
        "b.yml",
    ]
    .map(k)
    .to_vec();
    let mut m = Multi::default();
    m.admit(["b.yml", "f/g/y.yml", "a.yml", "f", "fx.yml"].map(k));
    let got: Vec<String> = m
        .normalised(&rows)
        .into_iter()
        .map(|k| k.path.as_str().to_owned())
        .collect();
    assert_eq!(
        got,
        ["f", "fx.yml", "a.yml", "b.yml"],
        "fx.yml is no child of f"
    );
}

#[test]
fn rebase_follows_a_moved_folder_and_retain_returns_what_it_drops() {
    let mut m = Multi::default();
    // Seeded with the focus row a.yml, then f/x.yml toggled on and made the anchor.
    m.toggle(k("f/x.yml"), Some(&k("a.yml")), &[]);
    m.rebase(Path::new("/c"), &ip("f"), &ip("h"));
    assert_eq!(paths(&m), ["a.yml", "h/x.yml"]);
    assert_eq!(m.anchor(), Some(&k("h/x.yml")));
    m.rebase(Path::new("/other"), &ip("a.yml"), &ip("z.yml"));
    assert_eq!(paths(&m), ["a.yml", "h/x.yml"], "another root is untouched");
    let dropped = m.retain(|key| key.path.as_str() != "a.yml");
    assert_eq!(dropped, [k("a.yml")]);
    assert_eq!(paths(&m), ["h/x.yml"]);
}
