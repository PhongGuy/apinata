//! Parse `git status --porcelain=v2 -z --branch` output into a [`RepoStatus`].

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use super::{FileStatus, GitError, RepoStatus, StageState};

/// The empty blob's id in SHA-1 and SHA-256 repositories: what an `add -N` entry records.
const EMPTY_BLOBS: [&[u8]; 2] = [
    b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
    b"473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813",
];

/// Parse `out` into the branch, the short `HEAD`, the changed files and their stage states. `prefix` is the
/// collection's path in the repository (`"col/"` or `""`); files outside it, submodules and
/// non-UTF-8 paths are skipped, and only the first `cap` files in path order are kept.
/// `head_paths` gives the repository paths in `HEAD`, `None` when they cannot be read; it is called
/// only to tell an `add -N` path gone from disk from a deleted empty file, which print alike.
pub(crate) fn parse(
    out: &[u8],
    prefix: &str,
    cap: usize,
    head_paths: impl FnOnce() -> Option<HashSet<Vec<u8>>>,
) -> Result<RepoStatus, GitError> {
    let mut oid: Option<&str> = None;
    let mut branch: Option<String> = None;
    let mut files: BTreeMap<PathBuf, FileStatus> = BTreeMap::new();
    let mut stage: BTreeMap<PathBuf, StageState> = BTreeMap::new();
    let mut moved_away: Vec<(&str, &[u8])> = Vec::new();
    let mut kept_at: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    let mut gone_empty: Vec<(PathBuf, &[u8])> = Vec::new();
    let mut records = out.split(|&b| b == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        match record[0] {
            b'#' => {
                let header = std::str::from_utf8(record)
                    .map_err(|_| malformed("git status header is not UTF-8"))?;
                if let Some(value) = header.strip_prefix("# branch.oid ") {
                    oid = Some(value);
                } else if let Some(value) = header.strip_prefix("# branch.head ") {
                    branch = (value != "(detached)").then(|| value.to_owned());
                }
            }
            // An ordinary change: the file is in HEAD unless it was added, staged or by `add -N`
            // (`.A`), and on disk unless either column deleted it. A path the index changed and
            // the disk lost is what a commit records from the index; `DD`, a tracked path re-added
            // by `add -N`, holds nothing a commit keeps. `DA` is that path still on disk: a staged
            // deletion and an unstaged add, read as `D.` plus `?`.
            b'1' => {
                let f = fields(record, 9)?;
                let (x, y) = xy(f[1])?;
                let in_head = x != b'A' && (x, y) != (b'.', b'A');
                let indexed_gone = y == b'D' && !matches!(x, b'.' | b'D');
                let (status, at) = match (in_head, indexed_gone) {
                    (true, true) => (FileStatus::Modified, StageState::StagedGone),
                    (false, true) => (FileStatus::Added, StageState::StagedGone),
                    (true, false) if y != b'A' && (x == b'D' || y == b'D') => {
                        (FileStatus::Deleted, stage_of(x, y))
                    }
                    (true, false) => (FileStatus::Modified, stage_of(x, y)),
                    (false, false) => (FileStatus::Added, stage_of(x, y)),
                };
                if !submodule(f[2]) {
                    insert(&mut files, &mut stage, prefix, f[8], status, at);
                    if (x, y) == (b'.', b'D') {
                        gone_empty.extend(empty_entry(prefix, f[8], f[7]));
                    }
                }
            }
            // A rename or copy carries its origin in the next NUL-separated field.
            b'2' => {
                let f = fields(record, 10)?;
                let origin = records
                    .next()
                    .ok_or_else(|| malformed("git status rename record has no origin path"))?;
                let (x, y) = xy(f[1])?;
                let Some(origin) = utf8(origin) else { continue };
                if submodule(f[2]) {
                    continue;
                }
                // A worktree copy or rename pairs an `add -N` path with its origin. A copy's origin
                // has its own record; a rename is the only trace of the origin's deletion, which is
                // settled once every record for it has passed. `D` in the index column is a tracked
                // path re-added that way, which reads as `DA`.
                if matches!(x, b'.' | b'D') && matches!(y, b'R' | b'C') {
                    let (status, at) = match x {
                        b'.' => (FileStatus::Added, StageState::Unstaged),
                        _ => (FileStatus::Modified, StageState::Both),
                    };
                    insert(&mut files, &mut stage, prefix, f[9], status, at);
                    if y == b'R' {
                        moved_away.push((origin, f[9]));
                        gone_empty.extend(empty_entry(prefix, origin.as_bytes(), f[7]));
                    }
                    continue;
                }
                // What is left of a rename is staged (`R.`, `RM`, `RD`); a copy is only ever staged.
                let kind = if x == b'R' || y == b'R' { b'R' } else { x };
                match (kind, y != b'D') {
                    // A rename from outside the collection is new to the collection.
                    (b'R', true) => {
                        let status = match inside(prefix, origin) {
                            Some(from) => FileStatus::Renamed { from },
                            None => FileStatus::Added,
                        };
                        insert(&mut files, &mut stage, prefix, f[9], status, stage_of(x, y));
                    }
                    // The origin's deletion is wholly staged, and the index holds the new path.
                    (b'R', false) => {
                        let (added, at) = (FileStatus::Added, StageState::StagedGone);
                        insert(&mut files, &mut stage, prefix, f[9], added, at);
                        let (gone, at) = (FileStatus::Deleted, StageState::Staged);
                        insert(&mut files, &mut stage, prefix, origin.as_bytes(), gone, at);
                    }
                    (b'C', on_disk) => {
                        let at = match on_disk {
                            true => stage_of(x, y),
                            false => StageState::StagedGone,
                        };
                        insert(&mut files, &mut stage, prefix, f[9], FileStatus::Added, at)
                    }
                    _ => return Err(malformed("git status rename record is neither R nor C")),
                }
            }
            b'u' => {
                let f = fields(record, 11)?;
                if !submodule(f[2]) {
                    insert(
                        &mut files,
                        &mut stage,
                        prefix,
                        f[10],
                        FileStatus::Conflicted,
                        StageState::Unstaged,
                    );
                }
            }
            // Untracked, unless the index just dropped a file still on disk (`D.` plus `?`).
            b'?' => {
                let path = record
                    .strip_prefix(b"? ")
                    .ok_or_else(|| malformed("git status untracked record has no path"))?;
                // An untracked nested repository is reported as a directory, `col/nested/`.
                let path = path.strip_suffix(b"/").unwrap_or(path);
                let Some(rel) = utf8(path).and_then(|p| inside(prefix, p)) else {
                    continue;
                };
                let (status, at) = match files.get(&rel) {
                    Some(FileStatus::Deleted) => (FileStatus::Modified, StageState::Both),
                    // A nested repository on the path leaves the index's copy gone from disk.
                    Some(_) if stage.get(&rel) == Some(&StageState::StagedGone) => continue,
                    _ => (FileStatus::Untracked, StageState::Unstaged),
                };
                stage.insert(rel.clone(), at);
                files.insert(rel, status);
            }
            _ => {
                return Err(malformed(&format!(
                    "git status record {:?} is not understood",
                    String::from_utf8_lossy(record)
                )));
            }
        }
    }
    for (origin, to) in moved_away {
        let Some(rel) = inside(prefix, origin) else {
            continue;
        };
        delete_in_worktree(&mut files, &mut stage, rel.clone());
        // A row left holding only the index still has its file, at the path it moved to.
        if stage.get(&rel) == Some(&StageState::StagedGone)
            && let Some(at) = utf8(to).and_then(|to| inside(prefix, to))
        {
            kept_at.insert(rel, at);
        }
    }
    let oid = oid.ok_or_else(|| malformed("git status output has no branch.oid header"))?;
    // An unborn branch reports `(initial)` and has no commit to name.
    let head = match oid {
        "(initial)" => None,
        _ => Some(
            oid.get(..7)
                .ok_or_else(|| malformed(&format!("git status branch.oid {oid}")))?
                .to_owned(),
        ),
    };
    // Hidden before the cap, so a row that is not listed never pushes out one that is.
    let in_head = match (gone_empty.is_empty(), oid) {
        (true, _) => None,
        (false, "(initial)") => Some(HashSet::new()),
        (false, _) => head_paths(),
    };
    if let Some(in_head) = in_head {
        for (rel, path) in gone_empty {
            if !in_head.contains(path) && files.get(&rel) == Some(&FileStatus::Deleted) {
                files.remove(&rel);
                stage.remove(&rel);
            }
        }
    }
    let truncated = files.len() > cap;
    if truncated {
        files = files.into_iter().take(cap).collect();
        stage = stage.into_iter().take(cap).collect();
        kept_at.retain(|rel, _| files.contains_key(rel));
    }
    Ok(RepoStatus {
        branch,
        head,
        files,
        stage,
        truncated,
        kept_at,
    })
}

fn malformed(message: &str) -> GitError {
    GitError::Malformed(message.to_owned())
}

/// Split a record into exactly `n` space-separated fields; the last, the path, may hold spaces.
fn fields(record: &[u8], n: usize) -> Result<Vec<&[u8]>, GitError> {
    let f: Vec<&[u8]> = record.splitn(n, |&b| b == b' ').collect();
    if f.len() == n {
        Ok(f)
    } else {
        Err(malformed(&format!(
            "git status record {:?} has too few fields",
            String::from_utf8_lossy(record)
        )))
    }
}

fn xy(field: &[u8]) -> Result<(u8, u8), GitError> {
    match field {
        [x, y] => Ok((*x, *y)),
        _ => Err(malformed("git status record has a bad XY field")),
    }
}

/// The submodule field reads `S...` for a submodule and `N...` otherwise.
fn submodule(field: &[u8]) -> bool {
    field.first() == Some(&b'S')
}

fn utf8(path: &[u8]) -> Option<&str> {
    std::str::from_utf8(path).ok()
}

/// The collection-relative form of a repository path, `None` when it lies outside `prefix`.
fn inside(prefix: &str, path: &str) -> Option<PathBuf> {
    path.strip_prefix(prefix)
        .filter(|rel| !rel.is_empty())
        .map(PathBuf::from)
}

/// Index changed is `X != '.'`, worktree changed is `Y != '.'`.
fn stage_of(x: u8, y: u8) -> StageState {
    match (x != b'.', y != b'.') {
        (true, false) => StageState::Staged,
        (false, _) => StageState::Unstaged,
        (true, true) => StageState::Both,
    }
}

/// The collection-relative and repository forms of `path` when its index entry `hash` is the empty
/// blob, which is what `add -N` leaves there.
fn empty_entry<'a>(prefix: &str, path: &'a [u8], hash: &[u8]) -> Option<(PathBuf, &'a [u8])> {
    let rel = utf8(path).and_then(|p| inside(prefix, p))?;
    EMPTY_BLOBS.contains(&hash).then_some((rel, path))
}

/// Record `status` and `at` at a repository path, skipping paths that are not UTF-8 or not in the
/// collection.
fn insert(
    files: &mut BTreeMap<PathBuf, FileStatus>,
    stage: &mut BTreeMap<PathBuf, StageState>,
    prefix: &str,
    path: &[u8],
    status: FileStatus,
    at: StageState,
) {
    if let Some(rel) = utf8(path).and_then(|p| inside(prefix, p)) {
        stage.insert(rel.clone(), at);
        files.insert(rel, status);
    }
}

/// Delete `rel` in the worktree over whatever its record says, by the `MD` and `AD` rules: a path
/// the index changed stays listed, since a commit records it. A staged rename also leaves its
/// origin's deletion wholly staged, or modified when that is back on disk.
fn delete_in_worktree(
    files: &mut BTreeMap<PathBuf, FileStatus>,
    stage: &mut BTreeMap<PathBuf, StageState>,
    rel: PathBuf,
) {
    match files.get(&rel).cloned() {
        None => {
            stage.insert(rel.clone(), StageState::Unstaged);
            files.insert(rel, FileStatus::Deleted);
        }
        Some(FileStatus::Modified | FileStatus::Added) => {
            stage.insert(rel, StageState::StagedGone);
        }
        Some(FileStatus::Renamed { from }) => {
            stage.insert(rel.clone(), StageState::StagedGone);
            files.insert(rel, FileStatus::Added);
            let (status, at) = match files.get(&from) {
                Some(FileStatus::Untracked) => (FileStatus::Modified, StageState::Both),
                _ => (FileStatus::Deleted, StageState::Staged),
            };
            stage.insert(from.clone(), at);
            files.insert(from, status);
        }
        Some(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::parse as parse_with;
    use crate::git::{GIT_FILE_CAP, GitError, RepoStatus};
    use std::collections::HashSet;

    const OID: &str = "0123456789abcdef0123456789abcdef01234567";

    /// Parse where no record may need the lookup of `HEAD`'s paths.
    fn parse(out: &[u8], prefix: &str, cap: usize) -> Result<RepoStatus, GitError> {
        parse_with(out, prefix, cap, || panic!("an unneeded lookup of HEAD"))
    }
    fn header() -> String {
        format!("# branch.oid {OID}\0# branch.head main\0")
    }
    fn status(records: &str, prefix: &str) -> RepoStatus {
        parse(
            format!("{}{records}", header()).as_bytes(),
            prefix,
            GIT_FILE_CAP,
        )
        .expect("parses")
    }

    #[test]
    fn branch_headers_give_the_branch_and_a_short_head() {
        let s = status("", "");
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.head.as_deref(), Some("0123456"));
        assert!(s.files.is_empty() && !s.truncated);

        let detached = parse(
            format!("# branch.oid {OID}\0# branch.head (detached)\0").as_bytes(),
            "",
            GIT_FILE_CAP,
        )
        .expect("parses");
        assert_eq!(detached.branch, None);
        assert_eq!(detached.head.as_deref(), Some("0123456"));

        let unborn = parse(
            b"# branch.oid (initial)\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +0 -0\0",
            "",
            GIT_FILE_CAP,
        )
        .expect("parses");
        assert_eq!(unborn.branch.as_deref(), Some("main"));
        assert_eq!(unborn.head, None);
    }

    #[test]
    fn a_status_without_a_branch_oid_header_is_malformed() {
        for bad in [&b""[..], b"# branch.head main\0"] {
            assert!(matches!(
                parse(bad, "", GIT_FILE_CAP),
                Err(GitError::Malformed(_))
            ));
        }
    }

    use crate::git::FileStatus;
    use std::path::PathBuf;

    const MODES: &str = "100644 100644 100644";
    const HASHES: &str =
        "cb46010117ba62af321588c78f2e681868628485 cb46010117ba62af321588c78f2e681868628485";

    fn one(xy: &str, path: &str) -> String {
        format!("1 {xy} N... {MODES} {HASHES} {path}\0")
    }
    fn two(xy: &str, score: &str, path: &str, from: &str) -> String {
        format!("2 {xy} N... {MODES} {HASHES} {score} {path}\0{from}\0")
    }
    fn unmerged(path: &str) -> String {
        format!("u UU N... 100644 100644 100644 100644 {HASHES} {OID} {path}\0")
    }
    fn untracked(path: &str) -> String {
        format!("? {path}\0")
    }
    fn files(records: &str) -> Vec<(String, FileStatus)> {
        status(records, "col/")
            .files
            .into_iter()
            .map(|(path, s)| (path.to_string_lossy().into_owned(), s))
            .collect()
    }
    fn file(path: &str, s: FileStatus) -> Vec<(String, FileStatus)> {
        vec![(path.to_owned(), s)]
    }

    #[test]
    fn ordinary_records_follow_the_head_and_disk_rules() {
        let cases = [
            (".M", Some(FileStatus::Modified)),
            ("M.", Some(FileStatus::Modified)),
            ("MM", Some(FileStatus::Modified)),
            (".T", Some(FileStatus::Modified)),
            (".A", Some(FileStatus::Added)),
            ("A.", Some(FileStatus::Added)),
            ("AM", Some(FileStatus::Added)),
            (".D", Some(FileStatus::Deleted)),
            ("D.", Some(FileStatus::Deleted)),
            ("DD", Some(FileStatus::Deleted)),
            ("MD", Some(FileStatus::Modified)),
            ("TD", Some(FileStatus::Modified)),
            ("AD", Some(FileStatus::Added)),
        ];
        for (xy, expected) in cases {
            let want: Vec<(String, FileStatus)> = expected
                .into_iter()
                .map(|s| ("a.yml".to_owned(), s))
                .collect();
            assert_eq!(files(&one(xy, "col/a.yml")), want, "{xy}");
        }
    }

    #[test]
    fn renames_keep_their_origin_unless_the_new_path_is_gone() {
        assert_eq!(
            files(&two("R.", "R100", "col/sub/new.yml", "col/old.yml")),
            file(
                "sub/new.yml",
                FileStatus::Renamed {
                    from: PathBuf::from("old.yml")
                }
            )
        );
        assert_eq!(
            files(&two("RM", "R87", "col/new.yml", "col/old.yml")),
            file(
                "new.yml",
                FileStatus::Renamed {
                    from: PathBuf::from("old.yml")
                }
            )
        );
        assert_eq!(
            files(&two("RD", "R100", "col/new.yml", "col/old.yml")),
            [
                ("new.yml".to_owned(), FileStatus::Added),
                ("old.yml".to_owned(), FileStatus::Deleted)
            ]
        );
    }

    #[test]
    fn a_staged_deletion_with_an_unstaged_add_is_modified_and_partly_staged() {
        let record = one("DA", "col/n.yml");
        assert_eq!(files(&record), file("n.yml", FileStatus::Modified));
        assert_eq!(stages(&record), one_stage("n.yml", StageState::Both));
    }

    #[test]
    fn a_staged_deletion_with_an_add_paired_to_a_copy_keeps_the_origin_record() {
        let records = two("DC", "C100", "col/n.yml", "col/old.yml") + &one(".M", "col/old.yml");
        assert_eq!(
            files(&records),
            [
                ("n.yml".to_owned(), FileStatus::Modified),
                ("old.yml".to_owned(), FileStatus::Modified),
            ]
        );
        assert_eq!(stages(&records)[0].1, StageState::Both);
    }

    #[test]
    fn a_staged_deletion_with_an_add_paired_to_a_rename_deletes_the_origin() {
        let record = two("DR", "R100", "col/n.yml", "col/old.yml");
        assert_eq!(
            files(&record),
            [
                ("n.yml".to_owned(), FileStatus::Modified),
                ("old.yml".to_owned(), FileStatus::Deleted),
            ]
        );
        assert_eq!(stages(&record)[0].1, StageState::Both);
    }

    #[test]
    fn a_gone_path_outside_the_collection_stays_skipped() {
        assert!(files(&one("AD", "other/n.yml")).is_empty());
        assert_eq!(
            files(&two("RD", "R100", "other/new.yml", "col/old.yml")),
            file("old.yml", FileStatus::Deleted)
        );
        assert_eq!(
            stages(&two("RD", "R100", "col/new.yml", "other/old.yml")),
            one_stage("new.yml", StageState::StagedGone)
        );
        let worktree = two(".R", "R100", "other/p.yml", "other/o.yml");
        assert!(files(&(one("A.", "other/o.yml") + &worktree)).is_empty());
    }

    #[test]
    fn an_added_empty_file_gone_from_disk_is_listed_without_a_lookup() {
        let record = format!("1 AD N... 000000 100644 000000 {OID0} {EMPTY_SHA1} col/a.yml\0");
        assert_eq!(stages(&record), one_stage("a.yml", StageState::StagedGone));
    }

    #[test]
    fn a_rename_from_outside_the_collection_is_added() {
        assert_eq!(
            files(&two("R.", "R100", "col/new.yml", "other/old.yml")),
            file("new.yml", FileStatus::Added)
        );
    }

    #[test]
    fn copies_are_added() {
        assert_eq!(
            files(&two("C.", "C75", "col/copy.yml", "col/orig.yml")),
            file("copy.yml", FileStatus::Added)
        );
        let gone = two("CD", "C75", "col/copy.yml", "col/orig.yml");
        assert_eq!(files(&gone), file("copy.yml", FileStatus::Added));
        assert_eq!(stages(&gone), one_stage("copy.yml", StageState::StagedGone));
    }

    #[test]
    fn a_worktree_copy_is_the_new_path_added_and_leaves_its_origin_to_its_own_record() {
        let records = two(".C", "C100", "col/cp.yml", "col/old.yml") + &one(".M", "col/old.yml");
        assert_eq!(
            files(&records),
            [
                ("cp.yml".to_owned(), FileStatus::Added),
                ("old.yml".to_owned(), FileStatus::Modified),
            ]
        );
        assert_eq!(
            stages(&records),
            [
                ("cp.yml".to_owned(), StageState::Unstaged),
                ("old.yml".to_owned(), StageState::Unstaged),
            ]
        );
    }

    #[test]
    fn a_worktree_copy_beside_a_rename_of_its_origin_deletes_the_origin_once() {
        let copy = two(".C", "C100", "col/a.yml", "col/old.yml");
        let rename = two(".R", "R100", "col/b.yml", "col/old.yml");
        assert_eq!(
            files_either_way(&copy, &rename),
            [
                ("a.yml".to_owned(), FileStatus::Added),
                ("b.yml".to_owned(), FileStatus::Added),
                ("old.yml".to_owned(), FileStatus::Deleted),
            ]
        );
    }

    #[test]
    fn a_worktree_copy_out_of_the_collection_lists_nothing() {
        assert!(files(&two(".C", "C100", "other/cp.yml", "col/old.yml")).is_empty());
    }

    #[test]
    fn unmerged_is_conflicted_and_question_marks_are_untracked() {
        assert_eq!(
            files(&unmerged("col/conf.yml")),
            file("conf.yml", FileStatus::Conflicted)
        );
        assert_eq!(
            files(&untracked("col/sub/new.yml")),
            file("sub/new.yml", FileStatus::Untracked)
        );
    }

    #[test]
    fn a_worktree_rename_is_the_new_path_added_and_the_origin_deleted() {
        assert_eq!(
            files(&two(".R", "R100", "col/new.yml", "col/old.yml")),
            [
                ("new.yml".to_owned(), FileStatus::Added),
                ("old.yml".to_owned(), FileStatus::Deleted)
            ]
        );
    }

    /// The files of `first` then `second`, after proving the order of the two does not matter.
    fn files_either_way(first: &str, second: &str) -> Vec<(String, FileStatus)> {
        let got = files(&format!("{first}{second}"));
        assert_eq!(got, files(&format!("{second}{first}")));
        got
    }

    #[test]
    fn a_worktree_rename_of_a_staged_rename_deletes_the_first_origin() {
        let staged = two("R.", "R100", "col/b.yml", "col/a.yml");
        let worktree = two(".R", "R100", "col/c.yml", "col/b.yml");
        assert_eq!(
            files_either_way(&staged, &worktree),
            [
                ("a.yml".to_owned(), FileStatus::Deleted),
                ("b.yml".to_owned(), FileStatus::Added),
                ("c.yml".to_owned(), FileStatus::Added),
            ]
        );
    }

    #[test]
    fn a_worktree_rename_of_a_staged_edit_keeps_it_modified() {
        let modified = one("M.", "col/b.yml");
        let worktree = two(".R", "R100", "col/c.yml", "col/b.yml");
        assert_eq!(
            files_either_way(&modified, &worktree),
            [
                ("b.yml".to_owned(), FileStatus::Modified),
                ("c.yml".to_owned(), FileStatus::Added),
            ]
        );
    }

    #[test]
    fn a_worktree_rename_of_a_staged_add_lists_both_paths() {
        let added = one("A.", "col/n.yml");
        let worktree = two(".R", "R100", "col/m.yml", "col/n.yml");
        assert_eq!(
            files_either_way(&added, &worktree),
            [
                ("m.yml".to_owned(), FileStatus::Added),
                ("n.yml".to_owned(), FileStatus::Added),
            ]
        );
        assert_eq!(
            stages(&format!("{added}{worktree}")),
            [
                ("m.yml".to_owned(), StageState::Unstaged),
                ("n.yml".to_owned(), StageState::StagedGone),
            ]
        );
    }

    #[test]
    fn a_worktree_rename_leaves_a_conflict_on_its_origin() {
        let conflict = unmerged("col/b.yml");
        let worktree = two(".R", "R100", "col/c.yml", "col/b.yml");
        assert_eq!(
            files_either_way(&conflict, &worktree),
            [
                ("b.yml".to_owned(), FileStatus::Conflicted),
                ("c.yml".to_owned(), FileStatus::Added),
            ]
        );
    }

    #[test]
    fn a_first_origin_back_on_disk_after_two_renames_is_modified() {
        let records = two("R.", "R100", "col/b.yml", "col/a.yml")
            + &two(".R", "R100", "col/c.yml", "col/b.yml")
            + &untracked("col/a.yml");
        assert_eq!(
            files(&records),
            [
                ("a.yml".to_owned(), FileStatus::Modified),
                ("b.yml".to_owned(), FileStatus::Added),
                ("c.yml".to_owned(), FileStatus::Added),
            ]
        );
        assert_eq!(stages(&records)[0].1, StageState::Both);
        assert_eq!(stages(&records)[1].1, StageState::StagedGone);
    }

    #[test]
    fn a_worktree_rename_out_of_the_collection_is_only_the_origin_deleted() {
        assert_eq!(
            files(&two(".R", "R100", "other/new.yml", "col/old.yml")),
            file("old.yml", FileStatus::Deleted)
        );
    }

    #[test]
    fn a_worktree_rename_into_the_collection_is_only_the_new_path_added() {
        assert_eq!(
            files(&two(".R", "R100", "col/new.yml", "other/old.yml")),
            file("new.yml", FileStatus::Added)
        );
    }

    #[test]
    fn an_untracked_nested_repository_has_no_trailing_slash() {
        assert_eq!(
            files(&untracked("col/nested/")),
            file("nested", FileStatus::Untracked)
        );
    }

    #[test]
    fn removed_from_the_index_but_still_on_disk_is_modified() {
        let records = one("D.", "col/a.yml") + &untracked("col/a.yml");
        assert_eq!(files(&records), file("a.yml", FileStatus::Modified));
    }

    #[test]
    fn submodules_and_paths_outside_the_collection_are_skipped() {
        let records = format!("1 .M S.M. 160000 160000 160000 {HASHES} col/lib\0")
            + &one(".M", "other/o.txt")
            + &one(".M", "colx/a.yml")
            + &untracked("other/new.txt");
        assert!(files(&records).is_empty());
    }

    #[test]
    fn an_empty_prefix_keeps_every_path() {
        let s = status(&(one(".M", "a.yml") + &untracked("d/b.yml")), "");
        let paths: Vec<PathBuf> = s.files.keys().cloned().collect();
        assert_eq!(paths, [PathBuf::from("a.yml"), PathBuf::from("d/b.yml")]);
    }

    #[test]
    fn paths_keep_spaces_quotes_and_newlines() {
        let records = one(".M", "col/sp ace \"q\".yml") + &untracked("col/new\nline.yml");
        let paths: Vec<String> = files(&records).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["new\nline.yml", "sp ace \"q\".yml"]);
    }

    /// `n` untracked files in reverse path order, so the cap must sort before cutting.
    fn many(n: usize) -> String {
        (0..n)
            .rev()
            .map(|i| untracked(&format!("col/f{i:05}.yml")))
            .collect()
    }

    #[test]
    fn the_cap_keeps_the_first_entries_in_path_order() {
        let at = status(&many(GIT_FILE_CAP), "col/");
        assert_eq!((at.files.len(), at.truncated), (GIT_FILE_CAP, false));
        let over = status(&many(GIT_FILE_CAP + 1), "col/");
        assert_eq!((over.files.len(), over.truncated), (GIT_FILE_CAP, true));
        assert_eq!(over.files.keys().next(), Some(&PathBuf::from("f00000.yml")));
        assert_eq!(
            over.files.keys().next_back(),
            Some(&PathBuf::from(format!("f{:05}.yml", GIT_FILE_CAP - 1)))
        );
    }

    #[test]
    fn malformed_output_is_an_error() {
        for bad in [
            String::new(),
            header() + "1 .M\0",
            header() + "X col/a.yml\0",
            header() + &format!("2 R. N... {MODES} {HASHES} R100 col/new.yml\0"),
        ] {
            assert!(
                matches!(
                    parse(bad.as_bytes(), "col/", GIT_FILE_CAP),
                    Err(GitError::Malformed(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn an_unlimited_cap_keeps_everything() {
        let all = parse(
            format!("{}{}", header(), many(GIT_FILE_CAP + 1)).as_bytes(),
            "col/",
            usize::MAX,
        )
        .expect("parses");
        assert_eq!((all.files.len(), all.truncated), (GIT_FILE_CAP + 1, false));
    }

    use crate::git::StageState;

    fn stages(records: &str) -> Vec<(String, StageState)> {
        status(records, "col/")
            .stage
            .into_iter()
            .map(|(path, s)| (path.to_string_lossy().into_owned(), s))
            .collect()
    }
    fn one_stage(path: &str, s: StageState) -> Vec<(String, StageState)> {
        vec![(path.to_owned(), s)]
    }

    #[test]
    fn ordinary_records_map_xy_to_a_stage_state() {
        use StageState::{Both, Staged, StagedGone, Unstaged};
        let cases = [
            ("M.", Staged),
            (".M", Unstaged),
            ("MM", Both),
            ("A.", Staged),
            ("AM", Both),
            ("D.", Staged),
            (".D", Unstaged),
            ("DD", Both),
            ("MD", StagedGone),
            ("TD", StagedGone),
            (".T", Unstaged),
            (".A", Unstaged),
            ("AD", StagedGone),
        ];
        for (xy, want) in cases {
            assert_eq!(
                stages(&one(xy, "col/a.yml")),
                one_stage("a.yml", want),
                "{xy}"
            );
        }
    }

    #[test]
    fn rename_records_follow_the_stored_path() {
        use StageState::{Both, Staged};
        let cases = [
            (
                two("R.", "R100", "col/new.yml", "col/old.yml"),
                "new.yml",
                Staged,
            ),
            (
                two("RM", "R90", "col/new.yml", "col/old.yml"),
                "new.yml",
                Both,
            ),
            (
                two("C.", "C75", "col/copy.yml", "col/orig.yml"),
                "copy.yml",
                Staged,
            ),
            (
                two("R.", "R100", "col/new.yml", "other/old.yml"),
                "new.yml",
                Staged,
            ),
        ];
        for (records, path, want) in cases {
            assert_eq!(stages(&records), one_stage(path, want), "{records:?}");
        }
        assert_eq!(
            stages(&two("RD", "R100", "col/new.yml", "col/old.yml")),
            [
                ("new.yml".to_owned(), StageState::StagedGone),
                ("old.yml".to_owned(), StageState::Staged),
            ]
        );
    }

    #[test]
    fn a_worktree_rename_is_unstaged_on_both_paths() {
        assert_eq!(
            stages(&two(".R", "R100", "col/new.yml", "col/old.yml")),
            [
                ("new.yml".to_owned(), StageState::Unstaged),
                ("old.yml".to_owned(), StageState::Unstaged)
            ]
        );
    }

    #[test]
    fn a_worktree_rename_keeps_the_staged_side_of_the_deleted_path() {
        use StageState::{Staged, StagedGone, Unstaged};
        let worktree = two(".R", "R100", "col/c.yml", "col/b.yml");
        let c = ("c.yml".to_owned(), Unstaged);
        for (staged, want) in [
            (
                two("R.", "R100", "col/b.yml", "col/a.yml"),
                vec![
                    ("a.yml".to_owned(), Staged),
                    ("b.yml".to_owned(), StagedGone),
                    c.clone(),
                ],
            ),
            (
                one("M.", "col/b.yml"),
                vec![("b.yml".to_owned(), StagedGone), c.clone()],
            ),
        ] {
            assert_eq!(stages(&format!("{staged}{worktree}")), want);
            assert_eq!(stages(&format!("{worktree}{staged}")), want);
        }
    }

    #[test]
    fn stage_keys_equal_file_keys_past_a_worktree_rename() {
        let worktree = two(".R", "R100", "col/c.yml", "col/b.yml");
        for other in [
            one("A.", "col/b.yml"),
            one("M.", "col/b.yml"),
            two("R.", "R100", "col/b.yml", "col/a.yml"),
            String::new(),
        ] {
            let s = status(&format!("{other}{worktree}"), "col/");
            assert!(s.files.keys().eq(s.stage.keys()), "{other:?}");
        }
    }

    #[test]
    fn unmerged_and_untracked_are_unstaged() {
        assert_eq!(
            stages(&unmerged("col/c.yml")),
            one_stage("c.yml", StageState::Unstaged)
        );
        assert_eq!(
            stages(&untracked("col/n.yml")),
            one_stage("n.yml", StageState::Unstaged)
        );
    }

    #[test]
    fn a_renamed_away_origin_back_on_disk_is_modified_and_both() {
        let records = two("RD", "R100", "col/new.yml", "col/old.yml") + &untracked("col/old.yml");
        assert_eq!(
            files(&records),
            [
                ("new.yml".to_owned(), FileStatus::Added),
                ("old.yml".to_owned(), FileStatus::Modified),
            ]
        );
        assert_eq!(
            stages(&records),
            [
                ("new.yml".to_owned(), StageState::StagedGone),
                ("old.yml".to_owned(), StageState::Both),
            ]
        );
    }

    #[test]
    fn a_nested_repository_on_a_gone_path_leaves_the_indexed_row() {
        let records = one("MD", "col/a.yml") + &untracked("col/a.yml/");
        assert_eq!(files(&records), file("a.yml", FileStatus::Modified));
        assert_eq!(stages(&records), one_stage("a.yml", StageState::StagedGone));
    }

    #[test]
    fn a_staged_deletion_with_the_file_back_on_disk_is_both() {
        let records = one("D.", "col/a.yml") + &untracked("col/a.yml");
        assert_eq!(stages(&records), one_stage("a.yml", StageState::Both));
    }

    #[test]
    fn stage_keys_equal_file_keys_through_the_cap_and_skips() {
        let s = status(&many(GIT_FILE_CAP + 1), "col/");
        assert!(s.truncated);
        assert!(s.files.keys().eq(s.stage.keys()));
        let skipped = status(
            &(one(".M", "other/o.txt") + &one("AD", "other/x.yml")),
            "col/",
        );
        assert!(skipped.files.is_empty() && skipped.stage.is_empty());
    }

    const OID0: &str = "0000000000000000000000000000000000000000";
    const EMPTY_SHA1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
    const EMPTY_SHA256: &str = "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813";

    fn gone_empty_as(hash: &str, path: &str) -> String {
        format!("1 .D N... 100644 100644 000000 {hash} {hash} {path}\0")
    }
    fn gone_empty(path: &str) -> String {
        gone_empty_as(EMPTY_SHA1, path)
    }
    fn head_of(paths: &[&str]) -> HashSet<Vec<u8>> {
        paths.iter().map(|p| p.as_bytes().to_vec()).collect()
    }
    /// The status of `records` with `lookup` answering for `HEAD`'s paths.
    fn looked_up(records: &str, lookup: Option<HashSet<Vec<u8>>>) -> RepoStatus {
        let out = format!("{}{records}", header());
        parse_with(out.as_bytes(), "col/", GIT_FILE_CAP, || lookup).expect("parses")
    }

    #[test]
    fn an_empty_deletion_missing_from_head_is_not_listed() {
        for hash in [EMPTY_SHA1, EMPTY_SHA256] {
            let records = gone_empty_as(hash, "col/a.yml") + &untracked("col/b.yml");
            let s = looked_up(&records, Some(head_of(&["col/other.yml"])));
            assert_eq!(
                s.files.into_iter().collect::<Vec<_>>(),
                [(PathBuf::from("b.yml"), FileStatus::Untracked)]
            );
            assert_eq!(s.stage.len(), 1);
        }
    }

    #[test]
    fn an_empty_deletion_found_in_head_stays_deleted() {
        let s = looked_up(&gone_empty("col/a.yml"), Some(head_of(&["col/a.yml"])));
        assert_eq!(s.files[&PathBuf::from("a.yml")], FileStatus::Deleted);
        assert_eq!(s.stage[&PathBuf::from("a.yml")], StageState::Unstaged);
    }

    #[test]
    fn head_is_matched_by_repository_path() {
        let s = looked_up(&gone_empty("col/a.yml"), Some(head_of(&["a.yml"])));
        assert!(s.files.is_empty(), "{s:?}");
    }

    #[test]
    fn a_failed_lookup_keeps_the_row() {
        let s = looked_up(&gone_empty("col/a.yml"), None);
        assert_eq!(s.files[&PathBuf::from("a.yml")], FileStatus::Deleted);
    }

    #[test]
    fn an_unborn_branch_hides_every_candidate_without_a_lookup() {
        let out = format!(
            "# branch.oid (initial)\0# branch.head main\0{}{}",
            gone_empty("col/a.yml"),
            gone_empty("col/b.yml")
        );
        let s = parse(out.as_bytes(), "col/", GIT_FILE_CAP).expect("parses");
        assert!(s.files.is_empty() && s.stage.is_empty(), "{s:?}");
    }

    #[test]
    fn no_lookup_runs_without_a_candidate_in_the_collection() {
        // The deletion of real content, one outside the collection, and a staged one.
        let records = one(".D", "col/a.yml")
            + &gone_empty("other/o.yml")
            + &format!("1 D. N... 100644 000000 000000 {EMPTY_SHA1} {EMPTY_SHA1} col/s.yml\0");
        let paths: Vec<String> = files(&records).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["a.yml", "s.yml"]);
    }

    #[test]
    fn hidden_rows_do_not_count_against_the_cap() {
        let records = gone_empty("col/a.yml") + &untracked("col/b.yml") + &untracked("col/c.yml");
        let out = format!("{}{records}", header());
        let s = parse_with(out.as_bytes(), "col/", 2, || Some(HashSet::new())).expect("parses");
        assert!(!s.truncated, "{s:?}");
        assert_eq!(s.files.len(), 2);
        assert!(s.files.keys().eq(s.stage.keys()));
    }

    /// A worktree rename whose origin's index entry is the empty blob.
    fn moved_empty(path: &str, from: &str) -> String {
        format!("2 .R N... {MODES} {EMPTY_SHA1} {EMPTY_SHA1} R100 {path}\0{from}\0")
    }

    #[test]
    fn an_empty_worktree_rename_from_outside_head_lists_only_the_new_path() {
        let s = looked_up(&moved_empty("col/o", "col/n"), Some(HashSet::new()));
        assert_eq!(
            s.files.into_iter().collect::<Vec<_>>(),
            [(PathBuf::from("o"), FileStatus::Added)]
        );
        assert_eq!(s.stage.len(), 1);
    }

    #[test]
    fn an_empty_worktree_rename_from_head_keeps_the_deleted_origin() {
        let s = looked_up(&moved_empty("col/o", "col/n"), Some(head_of(&["col/n"])));
        assert_eq!(s.files[&PathBuf::from("n")], FileStatus::Deleted);
        assert_eq!(s.files[&PathBuf::from("o")], FileStatus::Added);
    }

    #[test]
    fn a_nested_repository_on_a_candidate_is_not_hidden() {
        let records = gone_empty("col/n") + &untracked("col/n/");
        let s = looked_up(&records, Some(HashSet::new()));
        assert_eq!(s.files[&PathBuf::from("n")], FileStatus::Modified);
    }

    #[test]
    fn the_stage_map_is_not_serialized() {
        let s = status(&one("M.", "col/a.yml"), "col/");
        let json = serde_json::to_value(&s).unwrap();
        assert!(json.get("stage").is_none(), "{json}");
        assert_eq!(json["files"]["a.yml"]["kind"], "modified");
    }
}
