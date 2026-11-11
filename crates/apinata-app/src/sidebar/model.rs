//! Sidebar nodes built from collection snapshots: a pure function of each collection's
//! content, the view's expansion sets and the filter text.

use std::collections::{HashMap, HashSet};
use std::iter::Peekable;
use std::path::{Path, PathBuf};

use apinata_core::ItemPath;
use gpui_kit::SharedString;
use gpui_kit::component::tree::TreeItem;

use super::terms::FilterTerms;
use crate::name_hints::{Shown, hints};
use crate::snapshot::{Content, DiskState, Row, RowKind};

/// The expanded folders of each collection, by root; the root path stands for the collection row.
pub type Expanded = HashMap<PathBuf, HashSet<ItemPath>>;

/// What a sidebar node shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// An open collection's root row.
    Collection,
    /// A sub-folder.
    Folder,
    /// An `http` request.
    Request,
    /// A request file of a type Apinata does not model.
    Opaque,
    /// A file that failed to load.
    Invalid,
    /// A saved example of the request it is nested under.
    Example,
    /// The group heading a collection's open drafts.
    UnsavedGroup,
    /// An open draft tab that has never been saved.
    Draft,
    /// The muted row an empty folder shows when opened.
    Empty,
}

/// An open draft tab, as the sidebar lists it under its collection's Unsaved group.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub struct OpenDraft {
    pub root: PathBuf,
    pub path: ItemPath,
    pub name: String,
    /// Empty while the draft's view has not loaded; the row then shows no method.
    pub method: String,
}

/// The label of the group that lists a collection's open drafts.
pub const UNSAVED_LABEL: &str = "Unsaved";

/// The tree id of a collection's Unsaved group, `"{collection}:#unsaved"`.
pub fn unsaved_id(collection: usize) -> SharedString {
    format!("{collection}:#unsaved").into()
}

/// The label of the row an empty folder shows.
pub const EMPTY_LABEL: &str = "Empty";

/// The tree id of the Empty row in the folder at `path`, `"{collection}:{path}:#empty"`.
pub fn empty_id(collection: usize, path: &ItemPath) -> SharedString {
    format!("{collection}:{}:#empty", path.as_str()).into()
}

/// One sidebar row and the rows nested under it.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub struct Node {
    pub id: SharedString,
    pub collection: usize,
    pub root: PathBuf,
    pub path: ItemPath,
    pub kind: NodeKind,
    pub name: String,
    /// A request's full HTTP method, as written in its file.
    pub method: Option<String>,
    pub dirty: bool,
    pub disk: DiskState,
    pub expanded: bool,
    pub children: Vec<Node>,
    /// The example's index in its request's `examples`, on example nodes only.
    pub example: Option<usize>,
    /// The muted text that tells this row apart from another showing its name: a request's
    /// file name beside a same-named sibling, a collection's differing folder path.
    pub hint: Option<String>,
}

impl Node {
    /// The method tag a request row shows: the row's short label, coloured by the full method.
    pub fn method_tag(&self) -> Option<crate::ui::MethodTag> {
        let m = self.method.as_deref()?;
        Some(crate::ui::method_tag(m))
    }
}

/// The tree id of an item, `"{collection}:{path}"`; the collection row is `"{collection}:"`.
pub fn node_id(collection: usize, path: &ItemPath) -> SharedString {
    format!("{collection}:{}", path.as_str()).into()
}

/// The tree id of example `index` of the request at `path`: `"{collection}:{path}:{index}"`.
pub fn example_id(collection: usize, path: &ItemPath, index: usize) -> SharedString {
    format!("{collection}:{}:{index}", path.as_str()).into()
}

/// The tree id of a collection's `k`-th invalid row; item paths never contain `:`.
pub fn invalid_id(collection: usize, k: usize) -> SharedString {
    format!("{collection}::{k}").into()
}

/// One root node per (root, display name, content); `None` content is a collection still opening.
/// Each collection's `drafts`, in the order given, head it under an Unsaved group.
/// A non-blank filter leaves out the collections with no matching row.
pub fn build(
    collections: &[(&Path, &str, Option<&Content>)],
    drafts: &[OpenDraft],
    expanded: &Expanded,
    filter: &str,
) -> Vec<Node> {
    let filter = FilterTerms::parse(filter);
    let empty = HashSet::new();
    let shown: Vec<(&Path, &str)> = collections.iter().map(|&(r, n, _)| (r, n)).collect();
    let mut collection_hints = crate::name_hints::collection_hints(&shown).into_iter();
    collections
        .iter()
        .enumerate()
        .filter_map(|(ix, &(root, name, content))| {
            let hint = collection_hints.next().flatten();
            let ctx = Ctx {
                ix,
                root,
                content,
                expanded: expanded.get(root).unwrap_or(&empty),
                filter: filter.as_ref(),
            };
            let mut children: Vec<Node> = ctx.unsaved_group(drafts).into_iter().collect();
            if let Some(content) = content {
                children.extend(ctx.nest(&mut content.rows.iter().peekable(), 0, &[]));
                if ctx.filter.is_none() {
                    children.extend(invalid_rows(&ctx, content));
                }
            }
            let (dirty, disk) = content.map_or((false, DiskState::Present), |c| {
                (c.collection.dirty, c.collection.disk)
            });
            if let Some(content) = content {
                hint_siblings(&mut children, &sibling_hints(content));
            }
            // A filter leaves out a collection with no matching row; ids keep its open index.
            if ctx.filter.is_some() && children.is_empty() {
                return None;
            }
            let path = ItemPath::root();
            Some(Node {
                id: node_id(ix, &path),
                collection: ix,
                root: root.to_path_buf(),
                expanded: ctx.is_expanded(&path),
                path,
                kind: NodeKind::Collection,
                name: name.to_owned(),
                method: None,
                dirty,
                disk,
                children,
                example: None,
                hint,
            })
        })
        .collect()
}

/// The hints of the requests a sibling shares a name with, by path. The tree shows every other
/// row's place, so only siblings need one; counted over every row, not the filtered tree, so a
/// row's label stays the same whatever the filter shows.
fn sibling_hints(content: &Content) -> HashMap<ItemPath, String> {
    let mut by_parent: HashMap<Option<ItemPath>, Vec<&Row>> = HashMap::new();
    for row in content.rows.iter().filter(|r| r.kind == RowKind::Request) {
        by_parent.entry(row.path.parent()).or_default().push(row);
    }
    let mut out = HashMap::new();
    for rows in by_parent.values() {
        let shown: Vec<Shown> = (rows.iter())
            .map(|r| Shown {
                name: r.name.clone(),
                place: Vec::new(),
                file: Some(r.path.file_name().to_owned()),
            })
            .collect();
        for (row, hint) in rows.iter().zip(hints(&shown)) {
            if let Some(hint) = hint {
                out.insert(row.path.clone(), hint);
            }
        }
    }
    out
}

/// Gives each request row its sibling hint, if it has one.
fn hint_siblings(nodes: &mut [Node], found: &HashMap<ItemPath, String>) {
    for n in nodes {
        if n.kind == NodeKind::Request {
            n.hint = found.get(&n.path).cloned();
        }
        hint_siblings(&mut n.children, found);
    }
}

/// Mirror the nodes as gpui tree items; invalid and Empty rows are disabled.
pub fn to_tree_items(nodes: &[Node]) -> Vec<TreeItem> {
    nodes
        .iter()
        .map(|n| {
            TreeItem::new(n.id.clone(), SharedString::from(n.name.clone()))
                .expanded(n.expanded)
                .disabled(matches!(n.kind, NodeKind::Invalid | NodeKind::Empty))
                .children(to_tree_items(&n.children))
        })
        .collect()
}

/// One collection's inputs to the build.
struct Ctx<'a> {
    ix: usize,
    root: &'a Path,
    content: Option<&'a Content>,
    expanded: &'a HashSet<ItemPath>,
    filter: Option<&'a FilterTerms>,
}

impl Ctx<'_> {
    /// While filtering, everything kept is expanded; otherwise the view's set decides.
    fn is_expanded(&self, path: &ItemPath) -> bool {
        self.filter.is_some() || self.expanded.contains(path)
    }

    /// The always-expanded group of this collection's drafts, or None when no draft is kept.
    fn unsaved_group(&self, drafts: &[OpenDraft]) -> Option<Node> {
        let children: Vec<Node> = drafts
            .iter()
            .filter(|d| d.root == self.root && self.draft_matches(d))
            .map(|d| Node {
                id: node_id(self.ix, &d.path),
                collection: self.ix,
                root: self.root.to_path_buf(),
                path: d.path.clone(),
                kind: NodeKind::Draft,
                name: d.name.clone(),
                method: (!d.method.is_empty()).then(|| d.method.clone()),
                dirty: false,
                disk: DiskState::Present,
                expanded: false,
                children: Vec::new(),
                example: None,
                hint: None,
            })
            .collect();
        (!children.is_empty()).then(|| Node {
            id: unsaved_id(self.ix),
            collection: self.ix,
            root: self.root.to_path_buf(),
            path: ItemPath::root(),
            kind: NodeKind::UnsavedGroup,
            name: UNSAVED_LABEL.to_owned(),
            method: None,
            dirty: false,
            disk: DiskState::Present,
            expanded: true,
            children,
            example: None,
            hint: None,
        })
    }

    /// Whether a draft passes the filter. A draft has a method but no tag data in memory, so a
    /// `tag:` term never keeps it; name and `method:` terms apply as to any request.
    fn draft_matches(&self, d: &OpenDraft) -> bool {
        let method = (!d.method.is_empty()).then_some(d.method.as_str());
        self.filter.is_none_or(|f| f.matches(&d.name, method, None))
    }

    /// Whether a request or opaque row passes the filter, given the tags it inherits from its
    /// folders. Effective tags are its own plus every ancestor folder's, as a run computes them.
    fn row_matches(&self, row: &Row, inherited: &[String]) -> bool {
        let Some(f) = self.filter else { return true };
        let request = self.content.and_then(|c| c.requests.get(&row.path));
        let tags = request.map(|r| {
            let mut all = r.tags.clone();
            all.extend(inherited.iter().cloned());
            all
        });
        f.matches(
            &row.name,
            request.map(|r| r.method.as_str()),
            tags.as_deref(),
        )
    }

    /// Consume the depth-first rows at `depth` and below, nesting each folder's rows under it.
    /// Under a filter, leaves keep on a match and folders only with a kept child. `inherited`
    /// holds the tags of the enclosing folders, which apply to every request inside.
    fn nest<'r>(
        &self,
        rows: &mut Peekable<impl Iterator<Item = &'r Row>>,
        depth: usize,
        inherited: &[String],
    ) -> Vec<Node> {
        let mut out = Vec::new();
        while let Some(row) = rows.next_if(|r| r.depth >= depth) {
            let children = match row.kind {
                RowKind::Folder => {
                    let mut tags = inherited.to_vec();
                    if let Some(f) = self.content.and_then(|c| c.folders.get(&row.path)) {
                        tags.extend(f.tags.iter().cloned());
                    }
                    let inside = self.nest(rows, depth + 1, &tags);
                    match (inside.is_empty(), self.filter) {
                        (true, None) => vec![self.empty_row(&row.path)],
                        _ => inside,
                    }
                }
                RowKind::Request => self.examples(row),
                _ => Vec::new(),
            };
            let keep = match (self.filter, row.kind) {
                (None, _) => true,
                (Some(_), RowKind::Folder) => !children.is_empty(),
                // A request's examples never make it match; only its own data and folders' tags do.
                (Some(_), _) => self.row_matches(row, inherited),
            };
            if keep {
                out.push(self.node(row, children));
            }
        }
        out
    }

    // One leaf per saved example of the request row, in file order so indexes match the file.
    fn examples(&self, row: &Row) -> Vec<Node> {
        let Some(request) = self.content.and_then(|c| c.requests.get(&row.path)) else {
            return Vec::new();
        };
        request
            .examples
            .iter()
            .enumerate()
            .map(|(ix, e)| Node {
                id: example_id(self.ix, &row.path, ix),
                collection: self.ix,
                root: self.root.to_path_buf(),
                path: row.path.clone(),
                kind: NodeKind::Example,
                name: e.name.clone(),
                method: None,
                dirty: false,
                disk: DiskState::Present,
                expanded: false,
                children: Vec::new(),
                example: Some(ix),
                hint: None,
            })
            .collect()
    }

    // The disabled row that says the folder at `path` holds nothing; a filter never keeps one.
    fn empty_row(&self, path: &ItemPath) -> Node {
        Node {
            id: empty_id(self.ix, path),
            collection: self.ix,
            root: self.root.to_path_buf(),
            path: path.clone(),
            kind: NodeKind::Empty,
            name: EMPTY_LABEL.to_owned(),
            method: None,
            dirty: false,
            disk: DiskState::Present,
            expanded: false,
            children: Vec::new(),
            example: None,
            hint: None,
        }
    }

    fn node(&self, row: &Row, children: Vec<Node>) -> Node {
        let (kind, method) = match row.kind {
            RowKind::Folder => (NodeKind::Folder, None),
            RowKind::Opaque => (NodeKind::Opaque, None),
            RowKind::Request => {
                let request = self.content.and_then(|c| c.requests.get(&row.path));
                (NodeKind::Request, request.map(|r| r.method.clone()))
            }
        };
        Node {
            id: node_id(self.ix, &row.path),
            collection: self.ix,
            root: self.root.to_path_buf(),
            path: row.path.clone(),
            kind,
            name: row.name.clone(),
            method,
            dirty: row.dirty,
            disk: row.disk,
            expanded: match row.kind {
                RowKind::Folder => self.is_expanded(&row.path),
                // A request shows its examples by its own expansion, filter or not.
                RowKind::Request => !children.is_empty() && self.expanded.contains(&row.path),
                RowKind::Opaque => false,
            },
            children,
            example: None,
            hint: None,
        }
    }
}

/// Load errors as disabled rows, minus the opaque files core also reports as errors.
fn invalid_rows(ctx: &Ctx, content: &Content) -> Vec<Node> {
    let opaque: HashSet<&Path> = content
        .rows
        .iter()
        .filter(|r| r.kind == RowKind::Opaque)
        .map(|r| Path::new(r.path.as_str()))
        .collect();
    content
        .load_errors
        .iter()
        .filter(|e| !opaque.contains(e.path.as_path()))
        .enumerate()
        .map(|(k, e)| Node {
            id: invalid_id(ctx.ix, k),
            collection: ctx.ix,
            root: ctx.root.to_path_buf(),
            path: ItemPath::root(),
            kind: NodeKind::Invalid,
            name: format!("{}: {}", e.path.display(), e.message),
            method: None,
            dirty: false,
            disk: DiskState::Present,
            expanded: false,
            children: Vec::new(),
            example: None,
            hint: None,
        })
        .collect()
}
