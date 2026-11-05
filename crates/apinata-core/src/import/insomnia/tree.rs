//! Building the item tree: v4's flat `resources` linked by `parentId`, v5's nested `children`,
//! sibling order, item kinds and the folder depth cap.

use super::env::{self, EnvSource};
use super::model::{
    FolderSource, Item, RequestSource, Resource, name_or, rest_paths, sort_key, text,
};
use super::{Budget, Crumbs, request};
use crate::import::{
    DocumentError, ImportNote, ImportedFolder, ImportedItem, ImportedRequestItem, NoteKind,
};
use serde_json::Value;
use std::collections::HashMap;

/// Folders nest at most this deep; a deeper one is unwritten with its contents.
const MAX_FOLDER_DEPTH: usize = 128;

/// v4 resource types that have no counterpart and are dropped wherever they sit.
const DROPPED_KINDS: [&str; 11] = [
    "websocket_payload",
    "socketio_payload",
    "proto_file",
    "proto_directory",
    "unit_test_suite",
    "unit_test",
    "api_spec",
    "mock",
    "mock_route",
    "runner_result",
    "unknown",
];

/// One sibling before sorting.
pub(super) struct Node {
    sort: f64,
    id: String,
    kind: Kind,
}

enum Kind {
    Request(Box<RequestSource>),
    Folder(Box<FolderSource>, Vec<Node>),
    /// An item that is not written; `detail` says why.
    Unwritten {
        name: String,
        detail: String,
    },
}

/// One v4 workspace with its tree and the notes about its own resources.
pub(super) struct Workspace {
    pub(super) name: String,
    pub(super) docs: String,
    pub(super) nodes: Vec<Node>,
    pub(super) notes: Vec<ImportNote>,
    /// Each base environment with its subs, in resource order.
    pub(super) environments: Vec<(EnvSource, Vec<EnvSource>)>,
}

/// Why an item of an HTTP-less kind is not written, or `None` for a kind that is.
fn unwritten_detail(v4_kind: &str) -> Option<&'static str> {
    Some(match v4_kind {
        "grpc_request" => "gRPC requests are not imported",
        "websocket_request" => "WebSocket requests are not imported",
        "socketio_request" => "Socket.IO requests are not imported",
        "mcp_request" => "MCP requests are not imported",
        _ => return None,
    })
}

fn cut_detail() -> String {
    format!(
        "the folder nests deeper than {MAX_FOLDER_DEPTH} folders; it and its contents are not imported"
    )
}

/// The v4 resources as one tree per workspace, in resource order, plus a note for every
/// resource no workspace reaches. Each resource is visited at most once, so duplicate ids,
/// shared parents and `parentId` cycles cannot attach anything twice.
pub(super) fn v4(
    resources: Vec<Value>,
    budget: &mut Budget,
) -> Result<(Vec<Workspace>, Vec<ImportNote>), DocumentError> {
    let mut walk = Walk::new(resources);
    let roots: Vec<usize> = (0..walk.kinds.len())
        .filter(|&i| walk.kinds[i] == "workspace")
        .collect();
    if roots.is_empty() {
        return Err(DocumentError::Invalid {
            message: "the export holds no workspace".into(),
        });
    }
    let mut workspaces = Vec::with_capacity(roots.len());
    for i in roots {
        walk.visited[i] = true;
        let resource = walk.resources[i].take().unwrap_or_default();
        let mut notes = Vec::new();
        let mut unread = Vec::new();
        // Only a workspace's `scope` is export metadata.
        rest_paths(&resource.rest, "collection / ", &["scope"], &mut unread);
        budget.unread(&[], unread, &mut notes)?;
        let id = walk.ids[i].clone();
        let nodes = walk.children_of(&id, &mut notes, budget)?;
        workspaces.push(Workspace {
            name: name_or(resource.name.as_ref(), "Insomnia collection"),
            docs: text(resource.description.as_ref()),
            nodes,
            notes,
            environments: std::mem::take(&mut walk.environments),
        });
    }
    let mut stray = Vec::new();
    for i in 0..walk.kinds.len() {
        if walk.visited[i] {
            continue;
        }
        let kind = walk.kinds[i].as_str();
        let detail = if kind == "cookie_jar" {
            if !holds_cookies(walk.resources[i].as_ref()) {
                continue;
            }
            "cookies are not imported".to_owned()
        } else if DROPPED_KINDS.contains(&kind) {
            format!("Insomnia `{kind}` resources are not imported")
        } else if matches!(kind, "request" | "request_group" | "environment")
            || unwritten_detail(kind).is_some()
        {
            "not reachable from a workspace through `parentId`; not imported".to_owned()
        } else {
            format!("`{kind}` is not an Insomnia resource type that can be imported")
        };
        stray.push(budget.note(&[], &format!("resources[{i}]"), NoteKind::Dropped, detail)?);
    }
    Ok((workspaces, stray))
}

fn env_source(mut r: Resource, id: String, sort: f64, default: &str) -> EnvSource {
    let data = r.data.take();
    let mut source = EnvSource::new(r.name.as_ref(), default, id, sort, data);
    rest_paths(&r.rest, "", &[], &mut source.unread);
    source
}

fn holds_cookies(resource: Option<&Resource>) -> bool {
    resource
        .and_then(|r| r.cookies.as_ref())
        .and_then(Value::as_array)
        .is_some_and(|c| !c.is_empty())
}

/// The v4 resources with their ids, types and children grouped once by `parentId`.
struct Walk {
    resources: Vec<Option<Resource>>,
    ids: Vec<String>,
    kinds: Vec<String>,
    /// Children by parent id; a list is removed when taken, so it is attached at most once.
    children: HashMap<String, Vec<usize>>,
    visited: Vec<bool>,
    /// The environments claimed under the workspace being walked.
    environments: Vec<(EnvSource, Vec<EnvSource>)>,
}

impl Walk {
    fn new(values: Vec<Value>) -> Walk {
        // A resource that is not an object reads as one without a type, noted as unknown.
        let resources: Vec<Option<Resource>> = values
            .into_iter()
            .map(|v| Some(serde_json::from_value(v).unwrap_or_default()))
            .collect();
        let field =
            |r: &Option<Resource>, f: fn(&Resource) -> Option<&Value>| text(r.as_ref().and_then(f));
        let ids: Vec<String> = resources
            .iter()
            .map(|r| field(r, |r| r.id.as_ref()))
            .collect();
        let kinds: Vec<String> = resources
            .iter()
            .map(|r| field(r, |r| r.kind.as_ref()))
            .collect();
        let mut children: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in resources.iter().enumerate() {
            let parent = r.as_ref().and_then(|r| r.parent_id.as_ref());
            if kinds[i] != "workspace" && parent.is_some_and(|p| !p.is_null()) {
                children.entry(text(parent)).or_default().push(i);
            }
        }
        let visited = vec![false; resources.len()];
        Walk {
            resources,
            ids,
            kinds,
            children,
            visited,
            environments: Vec::new(),
        }
    }

    /// The written and unwritten items below the workspace `root`. The walk keeps its own stack
    /// of open folders, so nesting never grows the call stack; a folder past the cap is cut.
    fn children_of(
        &mut self,
        root: &str,
        notes: &mut Vec<ImportNote>,
        budget: &mut Budget,
    ) -> Result<Vec<Node>, DocumentError> {
        let mut stack = vec![Open::new(self.take_children(root), None)];
        while let Some(top) = stack.last_mut() {
            let Some(i) = top.list.next() else {
                let Some(done) = stack.pop() else { break };
                match stack.last_mut() {
                    Some(parent) => parent.close(done),
                    None => return Ok(done.nodes),
                }
                continue;
            };
            let depth = stack.len() - 1;
            let sort = self.resources[i]
                .as_ref()
                .map_or(0.0, |r| sort_key(r.sort_key.as_ref()));
            let id = self.ids[i].clone();
            let kind = match self.kinds[i].as_str() {
                "request" => Kind::Request(Box::new(RequestSource::from_v4(self.take(i)))),
                "request_group" if depth == MAX_FOLDER_DEPTH => {
                    let r = self.take(i);
                    self.silence(&id);
                    Kind::Unwritten {
                        name: name_or(r.name.as_ref(), "New Folder"),
                        detail: cut_detail(),
                    }
                }
                "request_group" => {
                    let source = Box::new(FolderSource::from_v4(self.take(i)));
                    let list = self.take_children(&id);
                    stack.push(Open::new(list, Some((sort, id, source))));
                    continue;
                }
                "environment" if depth == 0 => {
                    self.claim_environment(i, id, sort);
                    continue;
                }
                "cookie_jar" if depth == 0 => {
                    let r = self.take(i);
                    if holds_cookies(Some(&r)) {
                        let source = format!("resources[{i}]");
                        let detail = "cookies are not imported";
                        notes.push(budget.note(&[], &source, NoteKind::Dropped, detail)?);
                    }
                    continue;
                }
                other if DROPPED_KINDS.contains(&other) => {
                    let detail = format!("Insomnia `{other}` resources are not imported");
                    self.take(i);
                    let source = format!("resources[{i}]");
                    notes.push(budget.note(&[], &source, NoteKind::Dropped, detail)?);
                    continue;
                }
                other => match unwritten_detail(other) {
                    Some(detail) => Kind::Unwritten {
                        name: name_or(self.take(i).name.as_ref(), "New Request"),
                        detail: detail.to_owned(),
                    },
                    // Left unvisited, so the stray pass notes it by index.
                    None => continue,
                },
            };
            push_node(&mut stack, Node { sort, id, kind });
        }
        Ok(Vec::new())
    }

    /// Mark resource `i` visited and move it out.
    fn take(&mut self, i: usize) -> Resource {
        self.visited[i] = true;
        self.resources[i].take().unwrap_or_default()
    }

    /// The children of `parent`, removed so they are attached at most once.
    fn take_children(&mut self, parent: &str) -> std::vec::IntoIter<usize> {
        self.children.remove(parent).unwrap_or_default().into_iter()
    }

    /// Claim the base environment at `i` and its sub environments for the workspace being
    /// walked; other children of the base stay unvisited and are noted as strays.
    fn claim_environment(&mut self, i: usize, id: String, sort: f64) {
        self.visited[i] = true;
        let r = self.resources[i].take().unwrap_or_default();
        let subs_of = self.children.remove(&id).unwrap_or_default();
        let base = env_source(r, id, sort, "Base Environment");
        let mut subs = Vec::new();
        for sub in subs_of {
            if self.kinds[sub] == "environment" {
                self.visited[sub] = true;
                let r = self.resources[sub].take().unwrap_or_default();
                let sort = sort_key(r.sort_key.as_ref());
                subs.push(env_source(
                    r,
                    self.ids[sub].clone(),
                    sort,
                    "New Environment",
                ));
            }
        }
        self.environments.push((base, subs));
    }

    /// Mark everything below a cut folder as handled, so the folder's one note covers it.
    fn silence(&mut self, id: &str) {
        let mut stack = vec![id.to_owned()];
        while let Some(parent) = stack.pop() {
            for i in self.children.remove(&parent).unwrap_or_default() {
                self.visited[i] = true;
                stack.push(self.ids[i].clone());
            }
        }
    }
}

/// A folder whose children are still being walked: the rest of its list, the nodes built so
/// far, and the folder itself (`None` for the root list).
struct Open<L> {
    list: L,
    nodes: Vec<Node>,
    folder: Option<(f64, String, Box<FolderSource>)>,
}

impl<L> Open<L> {
    fn new(list: L, folder: Option<(f64, String, Box<FolderSource>)>) -> Open<L> {
        Open {
            list,
            nodes: Vec::new(),
            folder,
        }
    }

    /// Attach the finished child folder `done` to this one.
    fn close(&mut self, done: Open<L>) {
        if let Some((sort, id, source)) = done.folder {
            let kind = Kind::Folder(source, done.nodes);
            self.nodes.push(Node { sort, id, kind });
        }
    }
}

/// Add `node` to the innermost open folder.
fn push_node<L>(stack: &mut [Open<L>], node: Node) {
    if let Some(top) = stack.last_mut() {
        top.nodes.push(node);
    }
}

/// The v5 items of the `collection` list. Folders are walked with an explicit stack, so
/// nesting never grows the call stack; item paths read `collection[2].children[0]`.
pub(super) fn v5(
    list: Vec<Value>,
    unwritten: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Vec<Node>, DocumentError> {
    let root = ("collection".to_owned(), list.into_iter().enumerate());
    let mut stack = vec![Open::new(root, None)];
    while let Some(top) = stack.last_mut() {
        let Some((i, value)) = top.list.1.next() else {
            let Some(done) = stack.pop() else { break };
            match stack.last_mut() {
                Some(parent) => parent.close(done),
                None => return Ok(done.nodes),
            }
            continue;
        };
        let here = format!("{}[{i}]", top.list.0);
        let depth = stack.len() - 1;
        let Ok(item) = serde_json::from_value::<Item>(value) else {
            let detail = "the item is not an object; not imported";
            unwritten.push(budget.note(&[], &here, NoteKind::Dropped, detail)?);
            continue;
        };
        let meta = item.meta.as_ref();
        let id = text(meta.and_then(|m| m.get("id")));
        let sort = sort_key(meta.and_then(|m| m.get("sortKey")));
        let kind = if item.method.is_some() {
            Kind::Request(Box::new(RequestSource::from_v5(item)))
        } else if item.reflection_api.is_some() {
            Kind::Unwritten {
                name: name_or(item.name.as_ref(), "New Request"),
                detail: "gRPC requests are not imported".into(),
            }
        } else if item.children.is_some() || item.url.is_none() {
            let name = name_or(item.name.as_ref(), "New Folder");
            if depth == MAX_FOLDER_DEPTH {
                Kind::Unwritten {
                    name,
                    detail: cut_detail(),
                }
            } else {
                let (source, list) = FolderSource::from_v5(item, name);
                let source = Box::new(source);
                let list = (format!("{here}.children"), list.into_iter().enumerate());
                stack.push(Open::new(list, Some((sort, id, source))));
                continue;
            }
        } else {
            let detail = if id.starts_with("socketio-req") {
                "Socket.IO requests are not imported"
            } else {
                "WebSocket requests are not imported"
            };
            Kind::Unwritten {
                name: name_or(item.name.as_ref(), "New Request"),
                detail: detail.into(),
            }
        };
        push_node(&mut stack, Node { sort, id, kind });
    }
    Ok(Vec::new())
}

/// A folder whose items are still being built: its remaining sorted children, the items
/// built so far, and the folder's own parts (`None` for the collection root).
struct Building {
    nodes: std::vec::IntoIter<Node>,
    out: Vec<ImportedItem>,
    folder: Option<ImportedFolder>,
}

impl Building {
    /// Siblings sort by sort key, ties by id descending.
    fn new(mut nodes: Vec<Node>, folder: Option<ImportedFolder>) -> Building {
        nodes.sort_by(|a, b| a.sort.total_cmp(&b.sort).then_with(|| b.id.cmp(&a.id)));
        Building {
            nodes: nodes.into_iter(),
            out: Vec::new(),
            folder,
        }
    }
}

/// Sort siblings and turn them into items numbered from 1; an unwritten one becomes a note in
/// `unwritten` sourced by its breadcrumb under `trail`. Folders are built with an explicit
/// stack, and each open folder's name stays on `trail` while its items are built.
pub(super) fn items(
    nodes: Vec<Node>,
    trail: &mut Vec<String>,
    unwritten: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Vec<ImportedItem>, DocumentError> {
    let mut stack = vec![Building::new(nodes, None)];
    while let Some(top) = stack.last_mut() {
        let seq = top.out.len() as u64 + 1;
        let Some(node) = top.nodes.next() else {
            let Some(done) = stack.pop() else { break };
            let Some(mut folder) = done.folder else {
                return Ok(done.out);
            };
            folder.items = done.out;
            folder.name = trail.pop().unwrap_or_default();
            if let Some(parent) = stack.last_mut() {
                parent.out.push(ImportedItem::Folder(folder));
            }
            continue;
        };
        match node.kind {
            Kind::Request(mut source) => {
                // The name joins the trail so the request's notes are sourced under it.
                budget.entry()?;
                trail.push(std::mem::take(&mut source.name));
                let mut notes = Vec::new();
                let request = request::request(*source, trail, &mut notes, budget)?;
                let name = trail.pop().unwrap_or_default();
                top.out.push(ImportedItem::Request(ImportedRequestItem {
                    name,
                    seq,
                    request,
                    notes,
                }));
            }
            Kind::Folder(mut source, children) => {
                budget.entry()?;
                trail.push(std::mem::take(&mut source.name));
                let mut notes = Vec::new();
                let environment = source.environment.take();
                let (docs, headers, auth) =
                    request::folder_parts(*source, trail, &mut notes, budget)?;
                // Variables stay on the folder; descendants inherit them without a copy.
                let vars = env::folder_vars(environment, trail, &mut notes, budget)?;
                let folder = ImportedFolder {
                    scripts: Default::default(),
                    name: String::new(),
                    seq,
                    docs,
                    auth,
                    headers,
                    vars,
                    items: Vec::new(),
                    notes,
                };
                stack.push(Building::new(children, Some(folder)));
            }
            Kind::Unwritten { name, detail } => {
                unwritten.push(budget.note(trail, &name, NoteKind::Dropped, detail)?);
            }
        }
    }
    Ok(Vec::new())
}
