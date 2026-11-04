//! A file's diff as the Changes dialog shows it: the changed sections by editor tab name, then
//! numbered rows under hunk headers, or a note in place of the rows.

use std::path::Path;

use apinata_core::{FileDiff, GitError, LineKind};

/// The note shown for a changed file that is not a request file.
const NOT_A_REQUEST: &str = "Only request files (.yml, .yaml) show their changes here.";

/// The text of the note under a line that ends its file without a newline.
const NO_NEWLINE: &str = "\\ No newline at end of file";

/// The indices of the lines in `lines` that end their file without a newline.
pub fn unterminated(lines: &[apinata_core::DiffLine]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.no_newline)
        .map(|(i, _)| i)
        .collect()
}

/// What one display row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A hunk's `@@` header.
    Header,
    /// A line present on both sides.
    Context,
    /// A line only in the working tree.
    Added,
    /// A line only in `HEAD`.
    Removed,
    /// Git's "\ No newline at end of file" note under the line it describes.
    NoNewline,
}

impl RowKind {
    /// The gutter mark that carries the row's meaning without colour.
    pub fn gutter(self) -> &'static str {
        match self {
            RowKind::Header | RowKind::NoNewline => "",
            RowKind::Context => " ",
            RowKind::Added => "+",
            RowKind::Removed => "-",
        }
    }
}

/// One display row: its kind, its line number on each side, and its screen-safe text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRow {
    /// What the row is.
    pub kind: RowKind,
    /// The line number in `HEAD`, for context and removed rows.
    pub old: Option<usize>,
    /// The line number in the working tree, for context and added rows.
    pub new: Option<usize>,
    /// The text to render, already passed through [`display_text`].
    pub text: String,
}

/// An app-owned copy of a core hunk: 1-based starts (0 for an empty side) and its lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    /// The first old line number.
    pub old_start: usize,
    /// The first new line number.
    pub new_start: usize,
    /// Each line's kind and text, without its trailing newline.
    pub lines: Vec<(LineKind, String)>,
    /// Indices into `lines` of the lines that end their file without a newline.
    pub unterminated: Vec<usize>,
}

/// What the diff pane shows below the section list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffBody {
    /// Numbered rows under hunk headers.
    Lines(Vec<DiffRow>),
    /// Either side is binary.
    Binary,
    /// Neither side's text differs.
    NoContentChange,
    /// The diff could not be shown; the text says why.
    Unavailable(String),
}

/// A file's diff ready to render.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffDoc {
    /// The changed parts of the request, by editor tab name, in core's order.
    pub sections: Vec<&'static str>,
    /// The rows, or the note that replaces them.
    pub body: DiffBody,
}

impl DiffDoc {
    /// The display form of a core diff: binary or hunk-less diffs become notes.
    pub fn from_diff(diff: &FileDiff) -> DiffDoc {
        let body = if diff.binary {
            DiffBody::Binary
        } else if diff.hunks.is_empty() {
            DiffBody::NoContentChange
        } else {
            let hunks: Vec<DiffHunk> = diff
                .hunks
                .iter()
                .map(|h| DiffHunk {
                    old_start: h.old_start,
                    new_start: h.new_start,
                    lines: h.lines.iter().map(|l| (l.kind, l.text.clone())).collect(),
                    unterminated: unterminated(&h.lines),
                })
                .collect();
            DiffBody::Lines(rows(&hunks))
        };
        DiffDoc {
            sections: diff.sections.iter().copied().map(section_label).collect(),
            body,
        }
    }

    /// A note for a diff that failed; a failed `git show` means the committed blob is not local.
    pub fn from_error(error: &GitError) -> DiffDoc {
        let note = match error {
            GitError::Failed { command, .. } if command.starts_with("git show") => {
                "The committed version is not available locally.".to_owned()
            }
            other => format!("Could not read the changes: {other}"),
        };
        DiffDoc {
            sections: Vec::new(),
            body: DiffBody::Unavailable(note),
        }
    }

    /// The fixed note for a changed file whose diff is not requested.
    pub fn not_a_request() -> DiffDoc {
        DiffDoc {
            sections: Vec::new(),
            body: DiffBody::Unavailable(NOT_A_REQUEST.to_owned()),
        }
    }

    /// The note shown in place of rows, or `None` when there are rows.
    pub fn note(&self) -> Option<&str> {
        match &self.body {
            DiffBody::Lines(_) => None,
            DiffBody::Binary => Some("Binary file changed"),
            DiffBody::NoContentChange => Some("No content change"),
            DiffBody::Unavailable(note) => Some(note),
        }
    }
}

/// Whether core can diff this collection-relative path: its extension is `yml` or `yaml`.
pub fn is_request_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|x| x.to_str()),
        Some("yml" | "yaml")
    )
}

/// Display rows for hunks: a `@@ -o,oc +n,nc @@` header each (counts always printed), then each
/// line numbered on the sides it exists on.
pub fn rows(hunks: &[DiffHunk]) -> Vec<DiffRow> {
    let mut out = Vec::new();
    for hunk in hunks {
        let on_old = |k: &LineKind| !matches!(k, LineKind::Added);
        let on_new = |k: &LineKind| !matches!(k, LineKind::Removed);
        let old_count = hunk.lines.iter().filter(|(k, _)| on_old(k)).count();
        let new_count = hunk.lines.iter().filter(|(k, _)| on_new(k)).count();
        out.push(DiffRow {
            kind: RowKind::Header,
            old: None,
            new: None,
            text: format!(
                "@@ -{},{old_count} +{},{new_count} @@",
                hunk.old_start, hunk.new_start
            ),
        });
        let (mut old, mut new) = (hunk.old_start, hunk.new_start);
        for (at, (kind, text)) in hunk.lines.iter().enumerate() {
            // A kind added to core later counts as context, present on both sides.
            let kind = match kind {
                LineKind::Added => RowKind::Added,
                LineKind::Removed => RowKind::Removed,
                _ => RowKind::Context,
            };
            let old_no = (kind != RowKind::Added).then(|| {
                old += 1;
                old - 1
            });
            let new_no = (kind != RowKind::Removed).then(|| {
                new += 1;
                new - 1
            });
            out.push(DiffRow {
                kind,
                old: old_no,
                new: new_no,
                text: display_text(text),
            });
            if hunk.unterminated.contains(&at) {
                out.push(DiffRow {
                    kind: RowKind::NoNewline,
                    old: None,
                    new: None,
                    text: NO_NEWLINE.to_owned(),
                });
            }
        }
    }
    out
}

/// A line made safe to render: one trailing `\r` (a CRLF file) is dropped and any other `\r`
/// shows as the two characters `\r`.
pub fn display_text(line: &str) -> String {
    line.strip_suffix('\r').unwrap_or(line).replace('\r', "\\r")
}

/// The request editor's tab name for a core section; a section added to core later is "Other".
pub fn section_label(section: apinata_core::Section) -> &'static str {
    use apinata_core::Section;
    match section {
        Section::Info => "Info",
        Section::Method => "Method",
        Section::Url => "URL",
        Section::Params => "Params",
        Section::Headers => "Headers",
        Section::Body => "Body",
        Section::Auth => "Auth",
        Section::Vars => "Vars",
        Section::Scripts => "Script",
        Section::Assertions => "Assertions",
        Section::Actions => "Actions",
        Section::Docs => "Docs",
        Section::Settings => "Settings",
        Section::Examples => "Examples",
        _ => "Other",
    }
}
