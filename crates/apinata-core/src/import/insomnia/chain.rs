//! Response tags that read another request's response, as post-response variables the source
//! request sets and the reading value refers to.

use crate::PostResponseVar;
use crate::import::ImportedItem;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Every chained name starts with this, so only input keys with it can collide.
const PREFIX: &str = "res_";

/// A slug part longer than this is cut, so a long request name stays a usable variable name.
const SLUG_CHARS: usize = 32;

/// The requests a response tag may read from, the variables linked so far, and where each
/// request was built in the item tree.
#[derive(Default)]
pub(super) struct Chains {
    /// Request id to name, for the collection being built.
    sources: HashMap<String, String>,
    /// Names already used by the input or by an earlier link.
    taken: HashSet<String>,
    links: Vec<Link>,
    /// `(source id, expression)` to its index in `links`.
    by_read: HashMap<(String, String), usize>,
    /// Request id to its index path through the built items.
    placed: HashMap<String, Vec<usize>>,
}

/// One post-response variable to set on a source request.
pub(super) struct Link {
    pub(super) source: String,
    pub(super) name: String,
    pub(super) expression: String,
}

impl Chains {
    /// Chains among `sources` (request id to name), avoiding the `taken` names.
    pub(super) fn new(sources: HashMap<String, String>, taken: HashSet<String>) -> Chains {
        Chains {
            sources,
            taken,
            ..Chains::default()
        }
    }

    #[cfg(test)]
    pub(super) fn links(&self) -> &[Link] {
        &self.links
    }

    /// The variable holding `expression` of request `id`'s response, named after the source
    /// and `slug`, with the source's name; the same read always gets the same variable. `Err`
    /// says why when the source is not in this collection.
    pub(super) fn link(
        &mut self,
        id: &str,
        expression: String,
        slug: &str,
    ) -> Result<(String, String), String> {
        let Some(source) = self.sources.get(id).cloned() else {
            return Err("the request it reads from is not in this collection".to_owned());
        };
        let key = (id.to_owned(), expression);
        if let Some(&i) = self.by_read.get(&key) {
            return Ok((self.links[i].name.clone(), source));
        }
        let base = format!("{PREFIX}{}_{}", slugify(&source), slugify(slug));
        let mut name = base.clone();
        let mut n = 2;
        while self.taken.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        self.taken.insert(name.clone());
        self.by_read.insert(key.clone(), self.links.len());
        self.links.push(Link {
            source: key.0,
            name: name.clone(),
            expression: key.1,
        });
        Ok((name, source))
    }

    /// Record that request `id` was built at `path` through the item tree.
    pub(super) fn place(&mut self, id: &str, path: Vec<usize>) {
        if self.sources.contains_key(id) {
            self.placed.insert(id.to_owned(), path);
        }
    }

    /// Give each linked source request its post-response variables, then start over for the
    /// next collection.
    pub(super) fn attach(&mut self, items: &mut [ImportedItem]) {
        let chains = std::mem::take(self);
        for link in chains.links {
            let Some(path) = chains.placed.get(&link.source) else {
                continue;
            };
            if let Some(request) = at(items, path) {
                let vars = &mut request.request.post_response_vars;
                vars.push(PostResponseVar::new(link.name, link.expression));
            }
        }
    }
}

/// The request at index `path`, descending through folders.
fn at<'a>(
    mut items: &'a mut [ImportedItem],
    path: &[usize],
) -> Option<&'a mut crate::import::ImportedRequestItem> {
    let (last, folders) = path.split_last()?;
    for &i in folders {
        match items.get_mut(i)? {
            ImportedItem::Folder(f) => items = &mut f.items,
            _ => return None,
        }
    }
    match items.get_mut(*last)? {
        ImportedItem::Request(r) => Some(r),
        _ => None,
    }
}

/// Every object key in `root` that a chained name could equal, at any depth: environment
/// data nests, and over-collecting only lengthens a name.
pub(super) fn taken_names(root: &Value) -> HashSet<String> {
    let mut taken = HashSet::new();
    let mut stack = vec![root];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if key.starts_with(PREFIX) {
                        taken.insert(key.clone());
                    }
                    stack.push(child);
                }
            }
            Value::Array(list) => stack.extend(list),
            _ => {}
        }
    }
    taken
}

/// Lowercase ASCII letters and digits, every other run as one `_`.
fn slugify(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
        if out.len() >= SLUG_CHARS {
            break;
        }
    }
    let out = out.trim_end_matches('_');
    if out.is_empty() {
        "value".to_owned()
    } else {
        out.to_owned()
    }
}
