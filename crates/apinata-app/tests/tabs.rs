use apinata_app::tabs::{Side, TabEntry, TabId, TabKind, Tabs, tab_details};
use apinata_core::ItemPath;
use std::path::{Path, PathBuf};

const R: &str = "/c/a";

fn ip(rel: &str) -> ItemPath {
    ItemPath::new(rel).unwrap()
}

fn req(rel: &str) -> TabKind {
    TabKind::Request(ip(rel))
}

fn root() -> &'static Path {
    Path::new(R)
}

fn names(t: &Tabs) -> Vec<&str> {
    t.entries()
        .iter()
        .filter_map(|e| e.item())
        .map(ItemPath::as_str)
        .collect()
}

fn active(t: &Tabs) -> Option<&str> {
    t.active().and_then(|e| e.item()).map(ItemPath::as_str)
}

#[test]
fn a_preview_tab_is_replaced_by_the_next_preview() {
    let mut t = Tabs::new();
    let a = t.open(root(), req("a.yml"), true);
    let b = t.open(root(), req("b.yml"), true);
    assert_ne!(a, b);
    assert_eq!(names(&t), ["b.yml"]);
    assert!(t.get(a).is_none());
    assert!(t.active().unwrap().preview);
    t.keep(b);
    t.open(root(), req("c.yml"), true);
    assert_eq!(names(&t), ["b.yml", "c.yml"]);
    let previews: Vec<bool> = t.entries().iter().map(|e| e.preview).collect();
    assert_eq!(previews, [false, true]);
}

#[test]
fn opening_an_open_item_activates_it_and_a_permanent_open_pins_it() {
    let mut t = Tabs::new();
    let a = t.open(root(), req("a.yml"), false);
    let b = t.open(root(), req("b.yml"), true);
    assert_eq!(t.open(root(), req("a.yml"), true), a);
    assert_eq!(t.active().unwrap().id, a);
    assert!(!t.get(a).unwrap().preview);
    assert_eq!(t.open(root(), req("b.yml"), false), b);
    assert!(!t.get(b).unwrap().preview);
    t.open(Path::new("/c/b"), req("a.yml"), false);
    assert_eq!(t.entries().len(), 3);
}

#[test]
fn closing_activates_the_right_neighbour_then_the_left() {
    let mut t = Tabs::new();
    let a = t.open(root(), req("a.yml"), false);
    let b = t.open(root(), req("b.yml"), false);
    let c = t.open(root(), req("c.yml"), false);
    assert!(t.activate(b));
    assert_eq!(t.close(b).map(|e| e.id), Some(b));
    assert_eq!(active(&t), Some("c.yml"));
    t.close(c);
    assert_eq!(active(&t), Some("a.yml"));
    t.close(a);
    assert!(t.active().is_none());
    assert!(t.close(a).is_none());
}

#[test]
fn closed_tabs_reopen_newest_first_and_replaced_previews_are_forgotten() {
    let mut t = Tabs::new();
    t.open(root(), req("p.yml"), true);
    t.open(root(), req("q.yml"), true);
    let a = t.open(root(), req("a.yml"), false);
    let b = t.open(root(), req("b.yml"), false);
    t.close(a);
    t.close(b);
    assert_eq!(t.pop_closed(), Some((PathBuf::from(R), req("b.yml"))));
    assert_eq!(t.pop_closed(), Some((PathBuf::from(R), req("a.yml"))));
    assert_eq!(t.pop_closed(), None);
}

#[test]
fn tabs_activate_by_index_last_and_offset() {
    let mut t = Tabs::new();
    for rel in ["a.yml", "b.yml", "c.yml"] {
        t.open(root(), req(rel), false);
    }
    assert!(t.activate_index(0));
    assert_eq!(active(&t), Some("a.yml"));
    assert!(!t.activate_index(5));
    assert_eq!(active(&t), Some("a.yml"));
    assert!(t.activate_last());
    assert_eq!(active(&t), Some("c.yml"));
    t.activate_offset(1);
    assert_eq!(active(&t), Some("a.yml"));
    t.activate_offset(-1);
    assert_eq!(active(&t), Some("c.yml"));
}

#[test]
fn a_move_rebases_tabs_at_and_under_its_path_in_that_root_only() {
    let mut t = Tabs::new();
    for rel in ["users/a.yml", "users/admin/b.yml", "other.yml"] {
        t.open(root(), req(rel), false);
    }
    t.open(Path::new("/c/b"), req("users/a.yml"), false);
    t.follow_move(root(), &ip("users"), &ip("team/users"));
    assert_eq!(
        names(&t),
        [
            "team/users/a.yml",
            "team/users/admin/b.yml",
            "other.yml",
            "users/a.yml"
        ]
    );
    t.follow_move(root(), &ip("other.yml"), &ip("renamed.yml"));
    assert_eq!(names(&t)[2], "renamed.yml");
}

#[test]
fn retain_drops_tabs_without_remembering_them() {
    let mut t = Tabs::new();
    t.open(root(), req("a.yml"), false);
    t.open(Path::new("/c/b"), req("b.yml"), false);
    let gone = t.retain(|e| e.root != Path::new(R));
    assert_eq!(gone.len(), 1);
    assert_eq!(names(&t), ["b.yml"]);
    assert_eq!(active(&t), Some("b.yml"));
    assert_eq!(t.pop_closed(), None);
}

fn entry(id: u64, kind: TabKind) -> TabEntry {
    TabEntry {
        id: TabId(id),
        root: PathBuf::from(R),
        kind,
        preview: false,
        pinned: false,
    }
}

fn places(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect()
}

#[test]
fn shared_names_show_the_shortest_differing_place_and_unique_names_nothing() {
    let entries = [
        entry(1, req("request_1.yml")),
        entry(2, req("folder_1/request_1.yml")),
        entry(3, req("request_2.yml")),
        entry(4, req("a/b/request_1.yml")),
        entry(5, TabKind::Environments),
    ];
    let labels: Vec<String> = [
        "request_1",
        "request_1",
        "request_2",
        "request_1",
        "Environments",
    ]
    .map(String::from)
    .to_vec();
    let places = places(&[&["c"], &["c", "folder_1"], &["c"], &["c", "a", "b"], &["c"]]);
    assert_eq!(
        tab_details(&entries, &labels, &places),
        vec![
            Some("c".to_string()),
            Some("folder_1".to_string()),
            None,
            Some("b".to_string()),
            None,
        ],
        "the top-level request names its collection"
    );
}

#[test]
fn a_request_sharing_a_label_with_a_settings_tab_shows_its_file() {
    let entries = [
        entry(1, req("Environments.yml")),
        entry(2, TabKind::Environments),
    ];
    let labels = vec!["Environments".to_string(), "Environments".to_string()];
    assert_eq!(
        tab_details(&entries, &labels, &places(&[&["c"], &["c"]])),
        vec![Some("Environments.yml".to_string()), None]
    );
}

#[test]
fn the_environments_tabs_of_two_collections_name_their_collections() {
    let entries = [
        entry(1, TabKind::Environments),
        entry(2, TabKind::Environments),
    ];
    let labels = vec!["Environments".to_string(), "Environments".to_string()];
    assert_eq!(
        tab_details(&entries, &labels, &places(&[&["Demo"], &["Shop"]])),
        vec![Some("Demo".to_string()), Some("Shop".to_string())]
    );
}

#[test]
fn no_tabs_no_details() {
    assert!(tab_details(&[], &[], &[]).is_empty());
}

fn three() -> (Tabs, TabId, TabId, TabId) {
    let mut t = Tabs::new();
    let a = t.open(root(), req("a.yml"), false);
    let b = t.open(root(), req("b.yml"), false);
    let c = t.open(root(), req("c.yml"), false);
    (t, a, b, c)
}

#[test]
fn move_to_places_a_tab_on_either_side_of_its_anchor() {
    let (mut t, a, b, c) = three();
    assert!(t.move_to(a, c, Side::After));
    assert_eq!(names(&t), ["b.yml", "c.yml", "a.yml"]);
    assert!(t.move_to(a, b, Side::Before));
    assert_eq!(names(&t), ["a.yml", "b.yml", "c.yml"]);
    assert!(t.move_to(c, a, Side::Before));
    assert_eq!(names(&t), ["c.yml", "a.yml", "b.yml"]);
}

#[test]
fn move_to_reports_no_change_for_a_tab_already_in_place_or_dropped_on_itself() {
    let (mut t, a, b, c) = three();
    assert!(!t.move_to(a, a, Side::Before));
    assert!(!t.move_to(a, b, Side::Before), "a is already before b");
    assert!(!t.move_to(b, a, Side::After), "b is already after a");
    assert!(!t.move_to(c, b, Side::After));
    assert_eq!(names(&t), ["a.yml", "b.yml", "c.yml"]);
}

#[test]
fn move_to_ignores_a_tab_of_another_collection() {
    let mut t = Tabs::new();
    let a = t.open(root(), req("a.yml"), false);
    let other = t.open(Path::new("/c/b"), req("a.yml"), false);
    let b = t.open(root(), req("b.yml"), false);
    assert!(!t.move_to(a, other, Side::After));
    assert!(!t.move_to(other, b, Side::Before));
    let order: Vec<TabId> = t.entries().iter().map(|e| e.id).collect();
    assert_eq!(order, [a, other, b]);
}

#[test]
fn move_to_keeps_the_active_tab() {
    let (mut t, a, _, c) = three();
    t.activate(c);
    t.move_to(a, c, Side::After);
    assert_eq!(active(&t), Some("c.yml"));
}

fn open3(t: &mut Tabs) -> [TabId; 4] {
    ["a.yml", "b.yml", "c.yml", "d.yml"].map(|n| t.open(root(), req(n), false))
}

#[test]
fn pinning_moves_a_tab_to_the_end_of_the_pinned_group() {
    let mut t = Tabs::new();
    let [a, b, c, d] = open3(&mut t);
    assert!(t.set_pinned(c, true));
    assert_eq!(names(&t), ["c.yml", "a.yml", "b.yml", "d.yml"]);
    assert!(t.set_pinned(d, true));
    assert_eq!(names(&t), ["c.yml", "d.yml", "a.yml", "b.yml"]);
    assert!(!t.set_pinned(d, true), "already pinned");
    assert!(t.get(c).unwrap().pinned && !t.get(a).unwrap().pinned && !t.get(b).unwrap().pinned);
}

#[test]
fn unpinning_lands_right_after_the_remaining_pinned_tabs() {
    let mut t = Tabs::new();
    let [a, b, c, _d] = open3(&mut t);
    t.set_pinned(a, true);
    t.set_pinned(b, true);
    t.set_pinned(c, true);
    assert_eq!(names(&t), ["a.yml", "b.yml", "c.yml", "d.yml"]);
    assert!(t.set_pinned(a, false));
    assert_eq!(names(&t), ["b.yml", "c.yml", "a.yml", "d.yml"]);
    assert!(!t.set_pinned(a, false), "already unpinned");
    assert!(!t.set_pinned(TabId(99), true), "not open");
}

#[test]
fn pinning_a_preview_tab_makes_it_permanent() {
    let mut t = Tabs::new();
    let a = t.open(root(), req("a.yml"), true);
    t.set_pinned(a, true);
    assert!(!t.get(a).unwrap().preview);
    t.open(root(), req("b.yml"), true);
    assert_eq!(names(&t), ["a.yml", "b.yml"]);
}

#[test]
fn new_tabs_open_after_the_pinned_ones() {
    let mut t = Tabs::new();
    let a = t.open(root(), req("a.yml"), false);
    t.open(root(), req("b.yml"), false);
    t.set_pinned(a, true);
    t.open(root(), req("c.yml"), true);
    t.open(root(), req("d.yml"), true);
    assert_eq!(names(&t), ["a.yml", "b.yml", "d.yml"]);
    let closed = t.close(a).unwrap();
    t.open(root(), closed.kind, false);
    assert!(
        !t.entries().last().unwrap().pinned,
        "a reopened tab is not pinned"
    );
}

#[test]
fn a_drag_stays_inside_its_own_group() {
    let mut t = Tabs::new();
    let [a, b, c, d] = open3(&mut t);
    t.set_pinned(a, true);
    t.set_pinned(b, true);
    // Order is now a, b, c, d with a and b pinned.
    assert!(
        !t.move_to(c, b, Side::After),
        "unpinned onto the pinned side"
    );
    assert!(!t.move_to(c, a, Side::Before));
    assert!(
        !t.move_to(a, c, Side::Before),
        "pinned onto the unpinned side"
    );
    assert!(!t.move_to(b, d, Side::After));
    assert_eq!(names(&t), ["a.yml", "b.yml", "c.yml", "d.yml"]);
    assert!(t.move_to(b, a, Side::Before));
    assert!(t.move_to(d, c, Side::Before));
    assert_eq!(names(&t), ["b.yml", "a.yml", "d.yml", "c.yml"]);
}

#[test]
fn the_unchecked_moves_keep_pinned_tabs_in_front() {
    let mut t = Tabs::new();
    let [a, b, c, _d] = open3(&mut t);
    t.set_pinned(a, true);
    t.move_before(c, a);
    assert_eq!(names(&t)[0], "a.yml");
    t.move_after(a, b);
    assert_eq!(names(&t)[0], "a.yml");
}

#[test]
fn closing_and_retaining_keep_pinned_tabs_in_front() {
    let mut t = Tabs::new();
    let [a, b, c, d] = open3(&mut t);
    t.set_pinned(c, true);
    t.set_pinned(d, true);
    t.close(c);
    assert_eq!(names(&t), ["d.yml", "a.yml", "b.yml"]);
    let gone = t.retain(|e| e.pinned || e.id == b);
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].id, a);
    assert_eq!(names(&t), ["d.yml", "b.yml"]);
}
