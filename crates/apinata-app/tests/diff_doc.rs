//! Display rows for a file's diff: numbering, hunk headers, carriage returns, section labels,
//! and the notes that replace lines.

use crate::git_support;
use crate::support;

use std::path::Path;

use apinata_app::diff_doc::{
    DiffBody, DiffDoc, DiffHunk, DiffRow, RowKind, display_text, is_request_file, rows,
    section_label,
};
use apinata_core::{GitError, LineKind, Section, file_diff_with};
use git_support::{Sandbox, write};
use support::*;

fn row(kind: RowKind, old: Option<usize>, new: Option<usize>, text: &str) -> DiffRow {
    DiffRow {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

fn lines(doc: &DiffDoc) -> &[DiffRow] {
    match &doc.body {
        DiffBody::Lines(rows) => rows,
        other => panic!("expected lines, got {other:?}"),
    }
}

/// A committed collection holding `a.yml` with url `http://x/a`.
fn committed_repo(sb: &Sandbox) -> std::path::PathBuf {
    let repo = sb.repo("r");
    write(&repo, "opencollection.yml", ROOT);
    write(&repo, "a.yml", &request("a", "http://x/a"));
    sb.commit_all(&repo, "init");
    repo
}

#[test]
fn rows_number_both_sides_under_a_hunk_header() {
    let hunk = DiffHunk {
        old_start: 2,
        new_start: 2,
        lines: vec![
            (LineKind::Context, "b".into()),
            (LineKind::Removed, "c".into()),
            (LineKind::Added, "C".into()),
            (LineKind::Added, "D".into()),
            (LineKind::Context, "e".into()),
        ],
        unterminated: vec![],
    };
    assert_eq!(
        rows(&[hunk]),
        vec![
            row(RowKind::Header, None, None, "@@ -2,3 +2,4 @@"),
            row(RowKind::Context, Some(2), Some(2), "b"),
            row(RowKind::Removed, Some(3), None, "c"),
            row(RowKind::Added, None, Some(3), "C"),
            row(RowKind::Added, None, Some(4), "D"),
            row(RowKind::Context, Some(4), Some(5), "e"),
        ]
    );
}

#[test]
fn an_empty_side_reports_zero_lines() {
    let added = DiffHunk {
        old_start: 0,
        new_start: 1,
        lines: vec![(LineKind::Added, "x".into()), (LineKind::Added, "y".into())],
        unterminated: vec![],
    };
    let removed = DiffHunk {
        old_start: 1,
        new_start: 0,
        lines: vec![(LineKind::Removed, "x".into())],
        unterminated: vec![],
    };
    assert_eq!(
        rows(&[added, removed]),
        vec![
            row(RowKind::Header, None, None, "@@ -0,0 +1,2 @@"),
            row(RowKind::Added, None, Some(1), "x"),
            row(RowKind::Added, None, Some(2), "y"),
            row(RowKind::Header, None, None, "@@ -1,1 +0,0 @@"),
            row(RowKind::Removed, Some(1), None, "x"),
        ]
    );
}

#[test]
fn carriage_returns_never_reach_the_screen() {
    assert_eq!(display_text("url: x\r"), "url: x");
    assert_eq!(display_text("a\rb"), "a\\rb");
    assert_eq!(display_text("plain"), "plain");
    let hunk = DiffHunk {
        old_start: 1,
        new_start: 1,
        lines: vec![(LineKind::Context, "a\r".into())],
        unterminated: vec![],
    };
    assert_eq!(rows(&[hunk])[1].text, "a");
}

#[test]
fn gutters_carry_the_meaning_without_colour() {
    let marks: Vec<&str> = [
        RowKind::Header,
        RowKind::Context,
        RowKind::Added,
        RowKind::Removed,
    ]
    .into_iter()
    .map(RowKind::gutter)
    .collect();
    assert_eq!(marks, ["", " ", "+", "-"]);
}

#[test]
fn sections_use_the_editor_tab_names() {
    let labels: Vec<&str> = [
        Section::Info,
        Section::Method,
        Section::Url,
        Section::Params,
        Section::Headers,
        Section::Body,
        Section::Auth,
        Section::Vars,
        Section::Scripts,
        Section::Assertions,
        Section::Actions,
        Section::Docs,
        Section::Settings,
        Section::Examples,
        Section::Other,
    ]
    .into_iter()
    .map(section_label)
    .collect();
    assert_eq!(
        labels,
        [
            "Info",
            "Method",
            "URL",
            "Params",
            "Headers",
            "Body",
            "Auth",
            "Vars",
            "Script",
            "Assertions",
            "Actions",
            "Docs",
            "Settings",
            "Examples",
            "Other"
        ]
    );
}

#[test]
fn errors_become_notes() {
    let show = GitError::Failed {
        command: "git show HEAD:a.yml".into(),
        stderr: "fatal: bad object HEAD:a.yml".into(),
    };
    let doc = DiffDoc::from_error(&show);
    assert_eq!(
        doc.note(),
        Some("The committed version is not available locally.")
    );
    assert!(doc.sections.is_empty());
    assert_eq!(
        DiffDoc::from_error(&GitError::Timeout).note(),
        Some("Could not read the changes: git did not finish within the time limit")
    );
}

#[test]
fn only_yaml_files_are_diffed_and_the_rest_get_a_fixed_note() {
    assert!(is_request_file(Path::new("a.yml")));
    assert!(is_request_file(Path::new("users/folder.yaml")));
    assert!(!is_request_file(Path::new("README.md")));
    assert!(!is_request_file(Path::new(".gitignore")));
    assert!(!is_request_file(Path::new("yml")));
    let doc = DiffDoc::not_a_request();
    assert_eq!(
        doc.note(),
        Some("Only request files (.yml, .yaml) show their changes here.")
    );
    assert!(doc.sections.is_empty());
}

#[test]
fn a_modified_request_lists_its_sections_then_its_lines() {
    let sb = Sandbox::new();
    let repo = committed_repo(&sb);
    write(&repo, "a.yml", &request("a", "http://x/b"));
    let diff = file_diff_with(&repo, Path::new("a.yml"), &sb.options()).unwrap();
    let doc = DiffDoc::from_diff(&diff);
    assert_eq!(doc.sections, ["URL"]);
    assert_eq!(doc.note(), None);
    let changed: Vec<(RowKind, &str)> = lines(&doc)
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Added | RowKind::Removed))
        .map(|r| (r.kind, r.text.as_str()))
        .collect();
    assert_eq!(
        changed,
        [
            (RowKind::Removed, "  url: \"http://x/a\""),
            (RowKind::Added, "  url: \"http://x/b\""),
        ]
    );
}

#[test]
fn a_deleted_request_is_all_removed_lines() {
    let sb = Sandbox::new();
    let repo = committed_repo(&sb);
    std::fs::remove_file(repo.join("a.yml")).unwrap();
    let diff = file_diff_with(&repo, Path::new("a.yml"), &sb.options()).unwrap();
    let doc = DiffDoc::from_diff(&diff);
    let body: Vec<RowKind> = lines(&doc)
        .iter()
        .map(|r| r.kind)
        .filter(|k| *k != RowKind::Header)
        .collect();
    assert!(!body.is_empty());
    assert!(body.iter().all(|k| *k == RowKind::Removed), "{body:?}");
}

#[test]
fn a_binary_file_and_an_unchanged_text_get_notes() {
    let sb = Sandbox::new();
    let repo = committed_repo(&sb);
    std::fs::write(repo.join("a.yml"), [0xffu8, 0xfe, b'\n']).unwrap();
    let diff = file_diff_with(&repo, Path::new("a.yml"), &sb.options()).unwrap();
    assert_eq!(
        DiffDoc::from_diff(&diff).note(),
        Some("Binary file changed")
    );

    let sb = Sandbox::new();
    let repo = committed_repo(&sb);
    sb.git(&repo, &["rm", "--cached", "-q", "a.yml"]);
    let diff = file_diff_with(&repo, Path::new("a.yml"), &sb.options()).unwrap();
    assert!(diff.status.is_some());
    assert_eq!(DiffDoc::from_diff(&diff).note(), Some("No content change"));
}

#[test]
fn a_file_without_a_status_still_shows_its_lines() {
    let sb = Sandbox::new();
    let repo = committed_repo(&sb);
    write(&repo, ".gitignore", "ignored.yml\n");
    sb.commit_all(&repo, "ignore");
    write(&repo, "ignored.yml", &request("i", "http://x/i"));
    let diff = file_diff_with(&repo, Path::new("ignored.yml"), &sb.options()).unwrap();
    assert_eq!(diff.status, None);
    let doc = DiffDoc::from_diff(&diff);
    assert!(lines(&doc).iter().any(|r| r.kind == RowKind::Added));
}
