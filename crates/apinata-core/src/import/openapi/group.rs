//! Placing imported requests into folders, by first tag or by path segment.

use super::Doc;
use super::operations::{Operation, dedupe};
use crate::edit::AuthSpec;
use crate::import::{
    DocumentError, GroupBy, ImportNote, ImportedFolder, ImportedItem, ImportedRequestItem, NoteKind,
};
use serde_json::Value;
use std::collections::HashMap;

/// The deepest folder nesting a path makes; deeper segments share the last folder's name.
const MAX_DEPTH: usize = 32;

type Pair<'a> = (Operation<'a>, ImportedRequestItem);

/// A folder being filled: its requests and subfolders, each in order of first use.
struct Group<'a> {
    name: String,
    docs: String,
    requests: Vec<Pair<'a>>,
    children: Vec<Group<'a>>,
    index: HashMap<String, usize>,
}

impl<'a> Group<'a> {
    fn new(name: String, docs: String) -> Group<'a> {
        Group {
            name,
            docs,
            requests: Vec::new(),
            children: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// The child named `name`, created and charged to the budget on first use.
    fn child(
        &mut self,
        doc: &mut Doc<'_>,
        name: &str,
        docs: &str,
    ) -> Result<&mut Group<'a>, DocumentError> {
        let i = match self.index.get(name) {
            Some(&i) => i,
            None => {
                // The name is copied twice, into the folder and into the index.
                doc.budget.charge(1)?;
                doc.budget
                    .charge_bytes(2 * name.len() as u64 + docs.len() as u64)?;
                self.index.insert(name.to_owned(), self.children.len());
                self.children
                    .push(Group::new(name.to_owned(), docs.to_owned()));
                self.children.len() - 1
            }
        };
        Ok(&mut self.children[i])
    }
}

/// The collection's top-level items: `requests` placed by `by`. Each sibling list of
/// requests is deduplicated and every sibling list numbered from 1. Each folder is charged
/// to the budget; a path cut at `MAX_DEPTH` folders gets a `Converted` note in `unwritten`.
pub(crate) fn build(
    doc: &mut Doc<'_>,
    requests: Vec<(Operation<'_>, ImportedRequestItem)>,
    by: GroupBy,
    unwritten: &mut Vec<ImportNote>,
) -> Result<Vec<ImportedItem>, DocumentError> {
    let mut root = Group::new(String::new(), String::new());
    if by == GroupBy::Path {
        for pair in requests {
            place_by_path(doc, &mut root, pair, unwritten)?;
        }
        return finish(doc, root.requests, root.children, true);
    }
    let docs = tag_docs(doc.root);
    let mut untagged = Vec::new();
    for pair in requests {
        match first_tag(pair.0.op()) {
            Some(tag) => {
                let docs = docs.get(tag).copied().unwrap_or_default();
                root.child(doc, tag, docs)?.requests.push(pair);
            }
            None => untagged.push(pair),
        }
    }
    finish(doc, untagged, root.children, false)
}

/// Walk the path's segments in a loop, so a path of thousands of segments never recurses,
/// and stop nesting at `MAX_DEPTH` with the rest joined into the last folder's name.
fn place_by_path<'a>(
    doc: &mut Doc<'_>,
    root: &mut Group<'a>,
    pair: Pair<'a>,
    unwritten: &mut Vec<ImportNote>,
) -> Result<(), DocumentError> {
    let segments: Vec<&str> = pair.0.path.split('/').filter(|s| !s.is_empty()).collect();
    let mut node = root;
    if segments.is_empty() {
        node = node.child(doc, "Root", "")?;
    }
    for (depth, segment) in segments.iter().enumerate() {
        if depth + 1 == MAX_DEPTH && segments.len() > MAX_DEPTH {
            let joined = segments[depth..].join("/");
            let note = ImportNote::new(
                format!("paths.{}", pair.0.path),
                NoteKind::Converted,
                format!(
                    "the path nests deeper than {MAX_DEPTH} folders; its remaining segments \
                     name the {MAX_DEPTH}nd folder"
                ),
            );
            doc.budget.charge_note(&note)?;
            unwritten.push(note);
            doc.budget.charge_text(&joined)?;
            node = node.child(doc, &joined, "")?;
            break;
        }
        node = node.child(doc, segment, "")?;
    }
    node.requests.push(pair);
    Ok(())
}

/// One sibling list: the requests deduplicated, then the folders (or the folders first),
/// numbered from 1.
fn finish(
    doc: &mut Doc<'_>,
    mut requests: Vec<Pair<'_>>,
    folders: Vec<Group<'_>>,
    requests_first: bool,
) -> Result<Vec<ImportedItem>, DocumentError> {
    dedupe(doc, &mut requests)?;
    let requests = requests
        .into_iter()
        .map(|(_, request)| ImportedItem::Request(request));
    let mut folder_items = Vec::with_capacity(folders.len());
    for group in folders {
        let items = finish(doc, group.requests, group.children, true)?;
        folder_items.push(ImportedItem::Folder(ImportedFolder {
            scripts: Default::default(),
            name: group.name,
            seq: 0,
            docs: group.docs,
            auth: AuthSpec::Inherit,
            headers: Vec::new(),
            vars: Vec::new(),
            items,
            notes: Vec::new(),
        }));
    }
    let mut items: Vec<ImportedItem> = if requests_first {
        requests.chain(folder_items).collect()
    } else {
        folder_items.into_iter().chain(requests).collect()
    };
    for (seq, item) in (1..).zip(&mut items) {
        match item {
            ImportedItem::Folder(f) => f.seq = seq,
            ImportedItem::Request(r) => r.seq = seq,
        }
    }
    Ok(items)
}

/// The operation's first tag, trimmed, when it is a non-empty string.
fn first_tag(op: &Value) -> Option<&str> {
    op.get("tags")?
        .as_array()?
        .first()?
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Each root `tags[]` name with its description; a later entry of the same name wins.
fn tag_docs(root: &Value) -> HashMap<&str, &str> {
    root.get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| {
            let name = tag.get("name")?.as_str()?.trim();
            let docs = tag.get("description")?.as_str()?;
            (!name.is_empty() && !docs.is_empty()).then_some((name, docs))
        })
        .collect()
}
