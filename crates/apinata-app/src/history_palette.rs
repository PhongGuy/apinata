//! The History command: a palette of recent sends across the open collections, newest
//! first. Choosing a send opens its request's tab showing that send.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use apinata_core::ItemPath;

use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, App, AppContext as _, FocusHandle, Focusable as _, HighlightStyle,
    InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, StyledText, TestSupportExt as _, Window, actions, div, relative,
};

use crate::last_response::{RecentSend, age_of};
use crate::palette::{RowsKey, request_entries};
use crate::style;
use crate::workspace::Workspace;

actions!(
    apinata,
    [
        /// Lists recent sends across the open collections.
        OpenHistory
    ]
);

/// Opens a request's tab showing one of its kept sends; the Shell handles it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = apinata, no_json)]
pub struct OpenHistoryEntry {
    /// The root of the request's collection.
    pub root: PathBuf,
    /// The request's path in its collection.
    pub path: ItemPath,
    /// The send to show.
    pub seq: u64,
}

/// One row of the History palette.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    /// The root of the request's collection.
    pub root: PathBuf,
    /// The request's path in its collection.
    pub path: ItemPath,
    /// The send's number within its request.
    pub seq: u64,
    /// The request's name; the search matches inside it.
    pub label: String,
    /// The send's status and age.
    pub detail: String,
    /// `KeptSend::target` of the send.
    pub target: String,
    /// A muted mark after the target: [`UNSAVED_TAG`] or [`RUN_TAG`].
    pub tag: Option<&'static str>,
    /// The method sent, "" when unknown.
    pub method: String,
    /// The URL sent to as stored, secrets already scrubbed.
    pub url: String,
    /// The response status.
    pub status: u16,
}

/// A row the History search keeps, with the byte ranges of its label, target and detail
/// to highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryMatch {
    /// The row's place in the searched entries.
    pub index: usize,
    /// Ranges in [`HistoryEntry::label`].
    pub name: Vec<Range<usize>>,
    /// Ranges in [`HistoryEntry::target`].
    pub target: Vec<Range<usize>>,
    /// Ranges in [`HistoryEntry::detail`].
    pub detail: Vec<Range<usize>>,
}

// The method words a search term names; such a term never matches inside a URL.
const METHODS: [&str; 9] = [
    "get", "post", "put", "patch", "delete", "head", "options", "trace", "connect",
];

// One whitespace-separated search term, lowercased.
enum Term {
    // `4xx`: a status class, by its first digit.
    Class(u16),
    // A method word: the method exactly, or inside the name.
    Method(String),
    // Anything else: inside the name, the URL, the method or the status digits.
    Text(String),
}

impl Term {
    fn parse(raw: &str) -> Term {
        // Folded as the searched text is, so both sides agree char for char.
        let lower = Folded::new(raw).lower;
        if let [d @ b'1'..=b'5', b'x', b'x'] = lower.as_bytes() {
            return Term::Class(u16::from(d - b'0'));
        }
        if METHODS.contains(&lower.as_str()) {
            Term::Method(lower)
        } else {
            Term::Text(lower)
        }
    }
}

/// The entries matching every whitespace-separated term of `query`, case-insensitively, in
/// order and at most [`HISTORY_MAX`], with what to highlight. Only the name, method, stored
/// URL and status are searched; an empty query keeps the first rows, unhighlighted.
pub fn filter_history(entries: &[HistoryEntry], query: &str) -> Vec<HistoryMatch> {
    let terms: Vec<Term> = query.split_whitespace().map(Term::parse).collect();
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, e)| match_entry(index, e, &terms))
        .take(HISTORY_MAX)
        .collect()
}

/// The History palette's search over its entries, remembering the last query it answered
/// so a redraw or a confirm with the same query does not search again.
pub struct HistorySearch {
    entries: Rc<[HistoryEntry]>,
    last: RefCell<(String, Rc<[HistoryMatch]>)>,
}

impl HistorySearch {
    /// A search over `entries`, primed for the empty query.
    pub fn new(entries: Vec<HistoryEntry>) -> HistorySearch {
        let entries: Rc<[HistoryEntry]> = entries.into();
        let all = filter_history(&entries, "").into();
        HistorySearch {
            entries,
            last: RefCell::new((String::new(), all)),
        }
    }

    /// The searched entries, which [`HistoryMatch::index`] points into.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// The matches for `query`, from the cache when it last answered `query`.
    pub fn matches(&self, query: &str) -> Rc<[HistoryMatch]> {
        let mut last = self.last.borrow_mut();
        if last.0 != query {
            *last = (
                query.to_owned(),
                filter_history(&self.entries, query).into(),
            );
        }
        last.1.clone()
    }
}

// A field lowercased for search, with the byte range in the original that each lowercased
// byte came from; `None` for ASCII, where the offsets are the same.
struct Folded {
    lower: String,
    from: Option<Vec<(usize, usize)>>,
}

impl Folded {
    fn new(text: &str) -> Folded {
        if text.is_ascii() {
            return Folded {
                lower: text.to_ascii_lowercase(),
                from: None,
            };
        }
        let mut lower = String::with_capacity(text.len());
        let mut from = Vec::with_capacity(text.len());
        for (at, c) in text.char_indices() {
            // A final sigma folds to the plain one, as Unicode case folding does.
            lower.extend(c.to_lowercase().map(|l| if l == 'ς' { 'σ' } else { l }));
            from.resize(lower.len(), (at, at + c.len_utf8()));
        }
        Folded {
            lower,
            from: Some(from),
        }
    }

    // Where `needle`, already lowercase, first occurs, as a range of the original text.
    fn find(&self, needle: &str) -> Option<Range<usize>> {
        let at = self.lower.find(needle)?;
        let end = at + needle.len();
        Some(match &self.from {
            None => at..end,
            Some(from) => from[at].0..from[end - 1].1,
        })
    }
}

fn match_entry(index: usize, e: &HistoryEntry, terms: &[Term]) -> Option<HistoryMatch> {
    let mut m = HistoryMatch {
        index,
        name: Vec::new(),
        target: Vec::new(),
        detail: Vec::new(),
    };
    if terms.is_empty() {
        return Some(m);
    }
    // A draft's name is its target, so its name is searched as method and URL only, and
    // their highlights land in the name.
    let name_is_target = e.target.is_empty() && !e.url.is_empty();
    let name = (!name_is_target).then(|| Folded::new(&e.label));
    let (url, method) = (Folded::new(&e.url), Folded::new(&e.method));
    let status = e.status.to_string();
    let url_at = if e.method.is_empty() {
        0
    } else {
        e.method.len() + 1
    };
    let shown_status = e.detail.starts_with(&status);
    let status_class = e.status / 100;
    for term in terms {
        let (method_hit, url_hit, status_hit, name_hit) = match term {
            Term::Class(class) => (
                None,
                None,
                (status_class == *class).then_some(0..status.len()),
                None,
            ),
            Term::Method(word) => (
                (method.lower == *word).then_some(0..e.method.len()),
                None,
                None,
                name.as_ref().and_then(|n| n.find(word)),
            ),
            Term::Text(text) => (
                method.find(text),
                url.find(text),
                status.find(text.as_str()).map(|at| at..at + text.len()),
                name.as_ref().and_then(|n| n.find(text)),
            ),
        };
        if method_hit.is_none() && url_hit.is_none() && status_hit.is_none() && name_hit.is_none() {
            return None;
        }
        let shown = if name_is_target {
            &mut m.name
        } else {
            &mut m.target
        };
        // The target shows only with a URL, so the method highlights only then.
        shown.extend(method_hit.filter(|_| !e.url.is_empty()));
        shown.extend(url_hit.map(|r| r.start + url_at..r.end + url_at));
        m.detail.extend(status_hit.filter(|_| shown_status));
        m.name.extend(name_hit);
    }
    for ranges in [&mut m.name, &mut m.target, &mut m.detail] {
        merge(ranges);
    }
    Some(m)
}

// Sorts `ranges` and joins the ones that overlap or touch, as highlights must not overlap.
fn merge(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges.drain(..) {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    *ranges = merged;
}

/// The tag of a send from an unsaved draft.
pub const UNSAVED_TAG: &str = "Unsaved";

/// The tag of a send made by the collection runner.
pub const RUN_TAG: &str = "Run";

// The label of a draft send that kept no target.
const UNTITLED: &str = "Untitled request";

/// The most sends the History palette lists.
pub const HISTORY_MAX: usize = 100;

/// Shown when an unsaved draft's send has no request snapshot left to reopen.
pub const DRAFT_GONE: &str = "This request can no longer be opened.";

/// The palette rows for `recent`, in order, without sends of requests no open collection has,
/// at most [`HISTORY_MAX`] of them.
pub fn history_entries(recent: &[RecentSend], ws: &Workspace, cx: &App) -> Vec<HistoryEntry> {
    let mut entries = all_history_entries(recent, ws, cx);
    entries.truncate(HISTORY_MAX);
    entries
}

/// As [`history_entries`], uncapped: the History palette searches every listable send.
pub fn all_history_entries(recent: &[RecentSend], ws: &Workspace, cx: &App) -> Vec<HistoryEntry> {
    let names: HashMap<(PathBuf, ItemPath), String> = request_entries(ws, cx)
        .into_iter()
        .map(|r| ((r.root, r.path), r.label))
        .collect();
    let now = SystemTime::now();
    let open = |root: &PathBuf| ws.collections().iter().any(|c| c.root() == root);
    recent
        .iter()
        .filter_map(|r| {
            // A draft's send names its request by method and URL alone, since the draft
            // itself may be long gone; a saved request's send needs the request still listed.
            let (label, target, tag) = if r.item.is_draft() {
                if !open(&r.root) {
                    return None;
                }
                let target = r.send.target();
                let label = if target.is_empty() {
                    UNTITLED.to_owned()
                } else {
                    target
                };
                (label, String::new(), Some(UNSAVED_TAG))
            } else {
                let label = names.get(&(r.root.clone(), r.item.clone()))?.clone();
                let run = r.origin.as_deref() == Some(crate::last_response::RUN_ORIGIN);
                (label, r.send.target(), run.then_some(RUN_TAG))
            };
            Some(HistoryEntry {
                root: r.root.clone(),
                path: r.item.clone(),
                seq: r.send.seq,
                label,
                detail: format!("{} · {}", r.send.status, age_of(r.send.saved_at, now)),
                target,
                tag,
                method: r.send.method.clone(),
                url: r.send.url.clone(),
                status: r.send.status,
            })
        })
        .collect()
}

/// Opens the History dialog (`#history-palette`) over `window`; confirming dispatches
/// `OpenHistoryEntry`.
pub fn open_history(
    fallback: FocusHandle,
    entries: Vec<HistoryEntry>,
    window: &mut Window,
    cx: &mut App,
) {
    let target = window.focused(cx).unwrap_or(fallback);
    // The dialog builder runs every frame, so the rows are shared, and the state outlives it
    // so the query and highlight persist. The search runs once per query change, with the
    // kit's own filter off; Enter asks it for the current query too, since a key can land
    // before the frame that draws the new rows.
    let search = Rc::new(HistorySearch::new(entries));
    let state = cx.new(|cx| CommandState::new(window, cx));
    let dialog_state = state.clone();
    let rows_key = Rc::new(RowsKey::new());
    window.open_dialog(cx, move |dialog, window, cx| {
        // The entries are fixed while the dialog is open, so the query and height decide
        // whether the rows need building again.
        let query = dialog_state.read(cx).query(cx);
        let max_h = crate::palette::list_max_height(window);
        let command = rows_key.render(&dialog_state, (query.clone(), max_h), || {
            let found = search.matches(&query);
            let rows = found
                .iter()
                .enumerate()
                .map(|(r, m)| history_row(r, &search.entries()[m.index], m));
            let none = search.entries().is_empty();
            Command::new(&dialog_state)
                .bordered(false)
                .filterable(false)
                .max_h(max_h)
                .placeholder("Search by name, URL, method or status")
                .group(CommandGroup::new().label("History").items(rows))
                .on_confirm({
                    let (target, search, state) =
                        (target.clone(), search.clone(), dialog_state.clone());
                    move |path, window, cx| {
                        let query = state.read(cx).query(cx);
                        let found = search.matches(&query);
                        let Some(m) = found.get(path.row) else {
                            return;
                        };
                        confirm(&search.entries()[m.index], &target, window, cx);
                    }
                })
                .empty(move |_, _, cx| {
                    if none {
                        empty_message("history-empty", NO_HISTORY, cx)
                    } else {
                        empty_message("history-no-match", NO_MATCH, cx)
                    }
                })
        });
        crate::dialog_frame::place(dialog, window)
            .w(style::resolve(style::PALETTE_WIDTH, window))
            .close_button(false)
            .child(
                div()
                    .id("history-palette")
                    .test_support()
                    .capture_action(crate::palette::escape_closes(&dialog_state))
                    .child(command),
            )
    });
    // The dialog took focus for itself; the query input takes it over so typing filters.
    state.read(cx).focus_handle(cx).focus(window, cx);
}

/// The empty History palette's message.
const NO_HISTORY: &str = "No history yet";

/// The message of a search that kept no send.
const NO_MATCH: &str = "No matching sends";

// An empty-list message, styled as the kit's no-results message.
fn empty_message(id: &'static str, message: &'static str, cx: &App) -> impl IntoElement + use<> {
    div()
        .id(id)
        .test_support()
        .aria_label(message)
        .py_6()
        .w_full()
        .text_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(message)
}

// As in the command palette: close first so the dialog restores its focus record, focus
// `target` so the fallback wins, then dispatch from there a defer later.
fn confirm(e: &HistoryEntry, target: &FocusHandle, window: &mut Window, cx: &mut App) {
    let action: Box<dyn Action> = Box::new(OpenHistoryEntry {
        root: e.root.clone(),
        path: e.path.clone(),
        seq: e.seq,
    });
    window.close_dialog(cx);
    target.focus(window, cx);
    window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
}

// A row shows the request's name, the send's target muted and truncated (whole on hover),
// the send's status and age muted, then its tag muted in a column of its own. The search's
// hits are highlighted in each.
fn history_row(r: usize, entry: &HistoryEntry, found: &HistoryMatch) -> CommandItem {
    let (label, detail, target, tag) = (
        entry.label.clone(),
        entry.detail.clone(),
        entry.target.clone(),
        entry.tag,
    );
    let found = found.clone();
    CommandItem::new().label(label.clone()).child(move |_, cx| {
        crate::palette::count_row_build();
        let muted = cx.theme().muted_foreground;
        let (_, colour) = crate::body_find::match_colours(cx);
        let marked = |text: &str, ranges: &[Range<usize>]| {
            let style = HighlightStyle {
                background_color: Some(colour),
                ..Default::default()
            };
            StyledText::new(text.to_owned())
                .with_highlights(ranges.iter().map(|r| (r.clone(), style)))
        };
        let target = (!target.is_empty()).then(|| {
            let tip = target.clone();
            div()
                .id(("history-target", r))
                .test_support()
                .aria_label(target.clone())
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(muted)
                .child(marked(&target, &found.target))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        });
        // A highlighted name or detail changes its id, so tests can tell a hit from a
        // plain row.
        let name_id = if found.name.is_empty() {
            "history-name"
        } else {
            "history-name-match"
        };
        let detail_id = if found.detail.is_empty() {
            "history-detail"
        } else {
            "history-detail-match"
        };
        h_flex()
            .flex_1()
            .min_w_0()
            .gap(style::SPACE_2)
            .justify_between()
            // The name truncates within half the row, so a long one never pushes the
            // target or the detail out of sight.
            .child(
                div()
                    .id((name_id, r))
                    .test_support()
                    .aria_label(label.clone())
                    .min_w_0()
                    .max_w(relative(0.5))
                    .truncate()
                    .child(marked(&label, &found.name)),
            )
            // A row with no target fills its place, so the detail stays right.
            .when(target.is_none(), |row| row.child(div().flex_1()))
            .children(target)
            .child(
                div()
                    .id((detail_id, r))
                    .test_support()
                    .flex_none()
                    .text_color(muted)
                    .child(marked(&detail, &found.detail)),
            )
            // Every row keeps the fixed tag slot last, so tags line up whatever the detail.
            .child(
                div()
                    .id(("history-tag-slot", r))
                    .test_support()
                    .flex_none()
                    .w(style::HISTORY_TAG_WIDTH)
                    .children(tag.map(|tag| {
                        div()
                            .id(("history-tag", r))
                            .test_support()
                            .aria_label(tag)
                            .flex_none()
                            .text_color(muted)
                            .child(tag)
                    })),
            )
    })
}
