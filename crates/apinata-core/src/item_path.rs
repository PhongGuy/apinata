//! Paths of collection items relative to the collection root.

use std::fmt;
use std::path::{Path, PathBuf};

/// A `/`-separated path of a folder or file relative to the collection root; `""` is the root.
///
/// Every segment is non-empty, neither `.` nor `..`, and free of `\` and `:`, so an item path
/// never leaves the root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub struct ItemPath(String);

impl ItemPath {
    /// The collection root.
    pub fn root() -> ItemPath {
        ItemPath(String::new())
    }

    /// Parse a relative path; `None` when it is absolute or has an empty, `.` or `..` segment
    /// or one containing `\` or `:`.
    pub fn new(rel: &str) -> Option<ItemPath> {
        if rel.is_empty() {
            return Some(ItemPath::root());
        }
        rel.split('/')
            .all(is_segment)
            .then(|| ItemPath(rel.to_owned()))
    }

    /// The path as `/`-separated segments.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the collection root.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The containing folder's path; `None` for the root.
    pub fn parent(&self) -> Option<ItemPath> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rsplit_once('/') {
            Some((parent, _)) => ItemPath(parent.to_owned()),
            None => ItemPath::root(),
        })
    }

    /// The last segment; `""` for the root.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or("")
    }

    /// Whether this names an unsaved draft request: its file name is exactly
    /// `untitled-<digits>.draft`, the shape core gives drafts, so a real `x.draft` folder is not one.
    pub fn is_draft(&self) -> bool {
        self.file_name()
            .strip_prefix("untitled-")
            .and_then(|rest| rest.strip_suffix(".draft"))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    }

    /// The path of `name` inside this one; `None` unless `name` is a single valid segment.
    pub fn join(&self, name: &str) -> Option<ItemPath> {
        if !is_segment(name) {
            return None;
        }
        Some(if self.is_root() {
            ItemPath(name.to_owned())
        } else {
            ItemPath(format!("{}/{name}", self.0))
        })
    }

    /// The disk path under the collection directory `root`.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        self.segments().fold(root.to_path_buf(), |p, s| p.join(s))
    }

    /// The segments from the root down; none for the root.
    pub(crate) fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|s| !s.is_empty())
    }
}

impl fmt::Display for ItemPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A segment must stay one path component on every platform, so Windows separators and drive
/// prefixes are refused along with `/`.
fn is_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', ':'])
}

#[cfg(test)]
mod tests {
    use super::ItemPath;

    #[test]
    fn segments_with_windows_separators_or_drive_prefixes_are_rejected() {
        for bad in ["a\\..\\..\\x", "C:", "a/C:x", "a\\b", "..\\x"] {
            assert_eq!(ItemPath::new(bad), None, "{bad}");
            assert_eq!(ItemPath::root().join(bad), None, "{bad}");
        }
    }
}
