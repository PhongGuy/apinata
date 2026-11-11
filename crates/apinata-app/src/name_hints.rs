//! The one rule for telling apart items that show the same name, wherever names leave their
//! tree: each gets a hint, the shortest trailing part of its place that no same-named item
//! shares, and siblings fall back to their file names.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use apinata_core::ItemPath;

use crate::snapshot::Content;

/// One shown item: its visible name, the places around it outermost first, and the file
/// name that tells it apart from a sibling sharing its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The text the item shows.
    pub name: String,
    /// Where the item sits, outermost first: its collection, then each enclosing folder.
    pub place: Vec<String>,
    /// The item's file name, for items backed by a file.
    pub file: Option<String>,
}

/// Each item's hint, by index; `None` where no other item shows its name. Segments join with "/".
pub fn hints(items: &[Shown]) -> Vec<Option<String>> {
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (ix, item) in items.iter().enumerate() {
        groups.entry(item.name.as_str()).or_default().push(ix);
    }
    let mut out = vec![None; items.len()];
    for group in groups.values().filter(|g| g.len() > 1) {
        let first = &items[group[0]].place;
        let common = (0..first.len())
            .take_while(|&k| group.iter().all(|&o| items[o].place.get(k) == first.get(k)))
            .count();
        let found: Vec<Option<String>> = (group.iter())
            .map(|&ix| hint(items, group, common, ix))
            .collect();
        // A top-level item names its collection, which a folder of the same name echoes; such
        // twins show their whole place instead.
        for (&ix, h) in group.iter().zip(&found) {
            let twin = h.is_some() && found.iter().filter(|o| *o == h).count() > 1;
            out[ix] = if twin {
                let mut parts = items[ix].place.clone();
                parts.extend(sibling_file(items, group, ix));
                Some(parts.join("/"))
            } else {
                h.clone()
            };
        }
    }
    out
}

// The leading `common` segments every item of the group shares say nothing, so the search runs
// on what is left; an item with nothing left sits at the top of the last shared place and
// names it.
fn hint(items: &[Shown], group: &[usize], common: usize, ix: usize) -> Option<String> {
    let place = &items[ix].place;
    let rest = |o: usize| &items[o].place[common..];
    let differing: Vec<usize> = (group.iter().copied())
        .filter(|&o| items[o].place != *place)
        .collect();
    let longest = differing.iter().map(|&o| rest(o).len()).max().unwrap_or(0);
    let k = (1..=longest.max(rest(ix).len()))
        .find(|&k| {
            differing
                .iter()
                .all(|&o| tail(rest(ix), k) != tail(rest(o), k))
        })
        .filter(|_| !differing.is_empty())
        .unwrap_or(0);
    let mut parts: Vec<String> = tail(rest(ix), k).to_vec();
    if k > 0 && parts.is_empty() && common > 0 {
        parts.push(place[common - 1].clone());
    }
    parts.extend(sibling_file(items, group, ix));
    (!parts.is_empty()).then(|| parts.join("/"))
}

// The item's file name when another item of the group shares its whole place.
fn sibling_file(items: &[Shown], group: &[usize], ix: usize) -> Option<String> {
    let place = &items[ix].place;
    (group.iter().any(|&o| o != ix && items[o].place == *place))
        .then(|| items[ix].file.clone())
        .flatten()
}

// The last `k` segments, or all of them when there are fewer.
fn tail(segments: &[String], k: usize) -> &[String] {
    &segments[segments.len().saturating_sub(k)..]
}

/// The hint of each (root, name) collection: the shortest part of its folder path that tells
/// it apart from collections showing the same name.
pub fn collection_hints(collections: &[(&Path, &str)]) -> Vec<Option<String>> {
    let shown: Vec<Shown> = collections
        .iter()
        .map(|&(root, name)| Shown {
            name: name.to_owned(),
            place: (root.components())
                .filter(|c| matches!(c, Component::Normal(_) | Component::Prefix(_)))
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect(),
            file: None,
        })
        .collect();
    hints(&shown)
}

/// What stands for each collection in an item's place: its name, or its hint when another
/// collection shows the same name.
pub fn collection_tags(collections: &[(&Path, &str)]) -> HashMap<PathBuf, String> {
    let hints = collection_hints(collections);
    collections
        .iter()
        .zip(hints)
        .map(|(&(root, name), hint)| (root.to_path_buf(), hint.unwrap_or_else(|| name.into())))
        .collect()
}

/// The place of `item` in its collection: the collection's `tag`, then each enclosing folder's
/// shown name, its directory name until the folder has loaded.
pub fn item_place(tag: &str, content: Option<&Content>, item: &ItemPath) -> Vec<String> {
    let mut folders = Vec::new();
    let mut parent = item.parent().filter(|p| !p.is_root());
    while let Some(p) = parent {
        let name = content
            .and_then(|c| c.folders.get(&p))
            .and_then(|f| f.name.clone())
            .unwrap_or_else(|| p.file_name().to_owned());
        parent = p.parent().filter(|p| !p.is_root());
        folders.push(name);
    }
    std::iter::once(tag.to_owned())
        .chain(folders.into_iter().rev())
        .collect()
}
