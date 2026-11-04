//! `list_items`: the collection tree as nested items in the app's sibling order.

use apinata_core::{Collection, ListOptions, ListedItem, ListedKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Arguments of `list_items`.
#[derive(Deserialize, JsonSchema)]
pub(crate) struct ListItemsArgs {
    /// The folder to list, relative to the collection root, each segment a directory name or a
    /// folder's display name; omit for the whole collection.
    #[serde(default)]
    path: Option<String>,
    /// Levels to list below the folder: 1 is its direct items only, with a `child_count` on each
    /// folder instead of its items. Omit to nest in full.
    #[serde(default)]
    depth: Option<usize>,
    /// The most direct items to return (1 to 500; larger is clamped). Omit for all.
    #[serde(default)]
    limit: Option<usize>,
    /// How many direct items to skip, to fetch the next page.
    #[serde(default)]
    offset: Option<usize>,
}

/// The items of one folder.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ItemList {
    items: Vec<Item>,
    /// How many direct items the folder holds in all; present when `depth`, `limit` or `offset`
    /// was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    /// More direct items follow this page: call again with a larger `offset`.
    #[serde(skip_serializing_if = "Option::is_none")]
    has_more: Option<bool>,
    /// The page size used, after clamping to 500; present when `limit` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
}

/// What an item is.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ItemKind {
    /// A folder.
    Folder,
    /// An HTTP or GraphQL request.
    Request,
    /// A request file of a type Apinata does not model, such as gRPC.
    Other,
    /// A request file that failed to load; `error` says why.
    Broken,
}

/// One folder, request or other item.
#[derive(Serialize, JsonSchema)]
pub(crate) struct Item {
    /// The item's path relative to the collection root: the `path` argument other tools take.
    path: String,
    kind: ItemKind,
    /// The display name.
    name: String,
    /// The sort position among siblings, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    /// The HTTP method; requests only.
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    /// The URL as written, variables unresolved; requests only.
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    /// Why the file failed to load; broken items, and folders whose `folder.yml` failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// How many items the folder holds directly; set on a folder whose `items` the `depth` cut.
    #[serde(skip_serializing_if = "Option::is_none")]
    child_count: Option<usize>,
    /// The folder's own items; folders only.
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<Vec<Item>>,
}

impl ListItemsArgs {
    pub(crate) fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Whether any paging argument was given.
    fn paged(&self) -> bool {
        self.depth.is_some() || self.limit.is_some() || self.offset.is_some()
    }
}

/// The items of the folder named by the arguments. With no paging argument the whole nested
/// listing comes back; otherwise one page with `total` and `has_more`.
pub(crate) fn list(collection: &Collection, args: &ListItemsArgs) -> Result<ItemList, String> {
    if !args.paged() {
        let list = collection
            .list_items(args.path())
            .map_err(|e| e.to_string())?;
        return Ok(ItemList {
            items: list.items().iter().map(item).collect(),
            total: None,
            has_more: None,
            limit: None,
        });
    }
    let mut options = ListOptions::default().with_offset(args.offset.unwrap_or(0));
    if let Some(depth) = args.depth {
        options = options.with_depth(depth);
    }
    if let Some(limit) = args.limit {
        options = options.with_limit(limit);
    }
    let page = collection
        .list_items_page(args.path(), &options)
        .map_err(|e| e.to_string())?;
    Ok(ItemList {
        items: page.items().iter().map(item).collect(),
        total: Some(page.total()),
        has_more: Some(page.has_more()),
        limit: page.limit(),
    })
}

fn item(listed: &ListedItem) -> Item {
    Item {
        path: listed.path().to_owned(),
        kind: match listed.kind() {
            ListedKind::Folder => ItemKind::Folder,
            ListedKind::Request => ItemKind::Request,
            ListedKind::Broken => ItemKind::Broken,
            _ => ItemKind::Other,
        },
        name: listed.name().to_owned(),
        seq: listed.seq(),
        method: listed.method().map(str::to_owned),
        url: listed.url().map(str::to_owned),
        error: listed.error().map(str::to_owned),
        child_count: listed.child_count(),
        items: listed.children().map(|c| c.iter().map(item).collect()),
    }
}
