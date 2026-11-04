//! `apinata ls`: the folder tree as text or JSON, in the order the MCP `list_items` tool uses.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apinata_core::{Collection, ListOptions, ListedItem, ListedKind};
use serde::Serialize;

use crate::trust::{self, shown};
use crate::{EXIT_USAGE, LsArgs, collection_arg};

/// One item as JSON; the fields of the MCP `list_items` item, plus `tags` on request with --tags.
#[derive(Serialize)]
struct Item {
    path: String,
    kind: &'static str,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    child_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<Vec<Item>>,
}

/// The listing; the paging fields appear only when a paging flag was given.
#[derive(Serialize)]
struct ItemList {
    items: Vec<Item>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_more: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    #[serde(skip)]
    offset: usize,
}

/// Print the items under `args.path`: 0 when listed, 3 for an unopenable collection or a path
/// that is not a folder.
pub fn run(args: &LsArgs) -> ExitCode {
    match list(args) {
        Ok(list) => {
            if args.json {
                match serde_json::to_string_pretty(&list) {
                    Ok(json) => outln!("{json}"),
                    Err(e) => {
                        eprintln!("error: {e}");
                        return ExitCode::from(EXIT_USAGE);
                    }
                }
            } else {
                out!("{}", text(&list.items, 0));
                if let Some(total) = list.total {
                    out!("{}", range_line(list.offset, list.items.len(), total));
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn list(args: &LsArgs) -> anyhow::Result<ItemList> {
    let (named, folder) = split_arguments(args)?;
    let root: PathBuf = collection_arg::with_hint(
        trust::directory_root(named.as_deref()),
        &collection_arg::optional_hint("apinata ls [COLLECTION] [PATH]"),
    )?;
    let collection = Collection::open(&root)?;
    if args.depth.is_none() && args.limit.is_none() && args.offset.is_none() {
        let listed = collection.list_items(folder.as_deref())?;
        return Ok(ItemList {
            items: listed.items().iter().map(|i| item(i, args.tags)).collect(),
            total: None,
            has_more: None,
            limit: None,
            offset: 0,
        });
    }
    let mut options = ListOptions::default().with_offset(args.offset.unwrap_or(0));
    if let Some(depth) = args.depth {
        options = options.with_depth(depth);
    }
    if let Some(limit) = args.limit {
        options = options.with_limit(limit);
    }
    let page = collection.list_items_page(folder.as_deref(), &options)?;
    Ok(ItemList {
        items: page.items().iter().map(|i| item(i, args.tags)).collect(),
        total: Some(page.total()),
        has_more: Some(page.has_more()),
        limit: page.limit(),
        offset: page.offset(),
    })
}

/// The closing line of a paged listing: which items were shown, or that the offset is past them.
fn range_line(offset: usize, shown: usize, total: usize) -> String {
    if shown == 0 {
        format!("(no items at offset {offset} of {total})\n")
    } else {
        format!("(showing {}-{} of {total})\n", offset + 1, offset + shown)
    }
}

/// The collection and the folder the arguments name. Two positionals are collection then folder.
/// A lone one stays the folder (today's meaning) unless `--collection` is absent, it is a
/// directory holding opencollection.yml, and the working directory's collection has no such folder.
fn split_arguments(args: &LsArgs) -> anyhow::Result<(Option<PathBuf>, Option<String>)> {
    let flag = args.collection.as_deref();
    match (&args.first, &args.second) {
        (Some(collection), Some(folder)) => {
            let named = collection_arg::pick(Some(Path::new(collection)), flag)?;
            Ok((named, Some(folder.clone())))
        }
        (Some(lone), None) => {
            if flag.is_none() && is_collection_dir(lone) && !cwd_has_folder(lone) {
                Ok((Some(PathBuf::from(lone)), None))
            } else {
                Ok((flag.map(Path::to_path_buf), Some(lone.clone())))
            }
        }
        _ => Ok((flag.map(Path::to_path_buf), None)),
    }
}

fn is_collection_dir(path: &str) -> bool {
    Path::new(path).join("opencollection.yml").is_file()
}

/// Whether the working directory's collection, if any, lists `folder` as one of its own.
fn cwd_has_folder(folder: &str) -> bool {
    trust::collection_root(None)
        .ok()
        .and_then(|root| Collection::open(&root).ok())
        .is_some_and(|c| c.list_items(Some(folder)).is_ok())
}

/// The JSON form of a listed item; tags are carried only when asked for.
fn item(listed: &ListedItem, tags: bool) -> Item {
    Item {
        path: listed.path().to_owned(),
        kind: match listed.kind() {
            ListedKind::Folder => "folder",
            ListedKind::Request => "request",
            ListedKind::Broken => "broken",
            _ => "other",
        },
        name: listed.name().to_owned(),
        seq: listed.seq(),
        method: listed.method().map(str::to_owned),
        url: listed.url().map(str::to_owned),
        tags: (tags && listed.kind() == ListedKind::Request).then(|| listed.tags().to_vec()),
        error: listed.error().map(str::to_owned),
        child_count: listed.child_count(),
        items: listed
            .children()
            .map(|c| c.iter().map(|i| item(i, tags)).collect()),
    }
}

/// Folders as `name/` (`name/  path` when the directory name differs from the name, plus
/// `  reason` when its folder.yml failed to load), requests as `METHOD  name  path` (the path is
/// what `apinata run` takes), other files as `other  name  path`, files that fail to load as
/// `broken  name  path  reason`; children are indented two spaces. Every collection-supplied
/// string goes through `shown`, so a name cannot drive the terminal or split its line.
fn text(items: &[Item], depth: usize) -> String {
    let mut out = String::new();
    let pad = "  ".repeat(depth);
    for item in items {
        match item.kind {
            "folder" => {
                // A renamed folder also shows its directory path, which is what other commands take.
                let mut line = format!("{pad}{}/", shown(&item.name));
                if item.path.rsplit('/').next() != Some(item.name.as_str()) {
                    line.push_str(&format!("  {}", shown(&item.path)));
                }
                if let Some(reason) = &item.error {
                    line.push_str(&format!("  {}", shown(reason)));
                }
                if let Some(count) = item.child_count {
                    line.push_str(&format!("  ({count} items)"));
                }
                out.push_str(&line);
                out.push('\n');
                out.push_str(&text(item.items.as_deref().unwrap_or(&[]), depth + 1));
            }
            kind => {
                let label = item.method.as_deref().unwrap_or(kind);
                out.push_str(&format!(
                    "{pad}{}  {}  {}",
                    shown(label),
                    shown(&item.name),
                    shown(&item.path)
                ));
                if let Some(t) = item.tags.as_ref().filter(|t| !t.is_empty()) {
                    let t: Vec<String> = t.iter().map(|t| shown(t)).collect();
                    out.push_str(&format!("  [{}]", t.join(", ")));
                }
                if item.kind == "broken" {
                    out.push_str(&format!("  {}", shown(item.error.as_deref().unwrap_or(""))));
                }
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broken(error: &str) -> Item {
        Item {
            path: "a.yml".into(),
            kind: "broken",
            name: "a".into(),
            seq: None,
            method: None,
            url: None,
            tags: None,
            error: Some(error.into()),
            child_count: None,
            items: None,
        }
    }

    #[test]
    fn a_reason_cannot_split_its_line_or_drive_the_terminal() {
        let out = text(&[broken("bad\nline\u{1b}[31m")], 0);
        assert_eq!(out.lines().count(), 1, "{out:?}");
        assert!(!out.contains('\u{1b}'), "{out:?}");
    }
}
