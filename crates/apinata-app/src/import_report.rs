//! The import report: a summary and the notes an import left, grouped by kind, shown in a
//! dialog after the import. The export report shows its notes in the same dialog.

use crate::style::{
    DIALOG_FOOTER_GAP, ROW_HEIGHT, SCROLLBAR_GUTTER, SPACE_1, SPACE_2, SPACE_3, resolve,
};
use crate::ui::{dialog_footer, form_heading, plain_quotes};
use apinata_core::import::{ImportNote, ImportReport, NoteKind};
use gpui_kit::component::WindowExt;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    App, InteractiveElement, ParentElement, ScrollHandle, StatefulInteractiveElement as _, Styled,
    TestSupportExt, Window, div, px,
};
use std::path::Path;

/// One line per note in note order: "<source>: <detail>", plus " (<file>)" when set, with
/// every backtick removed.
pub fn lines(report: &ImportReport) -> Vec<String> {
    report.notes.iter().map(line).collect()
}

/// "<items>, <environments> imported; <notes>." with each count singular or plural.
pub fn summary(report: &ImportReport) -> String {
    format!(
        "{}, {} imported; {}.",
        count(report.items.len(), "item"),
        count(report.environments.len(), "environment"),
        count(report.notes.len(), "note")
    )
}

/// The non-empty note groups, each a plain heading with its notes' lines in note order:
/// "Left out", "Changed", "Renamed", "Needs a value", then "Other".
pub fn groups(report: &ImportReport) -> Vec<(&'static str, Vec<String>)> {
    HEADINGS
        .iter()
        .filter_map(|&heading| {
            let lines: Vec<String> = report
                .notes
                .iter()
                .filter(|n| self::heading(&n.kind) == heading)
                .map(line)
                .collect();
            (!lines.is_empty()).then_some((heading, lines))
        })
        .collect()
}

/// `message` with its first character upper-cased, for an alert; core's errors start lowercase.
pub fn failure_text(message: &str) -> String {
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// An alert line for a failure tied to a file: the error first, then the path in parentheses.
/// Only the error gets plain quotes; the path is shown as it is.
pub fn failure_at(path: &Path, error: &str) -> String {
    format!(
        "{} ({})",
        plain_quotes(&failure_text(error)),
        path.display()
    )
}

/// The alert line for a collection at `root` that could not open. Core's open errors already
/// name the path, so a leading "<path>: <error>" is reordered into [`failure_at`]'s shape and
/// any other mention of the root is kept in place; a message without it gets the root appended.
pub fn open_failure(root: &Path, message: &str) -> String {
    let shown = root.display().to_string();
    // The path ends at the first ": " when it is the root itself or a file under it.
    if let Some(rest) = message.strip_prefix(&shown)
        && (rest.starts_with(':') || rest.starts_with(std::path::MAIN_SEPARATOR))
        && let Some(at) = rest.find(": ")
    {
        let path = Path::new(&message[..shown.len() + at]);
        return failure_at(path, &rest[at + 2..]);
    }
    if message.contains(&shown) {
        let parts: Vec<String> = message.split(&shown).map(plain_quotes).collect();
        return failure_text(&parts.join(&shown));
    }
    failure_at(root, message)
}

/// A dialog titled `title`: the summary ("import-summary"), each group's heading
/// ("import-group", g) and notes ("import-note", ix counted across groups), then a Close button
/// ("import-report-close").
pub fn open(title: &'static str, report: &ImportReport, window: &mut Window, cx: &mut App) {
    let groups = groups(report)
        .into_iter()
        .map(|(heading, lines)| (Some(heading), lines))
        .collect();
    open_report(title, IMPORT_IDS, summary(report), groups, window, cx);
}

/// The element ids of one kind of report dialog.
#[derive(Clone, Copy)]
pub struct ReportIds {
    /// The summary line.
    pub summary: &'static str,
    /// Each group's heading, numbered.
    pub group: &'static str,
    /// Each note, numbered across groups.
    pub note: &'static str,
    /// The scrolling box around the notes.
    pub scroll: &'static str,
    /// The Close button.
    pub close: &'static str,
}

const IMPORT_IDS: ReportIds = ReportIds {
    summary: "import-summary",
    group: "import-group",
    note: "import-note",
    scroll: "import-report-scroll",
    close: "import-report-close",
};

/// A report dialog titled `title`: `summary`, then each group's optional heading and its lines
/// in a box that scrolls when they outgrow the window, then a Close button.
pub fn open_report(
    title: &'static str,
    ids: ReportIds,
    summary: String,
    groups: Vec<(Option<&'static str>, Vec<String>)>,
    window: &mut Window,
    cx: &mut App,
) {
    let scroll = ScrollHandle::new();
    window.open_dialog(cx, move |d, window, _| {
        let d = crate::dialog_frame::place(d, window);
        // Note ids count across groups, so each group starts where the previous one ended.
        let mut first = 0usize;
        let sections = groups.iter().enumerate().map(|(g, (heading, lines))| {
            let start = first;
            first += lines.len();
            let notes = lines.iter().enumerate().map(move |(i, l)| {
                div()
                    .id((ids.note, start + i))
                    .test_support()
                    .aria_label(l.clone())
                    .child(l.clone())
            });
            div()
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .children(heading.map(|h| form_heading((ids.group, g), h)))
                .children(notes)
        });
        // The notes scroll inside the room above the status bar, less a row each for the
        // summary and the Close button, and the footer's gap and ring room, however many notes
        // there are.
        let room = crate::dialog_frame::titled_body_room(window)
            - resolve(ROW_HEIGHT, window) * 2.
            - resolve(DIALOG_FOOTER_GAP, window)
            - crate::style::FOCUS_RING_ROOM;
        d.title(title)
            .close_button(false)
            .child(
                div()
                    .id(ids.summary)
                    .test_support()
                    .aria_label(summary.clone())
                    .mb(SPACE_2)
                    .child(summary.clone()),
            )
            .child(
                // The capped box scrolls itself: the kit's scrollable fills its box by
                // percentage, which a box capped only by a maximum never resolves.
                div()
                    .relative()
                    .child(
                        div()
                            .id(ids.scroll)
                            .test_support()
                            .max_h(room.max(px(0.)))
                            .pr(SCROLLBAR_GUTTER)
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .child(div().flex().flex_col().gap(SPACE_3).children(sections)),
                    )
                    .vertical_scrollbar(&scroll),
            )
            .child(dialog_footer(
                None,
                [],
                Button::new(ids.close)
                    .label("Close")
                    .primary()
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            ))
    });
}

const HEADINGS: [&str; 5] = ["Left out", "Changed", "Renamed", "Needs a value", "Other"];

fn heading(kind: &NoteKind) -> &'static str {
    match kind {
        NoteKind::Dropped => "Left out",
        NoteKind::Converted => "Changed",
        NoteKind::Renamed => "Renamed",
        NoteKind::NeedsInput => "Needs a value",
        _ => "Other",
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn line(note: &ImportNote) -> String {
    let mut line = format!("{}: {}", note.source, note.detail);
    if let Some(file) = &note.file {
        line.push_str(&format!(" ({file})"));
    }
    line.replace('`', "")
}
