use apinata_app::sidebar::drop::{DraggedRow, DropZone, multi_drop};
use apinata_app::sidebar::model::NodeKind;
use apinata_core::ItemPath;

fn ip(p: &str) -> ItemPath {
    ItemPath::new(p).unwrap()
}
fn set(ps: &[&str]) -> Vec<ItemPath> {
    ps.iter().map(|p| ip(p)).collect()
}

#[test]
fn only_the_middle_of_a_folder_or_collection_takes_a_multi_drop() {
    let items = set(&["a.yml", "b.yml"]);
    let f = ip("f");
    assert_eq!(
        multi_drop(&items, &f, NodeKind::Folder, DropZone::Inside),
        Some(items.clone())
    );
    assert_eq!(
        multi_drop(&items, &f, NodeKind::Folder, DropZone::Before),
        None
    );
    assert_eq!(
        multi_drop(&items, &f, NodeKind::Folder, DropZone::After),
        None
    );
    assert_eq!(
        multi_drop(&items, &ip("c.yml"), NodeKind::Request, DropZone::Inside),
        None
    );
    assert_eq!(
        multi_drop(&items, &ip("w.yml"), NodeKind::Opaque, DropZone::Inside),
        None
    );
    assert_eq!(
        multi_drop(
            &set(&["f/x.yml", "a.yml"]),
            &ItemPath::root(),
            NodeKind::Collection,
            DropZone::Inside
        ),
        Some(set(&["f/x.yml"])),
        "a.yml is already in the root"
    );
    for zone in [DropZone::Before, DropZone::After] {
        assert_eq!(
            multi_drop(&items, &ItemPath::root(), NodeKind::Collection, zone),
            None,
            "a collection row takes only its middle"
        );
    }
    for kind in [
        NodeKind::Invalid,
        NodeKind::Draft,
        NodeKind::UnsavedGroup,
        NodeKind::Example,
    ] {
        assert_eq!(multi_drop(&items, &f, kind, DropZone::Inside), None);
    }
}

#[test]
fn a_target_that_is_or_lies_inside_a_dragged_item_rejects_the_whole_drop() {
    assert_eq!(
        multi_drop(
            &set(&["a.yml", "f"]),
            &ip("f"),
            NodeKind::Folder,
            DropZone::Inside
        ),
        None
    );
    assert_eq!(
        multi_drop(
            &set(&["a.yml", "f"]),
            &ip("f/g"),
            NodeKind::Folder,
            DropZone::Inside
        ),
        None
    );
    assert_eq!(
        multi_drop(
            &set(&["a.yml", "fx"]),
            &ip("f"),
            NodeKind::Folder,
            DropZone::Inside
        ),
        Some(set(&["a.yml", "fx"])),
        "a sibling with a longer name is no ancestor"
    );
}

#[test]
fn items_already_inside_are_skipped_and_nothing_left_is_no_drop() {
    assert_eq!(
        multi_drop(
            &set(&["f/x.yml", "a.yml"]),
            &ip("f"),
            NodeKind::Folder,
            DropZone::Inside
        ),
        Some(set(&["a.yml"]))
    );
    assert_eq!(
        multi_drop(
            &set(&["f/x.yml"]),
            &ip("f"),
            NodeKind::Folder,
            DropZone::Inside
        ),
        None
    );
}

#[test]
fn a_set_payload_reads_n_items_and_lists_every_path() {
    let d = DraggedRow::for_set("/c".into(), ip("b.yml"), &set(&["a.yml", "b.yml", "c.yml"]));
    assert_eq!(&*d.name, "3 items");
    assert_eq!(d.items(), set(&["b.yml", "a.yml", "c.yml"]));
    let one = DraggedRow::new("/c".into(), ip("a.yml"), "A".into());
    assert_eq!((&*one.name, one.items()), ("A", set(&["a.yml"])));
}

#[test]
fn a_set_that_normalises_to_one_item_drags_as_a_plain_row() {
    let d = DraggedRow::for_set("/c".into(), ip("f"), &set(&["f", "f/x.yml"]));
    assert!(d.also.is_empty());
    assert_eq!(d.items(), set(&["f"]));
    assert_ne!(&*d.name, "1 items");
}
