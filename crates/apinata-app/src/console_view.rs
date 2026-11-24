//! The one console view, drawn by the response pane's Console tab and by the dock: a level
//! filter, a search box and Copy over bounded rows. Each host keeps its own [`ConsoleFilter`],
//! so a filter set in one place never reaches the other. Rows arrive already redacted.

use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use apinata_core::{ConsoleEntry, ConsoleLevel, ConsoleLine, ScriptPhase};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ClipboardItem, Div, Entity, Hsla, TestSupportExt, Window, div};

use crate::log_feed;
use crate::response_pane::scroll_frame;
use crate::ui::SolidRing as _;
use crate::{style, ui};

/// One line the view can draw, from a single send's result or from the session log.
pub trait ConsoleItem {
    /// The console method called.
    fn level(&self) -> ConsoleLevel;
    /// The script phase that wrote the line, or `None` for a message from the host app.
    fn phase(&self) -> Option<ScriptPhase>;
    /// The request that wrote the line, where the view mixes lines of many requests.
    fn request(&self) -> Option<&str>;
    /// When the line was collected, where that is known.
    fn at(&self) -> Option<SystemTime>;
    /// The line's redacted text.
    fn text(&self) -> &str;
}

impl ConsoleItem for ConsoleEntry {
    fn level(&self) -> ConsoleLevel {
        self.level
    }
    fn phase(&self) -> Option<ScriptPhase> {
        Some(self.phase)
    }
    fn request(&self) -> Option<&str> {
        None
    }
    fn at(&self) -> Option<SystemTime> {
        None
    }
    fn text(&self) -> &str {
        &self.text
    }
}

impl ConsoleItem for ConsoleLine {
    fn level(&self) -> ConsoleLevel {
        self.level
    }
    fn phase(&self) -> Option<ScriptPhase> {
        self.phase
    }
    fn request(&self) -> Option<&str> {
        Some(&self.request)
    }
    fn at(&self) -> Option<SystemTime> {
        Some(self.at)
    }
    fn text(&self) -> &str {
        &self.text
    }
}

impl<T: ConsoleItem> ConsoleItem for Arc<T> {
    fn level(&self) -> ConsoleLevel {
        (**self).level()
    }
    fn phase(&self) -> Option<ScriptPhase> {
        (**self).phase()
    }
    fn request(&self) -> Option<&str> {
        (**self).request()
    }
    fn at(&self) -> Option<SystemTime> {
        (**self).at()
    }
    fn text(&self) -> &str {
        (**self).text()
    }
}

/// The element ids one host gives the view, so each keeps the names its tests and QA know.
#[non_exhaustive]
pub struct ConsoleIds {
    /// The level filter's segments.
    pub filter: &'static str,
    /// The search box's frame.
    pub search: &'static str,
    /// The Copy button.
    pub copy: &'static str,
    /// The scrolled list.
    pub list: &'static str,
    /// The empty state.
    pub none: &'static str,
    /// The note over a list that leaves out its oldest lines.
    pub omitted: &'static str,
    /// Each row, by index.
    pub row: &'static str,
    /// Each row's level cell, by index.
    pub level: &'static str,
    /// Each row's "more lines" note, by index.
    pub more: &'static str,
}

/// The response pane's Console tab ids.
pub const TAB_IDS: ConsoleIds = ConsoleIds {
    filter: "console-filter",
    search: "console-search",
    copy: "console-copy",
    list: "response-console",
    none: "console-none",
    omitted: "console-more-entries",
    row: "console",
    level: "console-level",
    more: "console-more",
};

/// The dock's Console tab ids.
pub const DOCK_IDS: ConsoleIds = ConsoleIds {
    filter: "dock-console-level",
    search: "dock-console-search",
    copy: "dock-console-copy",
    list: "dock-console-list",
    none: "dock-console-none",
    omitted: "dock-console-omitted",
    row: "dock-console",
    level: "dock-console-row-level",
    more: "dock-console-more",
};

/// One host's level filter and search, with the search box that edits it.
pub struct ConsoleFilter {
    level: Option<ConsoleLevel>,
    search: String,
    input: Entity<InputState>,
}

impl ConsoleFilter {
    /// Every level and no search, with an empty search box.
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter console lines"));
        ConsoleFilter {
            level: None,
            search: String::new(),
            input,
        }
    }

    /// The level filter; `None` keeps every level.
    pub fn level(&self) -> Option<ConsoleLevel> {
        self.level
    }

    /// Sets the level filter.
    pub fn set_level(&mut self, level: Option<ConsoleLevel>) {
        self.level = level;
    }

    /// The search text.
    pub fn search(&self) -> &str {
        &self.search
    }

    /// Sets the search text; the box keeps whatever it shows.
    pub fn set_search(&mut self, query: &str) {
        self.search = query.to_string();
    }

    /// The search box, whose changes the host feeds back through [`Self::set_search`].
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// Whether `item` passes the level filter and the search.
    pub fn keeps(&self, item: &impl ConsoleItem) -> bool {
        keeps(item, self.level, &self.search)
    }
}

fn keeps(item: &impl ConsoleItem, level: Option<ConsoleLevel>, search: &str) -> bool {
    let request = item.request().unwrap_or_default();
    log_feed::console_keeps(item.level(), request, item.text(), level, search)
}

/// A line as a row's label and Copy word it: "{clock} {request} {level} [{phase}] {text}",
/// leaving out the parts the line does not carry.
pub fn console_line(item: &impl ConsoleItem) -> String {
    let mut parts = Vec::with_capacity(5);
    parts.extend(item.at().map(log_feed::clock_label));
    parts.extend(item.request().map(str::to_owned));
    parts.push(item.level().as_str().to_owned());
    parts.extend(item.phase().map(|p| format!("[{}]", p.as_str())));
    parts.push(item.text().to_owned());
    parts.join(" ")
}

/// The lines of the items `level` keeps (all for `None`), joined by "\n".
pub fn console_text<T: ConsoleItem>(items: &[T], level: Option<ConsoleLevel>) -> String {
    matching_text(items, level, "")
}

fn matching_text<T: ConsoleItem>(items: &[T], level: Option<ConsoleLevel>, search: &str) -> String {
    items
        .iter()
        .filter(|i| keeps(*i, level, search))
        .map(console_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The level filter's segments, in order: All, then each level.
pub const CONSOLE_FILTERS: [Option<ConsoleLevel>; 6] = [
    None,
    Some(ConsoleLevel::Log),
    Some(ConsoleLevel::Debug),
    Some(ConsoleLevel::Info),
    Some(ConsoleLevel::Warn),
    Some(ConsoleLevel::Error),
];

/// The view: the filter, search and Copy over the kept items. The row budget is spent newest
/// first, so a noisy log keeps its latest rows, drawn back in time order under a note.
pub fn view<T: ConsoleItem + 'static>(
    ids: &'static ConsoleIds,
    items: Rc<[T]>,
    filter: &ConsoleFilter,
    on_level: impl Fn(Option<ConsoleLevel>, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let selected = CONSOLE_FILTERS
        .iter()
        .position(|f| *f == filter.level)
        .unwrap_or(0);
    let pick = ui::segmented(
        ids.filter,
        ["All", "Log", "Debug", "Info", "Warn", "Error"],
        selected,
        move |ix, _, cx| on_level(CONSOLE_FILTERS.get(*ix).copied().flatten(), cx),
    );
    let search = div()
        .id(ids.search)
        .test_support()
        .flex_1()
        .min_w_0()
        .child(ui::padded_input(
            Input::new(&filter.input).bg(ui::field_fill(false, cx)),
            &filter.input,
        ));
    // Copy builds its text only when clicked, from every kept line, not just the drawn ones.
    let (level, query, all) = (filter.level, filter.search.clone(), Rc::clone(&items));
    let copy = Button::new(ids.copy)
        .solid_ring(cx)
        .ghost()
        .small()
        .label("Copy")
        .on_click(move |_, _, cx| {
            let text = matching_text(&all, level, &query);
            cx.write_to_clipboard(ClipboardItem::new_string(text))
        });
    let kept: Vec<&T> = items.iter().filter(|i| filter.keeps(*i)).collect();
    let (mut bodies, left) = console_shown(kept.iter().rev().map(|i| i.text()));
    bodies.reverse();
    let drawn = &kept[left..];
    let muted = cx.theme().muted_foreground;
    let none = drawn
        .is_empty()
        .then(|| ui::empty_state(ids.none, "Nothing was logged.", &[]));
    let omitted = (left > 0).then(|| {
        div()
            .id(ids.omitted)
            .test_support()
            .text_color(muted)
            .child(format!(
                "{}; use Copy for the full log",
                log_feed::omitted_label(left, "line")
            ))
    });
    let rows = drawn
        .iter()
        .zip(bodies)
        .enumerate()
        .map(|(ix, (item, body))| {
            h_flex()
                .id((ids.row, ix))
                .test_support()
                .aria_label(console_line(*item))
                .items_start()
                .gap(style::SPACE_2)
                .children(item.at().map(|at| {
                    div()
                        .flex_none()
                        .text_color(muted)
                        .child(log_feed::clock_label(at))
                }))
                .children(
                    item.request()
                        .map(|r| div().flex_none().child(r.to_owned())),
                )
                .child(
                    div()
                        .id((ids.level, ix))
                        .test_support()
                        .flex_none()
                        .text_color(level_colour(item.level(), cx))
                        .child(item.level().as_str()),
                )
                .children(item.phase().map(|p| {
                    div()
                        .flex_none()
                        .text_color(muted)
                        .child(format!("[{}]", p.as_str()))
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(console_body(ids.more, ix, body, cx)),
                )
        });
    v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .gap(style::SPACE_2)
        .child(
            h_flex()
                .gap(style::SPACE_2)
                .items_center()
                .child(pick)
                .child(search)
                .child(copy),
        )
        .child(scroll_frame(
            ids.list,
            v_flex().children(omitted).children(none).children(rows),
        ))
        .into_any_element()
}

/// Warn and error lines stand out; every other level stays muted.
pub fn level_colour(level: ConsoleLevel, cx: &App) -> Hsla {
    match level {
        ConsoleLevel::Warn => cx.theme().warning,
        ConsoleLevel::Error => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    }
}

/// An entry's text as bounded rows, then a muted note `(more_id, ix)` for the lines left out.
pub fn console_body(
    more_id: &'static str,
    ix: usize,
    (lines, more): (Vec<String>, Option<String>),
    cx: &App,
) -> Div {
    v_flex()
        .children(lines.into_iter().map(|line| div().child(line)))
        .children(more.map(|note| {
            div()
                .id((more_id, ix))
                .test_support()
                .text_color(cx.theme().muted_foreground)
                .child(note)
        }))
}

/// One console entry's shown rows and its "... N more lines" note, if any.
pub type ConsoleRows = (Vec<String>, Option<String>);

/// How many lines of one console entry the view shows.
pub const CONSOLE_ROW_LINES: usize = 1000;
/// How many characters of one console entry the view shows, each line break counting as one.
pub const CONSOLE_ROW_CHARS: usize = 5000;
/// How many lines the view shows across all its entries.
pub const CONSOLE_PANE_LINES: usize = 2000;
/// How many characters the view shows across all its entries, counted as per entry.
pub const CONSOLE_PANE_CHARS: usize = 50_000;

/// The rows the view shows for an entry's `text`, and a "... N more lines" note when lines
/// are left out. A line cut at the character limit ends in "…"; a blank line is one space so
/// it keeps its height; a trailing carriage return is dropped.
pub fn console_rows(text: &str) -> ConsoleRows {
    let (mut lines, mut chars) = (CONSOLE_ROW_LINES, CONSOLE_ROW_CHARS);
    rows_within(text, &mut lines, &mut chars)
}

/// The rows of each entry text the view shows, as [`console_rows`] but drawing on one budget
/// of [`CONSOLE_PANE_LINES`] and [`CONSOLE_PANE_CHARS`]; the entries left out once it is
/// spent are counted, not returned.
pub fn console_shown<'a>(texts: impl IntoIterator<Item = &'a str>) -> (Vec<ConsoleRows>, usize) {
    let (mut lines, mut chars) = (CONSOLE_PANE_LINES, CONSOLE_PANE_CHARS);
    let mut shown = Vec::new();
    let mut left = 0;
    for text in texts {
        if lines == 0 || chars == 0 {
            left += 1;
            continue;
        }
        let mut entry_lines = lines.min(CONSOLE_ROW_LINES);
        let mut entry_chars = chars.min(CONSOLE_ROW_CHARS);
        let rows = rows_within(text, &mut entry_lines, &mut entry_chars);
        lines -= lines.min(CONSOLE_ROW_LINES) - entry_lines;
        chars -= chars.min(CONSOLE_ROW_CHARS) - entry_chars;
        shown.push(rows);
    }
    (shown, left)
}

// Takes rows of `text` while `lines` and `chars` last, spending them, and notes the rest.
fn rows_within(text: &str, lines: &mut usize, chars: &mut usize) -> ConsoleRows {
    let total = text.bytes().filter(|b| *b == b'\n').count() + 1;
    let mut rows = Vec::new();
    for line in text.split('\n') {
        if *lines == 0 || *chars == 0 {
            break;
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        *lines -= 1;
        if let Some((cut, _)) = line.char_indices().nth(*chars) {
            rows.push(format!("{}…", &line[..cut]));
            *chars = 0;
            break;
        }
        *chars = chars.saturating_sub(line.chars().count() + 1);
        rows.push(if line.is_empty() {
            " ".to_owned()
        } else {
            line.to_owned()
        });
    }
    let more = total - rows.len();
    let note = match more {
        0 => None,
        1 => Some("... 1 more line".to_owned()),
        n => Some(format!("... {n} more lines")),
    };
    (rows, note)
}
