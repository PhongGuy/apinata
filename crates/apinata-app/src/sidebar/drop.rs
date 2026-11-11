//! Where a dragged sidebar row lands: the zone under the pointer and the move it means.

use std::path::PathBuf;

use apinata_core::{DropTarget, ItemPath};
use gpui_kit::{
    Bounds, Context, IntoElement, ParentElement, Pixels, Render, SharedString, Styled, Window, div,
};

use super::model::NodeKind;
use crate::stale::is_within;
use crate::style;

/// The part of a row the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropZone {
    /// The top quarter.
    Before,
    /// The middle half.
    Inside,
    /// The bottom quarter.
    After,
}

/// The zone of `bounds` that height `y` falls in: the top quarter is `Before`, the bottom
/// quarter `After`, the rest `Inside`.
pub fn drop_zone(y: Pixels, bounds: Bounds<Pixels>) -> DropZone {
    let fraction = (y - bounds.origin.y) / bounds.size.height;
    if fraction < 0.25 {
        DropZone::Before
    } else if fraction > 0.75 {
        DropZone::After
    } else {
        DropZone::Inside
    }
}

/// The move that dropping `dragged` on the `target` row of kind `kind` in `zone` means.
/// None for the item itself, its own subtree, a no-op into its own parent, an invalid row, or a draft row or its group.
pub fn drop_target(
    dragged: &ItemPath,
    target: &ItemPath,
    kind: NodeKind,
    zone: DropZone,
) -> Option<DropTarget> {
    // Only a folder takes all three zones; a leaf row has no inside, and a collection row
    // has nothing above or below it within the collection. An Empty row stands for its folder.
    let to = match (kind, zone) {
        (NodeKind::Invalid | NodeKind::Draft | NodeKind::UnsavedGroup, _) => return None,
        (NodeKind::Collection, _) => DropTarget::Inside(ItemPath::root()),
        (NodeKind::Empty, _) => DropTarget::Inside(target.clone()),
        (NodeKind::Folder, DropZone::Inside) => DropTarget::Inside(target.clone()),
        (_, DropZone::Before) => DropTarget::Before(target.clone()),
        (_, _) => DropTarget::After(target.clone()),
    };
    let into_own_parent =
        matches!(&to, DropTarget::Inside(dir) if dragged.parent().as_ref() == Some(dir));
    let into_itself = !dragged.is_root() && is_within(target, dragged);
    (!into_own_parent && !into_itself).then_some(to)
}

/// The items a multi-payload drop moves into `target`, or None when the whole drop is rejected:
/// only the Inside zone of a Folder or Collection row is valid; any item that is the target or
/// contains it rejects the drop; items already directly inside the target are left out.
pub fn multi_drop(
    items: &[ItemPath],
    target: &ItemPath,
    kind: NodeKind,
    zone: DropZone,
) -> Option<Vec<ItemPath>> {
    // A collection row stands for the root directory.
    let dir = match (kind, zone) {
        (NodeKind::Collection, DropZone::Inside) => ItemPath::root(),
        (NodeKind::Folder, DropZone::Inside) | (NodeKind::Empty, _) => target.clone(),
        _ => return None,
    };
    if items.iter().any(|i| !i.is_root() && is_within(&dir, i)) {
        return None;
    }
    let moving: Vec<ItemPath> = items
        .iter()
        .filter(|i| i.parent().as_ref() != Some(&dir))
        .cloned()
        .collect();
    (!moving.is_empty()).then_some(moving)
}

/// The payload of a row being dragged.
#[derive(Clone)]
pub struct DraggedRow {
    /// The collection's root.
    pub root: PathBuf,
    /// The dragged item.
    pub path: ItemPath,
    /// Its name, for the drag preview.
    pub name: SharedString,
    /// The other members of a dragged selection, in sidebar order; empty for one row.
    pub also: Vec<ItemPath>,
}

impl DraggedRow {
    /// A payload of one row.
    pub fn new(root: PathBuf, path: ItemPath, name: SharedString) -> DraggedRow {
        DraggedRow {
            root,
            path,
            name,
            also: Vec::new(),
        }
    }

    /// A payload of the selection `set` (sidebar order, containing `path`); the preview reads
    /// `N items`. Members under another member's folder travel with it, and a set that leaves
    /// one item is a plain single payload.
    pub fn for_set(root: PathBuf, path: ItemPath, set: &[ItemPath]) -> DraggedRow {
        let top: Vec<&ItemPath> = set
            .iter()
            .filter(|p| !set.iter().any(|o| o != *p && crate::stale::is_within(p, o)))
            .collect();
        if top.len() < 2 {
            let only = top.first().map_or_else(|| path.clone(), |p| (*p).clone());
            let name = only.file_name().to_owned();
            return DraggedRow::new(root, only, name.into());
        }
        let also = top
            .iter()
            .filter(|p| ***p != path)
            .map(|p| (*p).clone())
            .collect();
        DraggedRow {
            root,
            path,
            name: format!("{} items", top.len()).into(),
            also,
        }
    }

    /// Every dragged item: the grabbed row first, then the rest.
    pub fn items(&self) -> Vec<ItemPath> {
        std::iter::once(self.path.clone())
            .chain(self.also.iter().cloned())
            .collect()
    }
}

/// The label that follows the pointer during a drag.
pub(crate) struct DragPreview(pub SharedString);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(style::SPACE_2)
            .py(style::SPACE_0)
            .child(self.0.clone())
    }
}
