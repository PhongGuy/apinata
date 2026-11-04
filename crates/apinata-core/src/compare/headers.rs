//! The header half of a comparison.

use super::{HeaderChange, HeaderChangeKind};

/// Headers that change on every send, so a difference in them is noise.
const VOLATILE: &[&str] = &[
    "date",
    "age",
    "etag",
    "last-modified",
    "server-timing",
    "x-request-id",
    "x-trace-id",
    "set-cookie",
];

/// One header name with every value it carried, in received order.
struct Group {
    lower: String,
    spelling: String,
    values: Vec<String>,
}

/// Groups repeated headers by case-insensitive name, keeping first appearance order.
fn group(headers: &[(String, String)]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        match groups.iter_mut().find(|g| g.lower == lower) {
            Some(g) => g.values.push(value.clone()),
            None => groups.push(Group {
                lower,
                spelling: name.clone(),
                values: vec![value.clone()],
            }),
        }
    }
    groups
}

/// Lists the headers that differ: old-side names in appearance order, then names only the new
/// side has. Value order matters within a name; order across names does not.
pub(super) fn compare_headers(
    old: &[(String, String)],
    new: &[(String, String)],
) -> Vec<HeaderChange> {
    let (old, new) = (group(old), group(new));
    let change = |g: &Group, kind, old, new| HeaderChange {
        name: g.spelling.clone(),
        kind,
        old,
        new,
        volatile: VOLATILE.contains(&g.lower.as_str()),
    };
    let mut out = Vec::new();
    for o in &old {
        match new.iter().find(|n| n.lower == o.lower) {
            None => out.push(change(
                o,
                HeaderChangeKind::Removed,
                o.values.clone(),
                Vec::new(),
            )),
            Some(n) if n.values != o.values => out.push(change(
                o,
                HeaderChangeKind::Changed,
                o.values.clone(),
                n.values.clone(),
            )),
            Some(_) => {}
        }
    }
    for n in &new {
        if !old.iter().any(|o| o.lower == n.lower) {
            out.push(change(
                n,
                HeaderChangeKind::Added,
                Vec::new(),
                n.values.clone(),
            ));
        }
    }
    out
}
