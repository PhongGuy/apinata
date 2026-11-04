//! One collection file compared with `HEAD`: the path guard, both sides, line hunks and the
//! changed request sections.

use std::io;
use std::path::{Component, Path};
use std::time::Duration;

use similar::{ChangeTag, TextDiff};
use yaml_serde::{Mapping, Value};

use super::{
    DiffLine, FileDiff, FileStatus, GitError, GitOptions, Hunk, LineKind, Section, StageState, run,
    show_prefix, status,
};
use crate::document::{is_special_file, read_regular};
use crate::order::{TOP, children_of};

/// Lines of unchanged context kept around each change, as git does by default.
const CONTEXT: usize = 3;

/// How long the line diff may search for a minimal result before settling for a coarser one.
const DIFF_TIMEOUT: Duration = Duration::from_secs(2);

/// Compare `rel` under `root` with `HEAD`. The path is checked before any git call; the status
/// comes from the uncapped collection-wide status, so a file past the cap still has one.
pub(super) fn file_diff(
    root: &Path,
    rel: &Path,
    options: &GitOptions,
) -> Result<FileDiff, GitError> {
    check_path(root, rel)?;
    let key = slash_key(rel);
    let prefix = show_prefix(root, options)?;
    let repo = status(root, &prefix, usize::MAX, options)?;
    let file_status = repo.files.get(Path::new(&key)).cloned();

    // The old side is absent for a new file or before the first commit, and a rename's lives
    // at its origin.
    let old_path = match &file_status {
        Some(FileStatus::Added | FileStatus::Untracked) => None,
        _ if repo.head.is_none() => None,
        Some(FileStatus::Renamed { from }) => Some(slash_key(from)),
        _ => Some(key.clone()),
    };
    let old = match old_path {
        Some(path) => show(root, &format!("HEAD:{prefix}{path}"), options)?,
        None => None,
    };
    // A file gone from disk is what a commit would record, which is the index's copy, or nothing
    // for a deletion; under a symlinked parent the disk path reads the link's target, which is
    // not that file. Git reports a pipe over a staged file as a working-tree edit, not a loss;
    // a tracked link is its target text, whatever it points at, so it never counts as a pipe.
    let path = root.join(rel);
    let new = match (&file_status, repo.stage.get(Path::new(&key))) {
        (Some(FileStatus::Deleted), _) => None,
        (_, Some(StageState::StagedGone)) => show(root, &format!(":0:{prefix}{key}"), options)?,
        (_, Some(StageState::Both))
            if is_special_file(&path) && !is_tracked_link(&path, old.is_some()) =>
        {
            show(root, &format!(":0:{prefix}{key}"), options)?
        }
        _ => on_disk(root, rel, old.is_some())?,
    };
    // With neither side the path is most likely a typo, which must not read as "unchanged".
    if old.is_none() && new.is_none() {
        return Err(GitError::Malformed(format!(
            "{} is not on disk or in HEAD",
            rel.display()
        )));
    }

    let mut diff = FileDiff {
        path: rel.to_path_buf(),
        status: file_status,
        old: None,
        new: None,
        binary: false,
        sections: Vec::new(),
        hunks: Vec::new(),
    };
    let text = |side: Option<Vec<u8>>| side.map(String::from_utf8).transpose();
    let (Ok(old), Ok(new)) = (text(old), text(new)) else {
        diff.binary = true;
        return Ok(diff);
    };
    diff.sections = sections(old.as_deref(), new.as_deref());
    diff.hunks = hunks(old.as_deref().unwrap_or(""), new.as_deref().unwrap_or(""));
    (diff.old, diff.new) = (old, new);
    Ok(diff)
}

/// Whether `path`, `tracked` in `HEAD`, is itself a link.
fn is_tracked_link(path: &Path, tracked: bool) -> bool {
    tracked && std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// The bytes of `rel` on disk, `None` when it is gone or a pipe, socket or device. A link tracked in `HEAD` is compared by its
/// target text, as git stores it; reading through it would set the target's content against that.
fn on_disk(root: &Path, rel: &Path, tracked: bool) -> Result<Option<Vec<u8>>, GitError> {
    let path = root.join(rel);
    let read = if is_tracked_link(&path, tracked) {
        std::fs::read_link(&path).map(|t| t.into_os_string().into_encoded_bytes())
    } else {
        read_regular(&path)
    };
    match read {
        Ok(bytes) => Ok(Some(bytes)),
        // A directory on the path, or a file over its parent, leaves it as gone as a missing one.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::NotADirectory
                    | io::ErrorKind::IsADirectory
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

/// The blob at `spec` (`HEAD:<path>` or `:0:<path>`), `None` when `HEAD` has no such path. Any other
/// failure, such as a blob a partial clone never fetched, is `Failed`.
fn show(root: &Path, spec: &str, options: &GitOptions) -> Result<Option<Vec<u8>>, GitError> {
    let args = ["show", spec];
    let out = run::git(&args, root, options)?;
    if out.status.success() {
        return Ok(Some(out.stdout));
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let absent = out.status.code() == Some(128)
        && (stderr.contains("does not exist in 'HEAD'")
            || stderr.contains("exists on disk, but not in 'HEAD'"));
    if absent {
        Ok(None)
    } else {
        Err(out.failed(&args))
    }
}

/// Refuse anything but a `.yml`/`.yaml` file made of plain relative components that resolves,
/// through any symlinks, inside the collection. A missing file is judged by its deepest existing
/// ancestor. The message names the path only, never its content.
pub(super) fn check_path(root: &Path, rel: &Path) -> Result<(), GitError> {
    let refuse = |why: &str| {
        GitError::Malformed(format!(
            "{} is not a request file in the collection: {why}",
            rel.display()
        ))
    };
    let plain = rel
        .components()
        .all(|part| matches!(part, Component::Normal(name) if name.to_str().is_some()));
    if rel.as_os_str().is_empty() || !plain {
        return Err(refuse("not a plain relative path"));
    }
    if !matches!(
        rel.extension().and_then(|x| x.to_str()),
        Some("yml" | "yaml")
    ) {
        return Err(refuse("not a .yml or .yaml file"));
    }
    let root = std::fs::canonicalize(root)?;
    let full = root.join(rel);
    let resolved = full
        .ancestors()
        .find_map(|path| std::fs::canonicalize(path).ok())
        .ok_or_else(|| refuse("it cannot be resolved"))?;
    if !resolved.starts_with(&root) {
        return Err(refuse("it resolves outside the collection"));
    }
    Ok(())
}

/// `rel`'s components joined with `/`, the form git and `RepoStatus::files` use.
fn slash_key(rel: &Path) -> String {
    rel.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The line hunks turning `old` into `new`, numbered like git's unified diff.
pub fn text_hunks(old: &str, new: &str) -> Vec<Hunk> {
    hunks(old, new)
}

/// [`text_hunks`] for the git module's own callers.
pub(super) fn hunks(old: &str, new: &str) -> Vec<Hunk> {
    hunks_within(old, new, DIFF_TIMEOUT)
}

/// [`hunks`] with a search limit; past it the diff is valid but no longer minimal.
pub(crate) fn hunks_within(old: &str, new: &str, limit: Duration) -> Vec<Hunk> {
    let diff = TextDiff::configure().timeout(limit).diff_lines(old, new);
    diff.grouped_ops(CONTEXT)
        .iter()
        .filter_map(|group| {
            let (first, last) = (group.first()?, group.last()?);
            // A side whose range in the group is empty names the line before it, so a start of
            // 0 is left 0-based; any other start becomes 1-based.
            let start = |from: usize, to: usize| if from == to { from } else { from + 1 };
            let lines = group
                .iter()
                .flat_map(|op| diff.iter_changes(op))
                .map(|change| DiffLine {
                    no_newline: !change.value().ends_with('\n'),
                    kind: match change.tag() {
                        ChangeTag::Equal => LineKind::Context,
                        ChangeTag::Insert => LineKind::Added,
                        ChangeTag::Delete => LineKind::Removed,
                    },
                    text: change
                        .value()
                        .strip_suffix('\n')
                        .unwrap_or(change.value())
                        .to_owned(),
                })
                .collect();
            Some(Hunk {
                old_start: start(first.old_range().start, last.old_range().end),
                new_start: start(first.new_range().start, last.new_range().end),
                lines,
            })
        })
        .collect()
}

/// The request sections whose YAML values differ between the two sides, in writer order with
/// `Other` last. An absent or null side is empty; a side that is not a YAML mapping gives none.
pub(super) fn sections(old: Option<&str>, new: Option<&str>) -> Vec<Section> {
    let (Some(old), Some(new)) = (document(old), document(new)) else {
        return Vec::new();
    };
    let mut changed = Vec::new();
    for key in union(&old, &new) {
        let (before, after) = (old.get(&key), new.get(&key));
        if before == after {
            continue;
        }
        let name = key.as_str().unwrap_or_default();
        if !is_parent(name) {
            changed.push(top_section(name));
            continue;
        }
        // A parent is compared child by child; one that is not a mapping on a side is Other.
        match (as_mapping(before), as_mapping(after)) {
            (Some(before), Some(after)) => {
                for child in union(&before, &after) {
                    if before.get(&child) != after.get(&child) {
                        changed.push(child_section(name, child.as_str().unwrap_or_default()));
                    }
                }
            }
            _ => changed.push(Section::Other),
        }
    }
    writer_order()
        .into_iter()
        .chain([Section::Other])
        .filter(|section| changed.contains(section))
        .collect()
}

/// Parse one side: absent or null is an empty mapping, anything but a mapping is `None`.
fn document(text: Option<&str>) -> Option<Mapping> {
    let Some(text) = text else {
        return Some(Mapping::new());
    };
    crate::document::bound_expansion(text).ok()?;
    match yaml_serde::from_str::<Value>(text).ok()? {
        Value::Null => Some(Mapping::new()),
        Value::Mapping(map) => Some(map),
        _ => None,
    }
}

/// A parent's value as a mapping: absent is empty, any other value is `None`.
fn as_mapping(value: Option<&Value>) -> Option<Mapping> {
    match value {
        None => Some(Mapping::new()),
        Some(Value::Mapping(map)) => Some(map.clone()),
        Some(_) => None,
    }
}

/// The keys of `a` then the keys of `b` not in `a`.
fn union(a: &Mapping, b: &Mapping) -> Vec<Value> {
    a.keys()
        .chain(b.keys().filter(|key| !a.contains_key(*key)))
        .cloned()
        .collect()
}

/// Top-level keys whose children are sections of their own.
fn is_parent(key: &str) -> bool {
    matches!(key, "http" | "graphql" | "runtime")
}

fn top_section(key: &str) -> Section {
    match key {
        "info" => Section::Info,
        "settings" => Section::Settings,
        "examples" => Section::Examples,
        "docs" => Section::Docs,
        _ => Section::Other,
    }
}

fn child_section(parent: &str, key: &str) -> Section {
    if !children_of(parent).contains(&key) {
        return Section::Other;
    }
    match key {
        "method" => Section::Method,
        "url" => Section::Url,
        "headers" => Section::Headers,
        "params" => Section::Params,
        "body" => Section::Body,
        "auth" => Section::Auth,
        "variables" => Section::Vars,
        "scripts" => Section::Scripts,
        "assertions" => Section::Assertions,
        "actions" => Section::Actions,
        _ => Section::Other,
    }
}

/// Every named section once, in the order the writer emits its keys.
fn writer_order() -> Vec<Section> {
    let mut order: Vec<Section> = Vec::new();
    for key in TOP {
        let found: Vec<Section> = if is_parent(key) {
            children_of(key)
                .iter()
                .map(|child| child_section(key, child))
                .collect()
        } else {
            vec![top_section(key)]
        };
        for section in found {
            if section != Section::Other && !order.contains(&section) {
                order.push(section);
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{hunks, hunks_within, sections};
    use crate::git::{Hunk, LineKind, Section};

    /// Hunks as text: a `@@ old new` line, then each line behind its gutter.
    fn shown(hunks: &[Hunk]) -> Vec<String> {
        let mut out = Vec::new();
        for hunk in hunks {
            out.push(format!("@@ {} {}", hunk.old_start, hunk.new_start));
            for line in &hunk.lines {
                let gutter = match line.kind {
                    LineKind::Context => ' ',
                    LineKind::Added => '+',
                    LineKind::Removed => '-',
                };
                out.push(format!("{gutter}{}", line.text));
            }
        }
        out
    }

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("l{i}\n")).collect()
    }

    #[test]
    fn a_changed_line_is_removed_then_added_within_its_context() {
        assert_eq!(
            shown(&hunks("a\nb\nc\n", "a\nB\nc\n")),
            ["@@ 1 1", " a", "-b", "+B", " c"]
        );
    }

    #[test]
    fn context_is_three_lines_each_side() {
        let new = numbered(10).replace("l6\n", "L6\n");
        assert_eq!(
            shown(&hunks(&numbered(10), &new)),
            [
                "@@ 3 3", " l3", " l4", " l5", "-l6", "+L6", " l7", " l8", " l9"
            ]
        );
    }

    #[test]
    fn distant_changes_make_separate_hunks() {
        let new = numbered(20)
            .replace("l1\n", "L1\n")
            .replace("l20\n", "L20\n");
        let heads: Vec<String> = shown(&hunks(&numbered(20), &new))
            .into_iter()
            .filter(|line| line.starts_with("@@"))
            .collect();
        assert_eq!(heads, ["@@ 1 1", "@@ 17 17"]);
    }

    #[test]
    fn an_empty_side_starts_at_zero() {
        assert_eq!(shown(&hunks("", "x\ny\n")), ["@@ 0 1", "+x", "+y"]);
        assert_eq!(shown(&hunks("x\n", "")), ["@@ 1 0", "-x"]);
    }

    #[test]
    fn identical_texts_have_no_hunks() {
        assert!(hunks("a\nb\n", "a\nb\n").is_empty());
    }

    #[test]
    fn a_diff_past_its_time_limit_still_rebuilds_both_sides() {
        let old: String = (0..40).map(|i| format!("{}\n", i % 7)).collect();
        let new: String = (0..40).map(|i| format!("{}\n", i * 3 % 5)).collect();
        let found = hunks_within(&old, &new, Duration::ZERO);
        let side = |keep: LineKind| -> String {
            found
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .filter(|line| line.kind == LineKind::Context || line.kind == keep)
                .map(|line| format!("{}\n", line.text))
                .collect()
        };
        // The second lines already differ, so one hunk spans both files from the first line.
        assert_eq!((side(LineKind::Removed), side(LineKind::Added)), (old, new));
    }

    #[test]
    fn line_text_has_no_newline() {
        assert_eq!(shown(&hunks("a", "b")), ["@@ 1 1", "-a", "+b"]);
    }

    #[test]
    fn a_last_line_without_a_newline_is_flagged() {
        let flags = |old: &str, new: &str| -> Vec<(String, bool)> {
            hunks(old, new)
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .map(|line| (line.text.clone(), line.no_newline))
                .collect()
        };
        assert_eq!(
            flags("a", "b\n"),
            [("a".to_owned(), true), ("b".to_owned(), false)]
        );
        assert_eq!(
            flags("a\n", "b"),
            [("a".to_owned(), false), ("b".to_owned(), true)]
        );
        // Both sides end the same way: the shared context line carries the flag once.
        assert_eq!(
            flags("x\na", "x\nb"),
            [
                ("x".to_owned(), false),
                ("a".to_owned(), true),
                ("b".to_owned(), true)
            ]
        );
    }

    const BASE: &str = "info:\n  name: get\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://example.com/v1\n  headers:\n    - name: a\n      value: b\nruntime:\n  variables:\n    - name: v\n      value: \"1\"\nsettings:\n  timeout: 0\ndocs: hello\n";

    fn changed(from: &str, to: &str) -> Vec<Section> {
        sections(Some(BASE), Some(&BASE.replace(from, to)))
    }

    /// BASE with `line` inserted just before `at`.
    fn inserted(at: &str, line: &str) -> Vec<Section> {
        sections(Some(BASE), Some(&BASE.replace(at, &format!("{line}{at}"))))
    }

    #[test]
    fn each_changed_key_maps_to_its_section() {
        assert_eq!(changed("name: get", "name: got"), [Section::Info]);
        assert_eq!(changed("method: GET", "method: POST"), [Section::Method]);
        assert_eq!(changed("/v1", "/v2"), [Section::Url]);
        assert_eq!(changed("value: b", "value: c"), [Section::Headers]);
        assert_eq!(changed("value: \"1\"", "value: \"2\""), [Section::Vars]);
        assert_eq!(changed("timeout: 0", "timeout: 5"), [Section::Settings]);
        assert_eq!(changed("docs: hello", "docs: bye"), [Section::Docs]);
    }

    #[test]
    fn added_children_map_by_their_parent() {
        assert_eq!(inserted("  headers:", "  params: []\n"), [Section::Params]);
        assert_eq!(inserted("  headers:", "  body: {}\n"), [Section::Body]);
        assert_eq!(inserted("  headers:", "  auth: inherit\n"), [Section::Auth]);
        assert_eq!(inserted("  headers:", "  extra: 1\n"), [Section::Other]);
        assert_eq!(
            inserted("  variables:", "  scripts: []\n"),
            [Section::Scripts]
        );
        assert_eq!(
            inserted("  variables:", "  assertions: []\n"),
            [Section::Assertions]
        );
        assert_eq!(
            inserted("  variables:", "  actions: []\n"),
            [Section::Actions]
        );
        let more = |extra: &str| sections(Some(BASE), Some(&format!("{BASE}{extra}")));
        assert_eq!(more("examples: []\n"), [Section::Examples]);
        assert_eq!(more("app: {}\n"), [Section::Other]);
    }

    #[test]
    fn graphql_children_share_the_http_sections() {
        assert_eq!(
            sections(Some("graphql:\n  url: a\n"), Some("graphql:\n  url: b\n")),
            [Section::Url]
        );
    }

    #[test]
    fn sections_follow_the_writer_order_once_each() {
        let new = BASE
            .replace("docs: hello", "docs: bye")
            .replace("timeout: 0", "timeout: 5")
            .replace("/v1", "/v2")
            .replace("name: get", "name: got")
            + "app: {}\nzzz: 1\n";
        assert_eq!(
            sections(Some(BASE), Some(&new)),
            [
                Section::Info,
                Section::Url,
                Section::Settings,
                Section::Docs,
                Section::Other
            ]
        );
    }

    #[test]
    fn an_absent_or_empty_side_counts_every_present_section() {
        let all = [
            Section::Info,
            Section::Method,
            Section::Url,
            Section::Headers,
            Section::Vars,
            Section::Settings,
            Section::Docs,
        ];
        assert_eq!(sections(None, Some(BASE)), all);
        assert_eq!(sections(Some(BASE), None), all);
        assert_eq!(sections(Some(""), Some(BASE)), all);
    }

    #[test]
    fn unparseable_non_mapping_or_identical_sides_have_no_sections() {
        assert!(sections(Some(BASE), Some("info: [unclosed\n")).is_empty());
        assert!(sections(Some("just text"), Some(BASE)).is_empty());
        assert!(sections(Some(BASE), Some(BASE)).is_empty());
    }

    #[test]
    fn a_parent_that_stops_being_a_mapping_is_other() {
        let flat = BASE.replace(
            "http:\n  method: GET\n  url: https://example.com/v1\n  headers:\n    - name: a\n      value: b\n",
            "http: none\n",
        );
        assert_eq!(sections(Some(BASE), Some(&flat)), [Section::Other]);
    }
}
