//! The item tree of a collection, in the order the app's sidebar shows it.

use crate::collection::{Collection, Folder, Sibling};
use crate::item_path::ItemPath;
use crate::paging::{PageError, effective_limit};

/// What a listed item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListedKind {
    /// A folder.
    Folder,
    /// An HTTP or GraphQL request.
    Request,
    /// A request file of a type Apinata does not model, such as gRPC.
    Other,
    /// A request file that failed to load; [`ListedItem::error`] says why.
    Broken,
}

/// Why [`Collection::list_items`] refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ListItemsError {
    /// The text is not an item path.
    #[error("invalid item path: {0}")]
    InvalidPath(String),
    /// No folder sits at the path (a request file is not a folder).
    #[error("no folder at {0}")]
    NoFolder(String),
    /// A path segment is the display name of several sibling folders.
    #[error("\"{name}\" matches several folders: {}", paths.join(", "))]
    AmbiguousFolder {
        /// The segment as given.
        name: String,
        /// The root-relative paths of the folders it matches.
        paths: Vec<String>,
    },
    /// The page request was refused.
    #[error(transparent)]
    Page(#[from] PageError),
}

/// One folder, request or other item.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ListedItem {
    path: String,
    kind: ListedKind,
    name: String,
    seq: Option<u64>,
    method: Option<String>,
    url: Option<String>,
    tags: Vec<String>,
    error: Option<String>,
    children: Option<Vec<ListedItem>>,
    child_count: Option<usize>,
}

impl ListedItem {
    /// The path relative to the collection root.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// What the item is.
    pub fn kind(&self) -> ListedKind {
        self.kind
    }

    /// The display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The sort position among siblings, when set.
    pub fn seq(&self) -> Option<u64> {
        self.seq
    }

    /// The HTTP method; requests only.
    pub fn method(&self) -> Option<&str> {
        self.method.as_deref()
    }

    /// The URL as written, variables unresolved; requests only.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// The request's tags; empty for anything else.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Why the file failed to load: set on a [`ListedKind::Broken`] item, and on a folder whose
    /// `folder.yml` failed to load.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The folder's own items; `None` for anything but a folder.
    pub fn children(&self) -> Option<&[ListedItem]> {
        self.children.as_deref()
    }

    /// How many items a folder holds directly, broken files included. Set only on a folder
    /// whose children were cut off by [`ListOptions::with_depth`].
    pub fn child_count(&self) -> Option<usize> {
        self.child_count
    }
}

/// The items of one folder.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ItemList {
    items: Vec<ListedItem>,
}

impl ItemList {
    /// The folder's items: sub-folders, then requests, then other files, then broken files.
    pub fn items(&self) -> &[ListedItem] {
        &self.items
    }
}

/// Which part of a folder [`Collection::list_items_page`] returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListOptions {
    /// Levels of nesting to list below the folder; 1 is its direct items only. Unset nests in
    /// full.
    pub depth: Option<usize>,
    /// The most direct items to return; unset returns all from `offset`.
    pub limit: Option<usize>,
    /// How many direct items to skip.
    pub offset: usize,
}

impl ListOptions {
    /// Set the levels of nesting to list.
    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = Some(depth);
        self
    }

    /// Set the page size.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set how many direct items to skip.
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }
}

/// One page of a folder's direct items.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ItemPage {
    items: Vec<ListedItem>,
    total: usize,
    has_more: bool,
    limit: Option<usize>,
    offset: usize,
}

impl ItemPage {
    /// The page's items, in the order [`ItemList::items`] has.
    pub fn items(&self) -> &[ListedItem] {
        &self.items
    }

    /// How many direct items the folder holds in all.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Whether items follow this page.
    pub fn has_more(&self) -> bool {
        self.has_more
    }

    /// The page size used, after clamping; `None` when the request had no limit.
    pub fn limit(&self) -> Option<usize> {
        self.limit
    }

    /// How many direct items were skipped.
    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl Collection {
    /// The items of the folder at `path` (the root when `None`), sub-folders nested in full.
    pub fn list_items(&self, path: Option<&str>) -> Result<ItemList, ListItemsError> {
        let raw = path.unwrap_or("");
        let item_path =
            ItemPath::new(raw).ok_or_else(|| ListItemsError::InvalidPath(raw.to_owned()))?;
        let folder = self.resolve_folder(&item_path, raw)?;
        Ok(ItemList {
            items: self.listed(folder, None),
        })
    }

    /// One page of the folder at `path`: `options.depth` levels of nesting (all when unset),
    /// and of the folder's direct items the ones from `offset`, at most `limit`. An offset past
    /// the end is an empty page. Fails on a zero `limit` or `depth`; a `limit` above
    /// [`MAX_PAGE_LIMIT`](crate::MAX_PAGE_LIMIT) is clamped.
    pub fn list_items_page(
        &self,
        path: Option<&str>,
        options: &ListOptions,
    ) -> Result<ItemPage, ListItemsError> {
        let limit = effective_limit(options.limit)?;
        if options.depth == Some(0) {
            return Err(PageError::ZeroDepth.into());
        }
        let raw = path.unwrap_or("");
        let item_path =
            ItemPath::new(raw).ok_or_else(|| ListItemsError::InvalidPath(raw.to_owned()))?;
        let folder = self.resolve_folder(&item_path, raw)?;
        let all = self.listed(folder, options.depth);
        let total = all.len();
        let end = limit.map_or(total, |l| options.offset.saturating_add(l).min(total));
        let items = all
            .into_iter()
            .skip(options.offset)
            .take(end.saturating_sub(options.offset))
            .collect();
        Ok(ItemPage {
            items,
            total,
            has_more: end < total,
            limit,
            offset: options.offset,
        })
    }

    /// How many items `folder` holds directly, counted as the listing would show them.
    fn child_count(&self, folder: &Folder) -> usize {
        let siblings = folder.siblings();
        let named = siblings
            .iter()
            .filter(|s| self.item_path(s.path()).is_some())
            .count();
        named + self.broken_items(folder, &siblings).len()
    }

    /// Walk `path` one segment at a time: a directory of that exact name wins, else the one
    /// sub-folder whose display name matches. Several display matches are ambiguous.
    fn resolve_folder(&self, path: &ItemPath, raw: &str) -> Result<&Folder, ListItemsError> {
        let mut folder = self.tree();
        for segment in path.segments() {
            if let Some(sub) = folder.folders().iter().find(|f| f.dir_name() == segment) {
                folder = sub;
                continue;
            }
            let mut named = folder.folders().iter().filter(|f| f.name() == segment);
            match (named.next(), named.next()) {
                (Some(only), None) => folder = only,
                (Some(first), Some(second)) => {
                    let paths = std::iter::once(first)
                        .chain(std::iter::once(second))
                        .chain(named)
                        .filter_map(|f| self.item_path(f.path()))
                        .map(|p| p.as_str().to_owned())
                        .collect();
                    return Err(ListItemsError::AmbiguousFolder {
                        name: segment.to_owned(),
                        paths,
                    });
                }
                _ => return Err(ListItemsError::NoFolder(raw.to_owned())),
            }
        }
        Ok(folder)
    }

    /// A folder's siblings in the order core sorts them. A file whose path cannot be named as
    /// an item path (a non-UTF-8 name) is left out, since nothing could take it.
    fn listed(&self, folder: &Folder, depth: Option<usize>) -> Vec<ListedItem> {
        let siblings = folder.siblings();
        let mut items: Vec<ListedItem> = siblings
            .iter()
            .copied()
            .filter_map(|sibling| {
                let path = self.item_path(sibling.path())?.as_str().to_owned();
                let mut item = ListedItem {
                    path,
                    kind: ListedKind::Other,
                    name: sibling.name().to_owned(),
                    seq: sibling.seq(),
                    method: None,
                    url: None,
                    tags: Vec::new(),
                    error: None,
                    children: None,
                    child_count: None,
                };
                match sibling {
                    Sibling::Folder(sub) => {
                        item.kind = ListedKind::Folder;
                        item.error = self.folder_file_error(sub);
                        match depth {
                            Some(1) => item.child_count = Some(self.child_count(sub)),
                            _ => {
                                item.children = Some(self.listed(sub, depth.map(|d| d - 1)));
                            }
                        }
                    }
                    Sibling::Request(request) => {
                        item.kind = ListedKind::Request;
                        item.method = Some(request.method().to_owned());
                        item.url = Some(request.url().to_owned());
                        item.tags = request.tags();
                    }
                    _ => {}
                }
                Some(item)
            })
            .collect();
        items.extend(self.broken_items(folder, &siblings));
        items
    }

    /// The message of a failed load of this folder's own `folder.yml`, if any.
    fn folder_file_error(&self, folder: &Folder) -> Option<String> {
        let file = folder.path().join("folder.yml");
        self.load_errors()
            .iter()
            .find(|e| e.path() == file)
            .map(|e| e.message().to_owned())
    }

    /// Request files directly in `folder` that failed to load and are not already listed as an
    /// opaque sibling, named by file stem and sorted by file name.
    fn broken_items(&self, folder: &Folder, siblings: &[Sibling<'_>]) -> Vec<ListedItem> {
        let mut failed: Vec<_> = self
            .load_errors()
            .iter()
            .filter(|e| {
                let path = e.path();
                path.parent() == Some(folder.path())
                    && path.extension().is_some_and(|x| x == "yml")
                    && !matches!(
                        path.file_name().and_then(|n| n.to_str()),
                        Some("folder.yml" | "opencollection.yml")
                    )
                    && !siblings.iter().any(|s| s.path() == path)
            })
            .collect();
        failed.sort_by(|a, b| a.path().file_name().cmp(&b.path().file_name()));
        failed
            .into_iter()
            .filter_map(|e| {
                let path = self.item_path(e.path())?.as_str().to_owned();
                let name = e.path().file_stem()?.to_string_lossy().into_owned();
                Some(ListedItem {
                    path,
                    kind: ListedKind::Broken,
                    name,
                    seq: None,
                    method: None,
                    url: None,
                    tags: Vec::new(),
                    error: Some(e.message().to_owned()),
                    children: None,
                    child_count: None,
                })
            })
            .collect()
    }
}
