//! The Search collection dialog: a query over the current collection's request text, with the
//! hits grouped under their request. Opening a hit opens its request and shows the section
//! that holds the match.

use crate::theme::SelectedStyle;
use crate::ui::{ChooseButton as _, LockButton as _};
use std::path::PathBuf;

use apinata_core::{
    ItemPath, ReplaceReport, ReplaceScope, SEARCH_HIT_CAP, SearchField, SearchHit, SearchOptions,
    SearchResults,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Enter, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, AnyElement, App, ClickEvent, Entity, FocusHandle, FontWeight, HighlightStyle,
    MouseButton, Point, ScrollHandle, SharedString, StyledText, Subscription, Task, TestSupportExt,
    Window, actions, div,
};

use crate::command::Command;
use crate::request_editor::Section;
use crate::style;
use crate::workspace::Workspace;

actions!(
    apinata,
    [
        /// Searches the current collection's requests.
        OpenCollectionSearch
    ]
);

/// Opens a search hit's request, showing `section` when the match is not in the URL bar; the
/// Shell handles it.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = apinata, no_json)]
pub struct OpenSearchHit {
    /// The root of the request's collection.
    pub root: PathBuf,
    /// The request's path in its collection.
    pub path: ItemPath,
    /// The request editor section holding the match.
    pub section: Option<Section>,
}

/// Counts the searches started, so only the newest answer is shown.
#[derive(Debug, Default)]
pub struct Latest(u64);

impl Latest {
    /// Starts a search and returns its ticket.
    pub fn start(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    /// Whether `ticket` belongs to the newest search.
    pub fn accepts(&self, ticket: u64) -> bool {
        ticket == self.0
    }
}

/// The status line over the hits.
pub fn summary(count: usize, capped: bool) -> String {
    match (count, capped) {
        (0, _) => "No hits".to_owned(),
        (1, _) => "1 hit".to_owned(),
        (n, false) => format!("{n} hits"),
        (n, true) => format!("{n} hits, showing the first {SEARCH_HIT_CAP}"),
    }
}

/// The request editor section that shows a hit's field; `None` for the name and the URL,
/// which the URL bar always shows.
pub fn section_for(field: SearchField, key: Option<&str>) -> Option<Section> {
    match field {
        SearchField::PathParam => Some(Section::Params),
        SearchField::HeaderName | SearchField::HeaderValue => Some(Section::Headers),
        SearchField::Body => Some(Section::Body),
        SearchField::Script if key == Some("tests") => Some(Section::Tests),
        SearchField::Script => Some(Section::Script),
        SearchField::Docs => Some(Section::Docs),
        _ => None,
    }
}

// Longest list of request names a toast spells out.
const NAMES_SHOWN: usize = 5;

/// The toast texts for a finished replace: the summary, then one text naming the requests
/// skipped for unsaved edits and one naming the failures, when there are any.
pub fn replace_notes(report: &ReplaceReport) -> Vec<String> {
    let mut notes = vec![report.summary()];
    if !report.skipped_unsaved.is_empty() {
        let shown = report.skipped_unsaved.iter().take(NAMES_SHOWN);
        let mut text = format!(
            "Skipped, unsaved edits: {}",
            shown.cloned().collect::<Vec<_>>().join(", ")
        );
        let more = report.skipped_unsaved.len().saturating_sub(NAMES_SHOWN);
        if more > 0 {
            text.push_str(&format!(" and {more} more"));
        }
        notes.push(text);
    }
    if !report.failed.is_empty() {
        let shown: Vec<String> = report
            .failed
            .iter()
            .take(NAMES_SHOWN)
            .map(|(name, why)| format!("{name} ({why})"))
            .collect();
        let mut text = format!("Could not replace in: {}", shown.join("; "));
        let more = report.failed.len().saturating_sub(NAMES_SHOWN);
        if more > 0 {
            text.push_str(&format!(" and {more} more"));
        }
        notes.push(text);
    }
    notes
}

// A replace toast, an error one when the replace could not run. Its text also labels a
// marker, `search-replace-note` with its place in the replace's toasts, so it can be read back.
fn replace_toast(ix: usize, note: String, failed: bool) -> Notification {
    let label = SharedString::from(note.clone());
    let toast = if failed {
        Notification::error(note)
    } else {
        crate::ui::info_toast(note)
    };
    toast.content(move |_, _, _| {
        div()
            .id(("search-replace-note", ix))
            .test_support()
            .aria_label(label.clone())
            .when(failed, |d| d.child(div().id("toast-error").test_support()))
            .into_any_element()
    })
}

// What a hit is in, from its structured field and key.
fn hit_what(hit: &SearchHit) -> String {
    let key = hit.key.as_deref().unwrap_or("");
    match hit.field {
        SearchField::Name => "Name".to_owned(),
        SearchField::Url => "URL".to_owned(),
        SearchField::PathParam => format!("Path param {key}"),
        SearchField::HeaderName => "Header name".to_owned(),
        SearchField::HeaderValue => format!("Header {key}"),
        SearchField::Body if key.is_empty() => "Body".to_owned(),
        SearchField::Body => format!("Body {key}"),
        SearchField::Script => format!("Script {key}"),
        SearchField::Docs => "Docs".to_owned(),
        _ => "Match".to_owned(),
    }
}

const REPLACE_HINT: &str = "Changes are written to disk. Revert them with git.";

const NOTE: &str =
    "Searches request text, including unsaved edits. Only secret variable values are left out.";

struct SearchForm {
    workspace: Entity<Workspace>,
    root: PathBuf,
    input: Entity<InputState>,
    results: SearchResults,
    // The query `results` answers; empty before the first answer.
    answered: String,
    // Why the last search could not run; shown in place of the hit count.
    error: Option<String>,
    options: SearchOptions,
    // The Replace field and its buttons are shown only while this is on.
    replacing: bool,
    replacement: Entity<InputState>,
    _replace: Option<Task<()>>,
    latest: Latest,
    selected: usize,
    scroll: ScrollHandle,
    // Where focus goes when the dialog closes on a hit.
    target: FocusHandle,
    // Replace all's own focus, so locking it can hand focus to the query.
    replace_all_focus: FocusHandle,
    _search: Option<Task<()>>,
    _typing: Subscription,
}

impl SearchForm {
    // Starts a search for the input's text on the collection's actor; a blank query clears the
    // list at once. Each search takes a ticket, and only the newest answer is kept.
    fn run(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().trim().to_owned();
        let ticket = self.latest.start();
        if query.is_empty() {
            self.results = SearchResults::default();
            self.answered.clear();
            self.error = None;
            self._search = None;
            cx.notify();
            return;
        }
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::Search {
            query: query.clone(),
            options: self.options.clone(),
            reply,
        };
        self.workspace
            .update(cx, |w, cx| w.send(&self.root, command, cx));
        self._search = Some(cx.spawn(async move |this, cx| {
            let found = answer
                .recv()
                .await
                .unwrap_or_else(|_| Ok(SearchResults::default()));
            this.update(cx, |form, cx| {
                if form.latest.accepts(ticket) {
                    (form.results, form.error) = match found {
                        Ok(results) => (results, None),
                        Err(text) => (SearchResults::default(), Some(text)),
                    };
                    form.answered = query;
                    form.selected = 0;
                    form.scroll.set_offset(Point::default());
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    // Flips an option and searches again with the same query.
    fn toggle(
        &mut self,
        flip: impl FnOnce(SearchOptions) -> SearchOptions,
        cx: &mut Context<Self>,
    ) {
        self.options = flip(self.options.clone());
        self.run(cx);
        cx.notify();
    }

    // A text toggle styled like the body find bar's case button, which is an icon: ghost,
    // compact, and the ghost selected look while on.
    fn option_button(
        &self,
        id: &'static str,
        text: &'static str,
        name: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        flip: fn(SearchOptions) -> SearchOptions,
    ) -> AnyElement {
        div()
            .id(id)
            .test_support()
            .aria_label(name)
            .child(
                // The press lives on the button so Enter or Space on it toggles too.
                Button::new(SharedString::from(format!("{id}-button")))
                    .key_context(crate::ui::DIALOG_FOOTER_CONTEXT)
                    .ghost()
                    .compact()
                    .label(text)
                    .tooltip(name)
                    .choose(on, SelectedStyle::Ghost, cx)
                    .on_click(
                        cx.listener(move |form, _: &ClickEvent, _, cx| form.toggle(flip, cx)),
                    ),
            )
            .into_any_element()
    }

    fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replacing = !self.replacing;
        let focus = if self.replacing {
            &self.replacement
        } else {
            &self.input
        };
        focus.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    // Replaces on the actor with the dialog's query, options and replacement text, then
    // searches again so the list shows what is left, and toasts what happened.
    fn replace(&mut self, scope: ReplaceScope, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().trim().to_owned();
        if query.is_empty() {
            return;
        }
        let (reply, answer) = async_channel::bounded(1);
        let command = Command::Replace {
            query,
            replacement: self.replacement.read(cx).value().to_string(),
            options: self.options.clone(),
            scope,
            reply,
        };
        self.workspace
            .update(cx, |w, cx| w.send(&self.root, command, cx));
        self._replace = Some(cx.spawn_in(window, async move |this, cx| {
            let done = answer
                .recv()
                .await
                .unwrap_or_else(|_| Err("The collection is not open".to_owned()));
            this.update_in(cx, |form, window, cx| {
                let (notes, failed) = match done {
                    Ok(report) => (replace_notes(&report), false),
                    Err(text) => (vec![text], true),
                };
                for (ix, note) in notes.into_iter().enumerate() {
                    window.push_notification(replace_toast(ix, note, failed), cx);
                }
                form.run(cx);
            })
            .ok();
        }));
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        let last = self.results.hits.len().saturating_sub(1);
        self.selected = if down {
            (self.selected + 1).min(last)
        } else {
            self.selected.saturating_sub(1)
        };
        cx.notify();
    }

    // The row is always scrolled fully into view. Its group header is the target instead only
    // when the header is above the viewport and header top to row bottom fits, so the header
    // sits above the row going up and is never pushed out going down. Bounds are last frame's.
    fn scroll_target(&self, row: usize, header: Option<usize>) -> usize {
        let Some(header) = header else { return row };
        // The top of the list is always the first header, whatever the last list scrolled to.
        if self.selected == 0 {
            return header;
        }
        let (Some(h), Some(r)) = (
            self.scroll.bounds_for_item(header),
            self.scroll.bounds_for_item(row),
        ) else {
            return row;
        };
        let view = self.scroll.bounds();
        // Child bounds are unscrolled, so the scroll offset is added to place them.
        if h.top() + self.scroll.offset().y < view.top() && r.bottom() - h.top() <= view.size.height
        {
            header
        } else {
            row
        }
    }

    // Closes the dialog, then dispatches from the focus it came from, as the palettes do.
    fn open_hit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.results.hits.get(ix) else {
            return;
        };
        let action: Box<dyn Action> = Box::new(OpenSearchHit {
            root: self.root.clone(),
            path: hit.path.clone(),
            section: section_for(hit.field, hit.key.as_deref()),
        });
        window.close_dialog(cx);
        self.target.focus(window, cx);
        window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
    }

    fn row(&self, ix: usize, hit: &SearchHit, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (active, hover, muted) = (theme.list_active, theme.list_hover, theme.muted_foreground);
        let (_, highlight) = crate::body_find::match_colours(cx);
        let what = hit_what(hit);
        let label = format!("{what}: {}", hit.snippet);
        let text = StyledText::new(hit.snippet.clone()).with_highlights([(
            hit.match_range.clone(),
            HighlightStyle {
                background_color: Some(highlight),
                ..Default::default()
            },
        )]);
        h_flex()
            .id(("search-hit", ix))
            .test_support()
            .aria_label(label)
            .gap(style::SPACE_2)
            .px(style::SPACE_3)
            .py(style::SPACE_0)
            .cursor_pointer()
            .when(ix == self.selected, |row| row.bg(active))
            .when(ix != self.selected, |row| row.hover(move |s| s.bg(hover)))
            .on_click(cx.listener(move |form, _, window, cx| form.open_hit(ix, window, cx)))
            .child(
                div()
                    .id(("search-what", ix))
                    .test_support()
                    .aria_label(what.clone())
                    .flex_none()
                    .w(style::SEARCH_FIELD_WIDTH)
                    .text_color(muted)
                    .truncate()
                    .child(what),
            )
            .child(div().flex_1().min_w_0().truncate().child(text))
            .when(self.replacing, |row| {
                // The press is claimed here so the row underneath does not open the hit.
                row.child(
                    div()
                        .id(("search-replace-hit", ix))
                        .test_support()
                        .aria_label("Replace this match")
                        .flex_none()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new(("search-replace-hit-button", ix))
                                .key_context(crate::ui::DIALOG_FOOTER_CONTEXT)
                                .ghost()
                                .compact()
                                .label("Replace")
                                .on_click(cx.listener(move |form, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    if let Some(hit) = form.results.hits.get(ix) {
                                        let scope = ReplaceScope::hit(hit.clone());
                                        form.replace(scope, window, cx);
                                    }
                                })),
                        ),
                )
            })
    }
}

impl Render for SearchForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let hits = self.results.hits.clone();
        // Flat children: a header where the request changes, then its rows. `at` is the
        // selected row's child index and `header` its group header's when it is the group's
        // first hit.
        let mut rows: Vec<AnyElement> = Vec::new();
        let (mut group, mut at, mut last) = (0usize, 0usize, None::<&ItemPath>);
        let mut header = None::<usize>;
        for (ix, hit) in hits.iter().enumerate() {
            let first = last != Some(&hit.path);
            if first {
                last = Some(&hit.path);
                if ix == self.selected {
                    header = Some(rows.len());
                }
                rows.push(
                    h_flex()
                        .id(("search-group", group))
                        .test_support()
                        .aria_label(hit.path.as_str().to_owned())
                        .gap(style::SPACE_2)
                        .px(style::SPACE_3)
                        .pt(style::SPACE_2)
                        .font_weight(FontWeight::MEDIUM)
                        .child(hit.request_name.clone())
                        .child(div().text_color(muted).child(hit.path.as_str().to_owned()))
                        .into_any_element(),
                );
                group += 1;
            }
            if ix == self.selected {
                at = rows.len();
            }
            rows.push(self.row(ix, hit, cx).into_any_element());
        }
        if !hits.is_empty() {
            self.scroll.scroll_to_item(self.scroll_target(at, header));
        }
        let danger = cx.theme().danger;
        let status = match (&self.error, self.answered.is_empty()) {
            (Some(text), _) => Some((text.clone(), danger)),
            (None, true) => None,
            (None, false) => Some((summary(hits.len(), self.results.capped), muted)),
        };
        let (case, regex) = (self.options.case_sensitive, self.options.regex);
        let case_button = self.option_button("search-case", "Aa", "Match case", case, cx, |o| {
            let on = !o.case_sensitive;
            o.with_case_sensitive(on)
        });
        let regex_button = self.option_button("search-regex", ".*", "Regex", regex, cx, |o| {
            let on = !o.regex;
            o.with_regex(on)
        });
        let replacing = self.replacing;
        let empty = hits.is_empty();
        // The kit keeps focus on a button disabled under it; a locked Replace all gives it to
        // the query instead, so typing searches again.
        if empty && self.replace_all_focus.is_focused(window) {
            let input = self.input.clone();
            window.defer(cx, move |window, cx| {
                input.update(cx, |i, cx| i.focus(window, cx));
            });
        }
        let toggle_name = if replacing {
            "Hide replace"
        } else {
            "Show replace"
        };
        let replace_toggle = div()
            .id("search-replace-toggle")
            .test_support()
            .aria_label(toggle_name)
            .child(
                Button::new("search-replace-toggle-button")
                    .key_context(crate::ui::DIALOG_FOOTER_CONTEXT)
                    .ghost()
                    .compact()
                    .label("Replace")
                    .tooltip(toggle_name)
                    .choose(replacing, SelectedStyle::Ghost, cx)
                    .on_click(cx.listener(|form, _: &ClickEvent, window, cx| {
                        form.toggle_replace(window, cx)
                    })),
            );
        let replace_all = div()
            .id("search-replace-all")
            .test_support()
            .aria_label("Replace all")
            .child(
                Button::new("search-replace-all-button")
                    .track_focus(&self.replace_all_focus)
                    .key_context(crate::ui::DIALOG_FOOTER_CONTEXT)
                    .compact()
                    .label("Replace all")
                    .lock(empty, cx)
                    .on_click(cx.listener(|form, _: &ClickEvent, window, cx| {
                        if !form.results.hits.is_empty() {
                            form.replace(ReplaceScope::all(), window, cx);
                        }
                    })),
            );
        v_flex()
            .gap(style::SPACE_2)
            .capture_action(cx.listener(|form, _: &MoveDown, _, cx| {
                form.step(true, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|form, _: &MoveUp, _, cx| {
                form.step(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|form, a: &Enter, window, cx| {
                if !a.secondary && !a.shift {
                    form.open_hit(form.selected, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                h_flex()
                    .gap(style::SPACE_1)
                    .child(crate::ui::padded_input(
                        Input::new(&self.input)
                            .id("search-input")
                            .flex_1()
                            .bg(crate::ui::field_fill(false, cx)),
                        &self.input,
                    ))
                    .child(case_button)
                    .child(regex_button)
                    .child(replace_toggle),
            )
            .when(replacing, |v| {
                v.child(
                    h_flex()
                        .gap(style::SPACE_1)
                        .child(crate::ui::padded_input(
                            Input::new(&self.replacement)
                                .id("search-replace-input")
                                .flex_1()
                                .bg(crate::ui::field_fill(false, cx)),
                            &self.replacement,
                        ))
                        .child(replace_all),
                )
                .child(
                    div()
                        .id("search-replace-hint")
                        .test_support()
                        .aria_label(REPLACE_HINT)
                        .px(style::SPACE_3)
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .child(REPLACE_HINT),
                )
            })
            .children(status.map(|(text, colour)| {
                div()
                    .id("search-summary")
                    .test_support()
                    .aria_label(text.clone())
                    .px(style::SPACE_3)
                    .text_color(colour)
                    .child(text)
            }))
            .child(
                v_flex()
                    .id("search-hits")
                    .test_support()
                    .max_h(crate::palette::list_max_height(window))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
            .child(
                div()
                    .id("search-note")
                    .test_support()
                    .aria_label(NOTE)
                    .px(style::SPACE_3)
                    .text_size(style::TEXT_SMALL)
                    .text_color(muted)
                    .child(NOTE),
            )
    }
}

/// Opens the Search collection dialog over `window` for the collection at `root`.
/// Choosing a hit dispatches [`OpenSearchHit`] from the focus the dialog came from.
pub fn open(
    workspace: &Entity<Workspace>,
    root: PathBuf,
    fallback: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let target = window.focused(cx).unwrap_or(fallback);
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search requests"));
    let replacement = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
    let form = cx.new(|cx| SearchForm {
        replacing: false,
        replacement,
        _replace: None,
        workspace: workspace.clone(),
        root,
        results: SearchResults::default(),
        answered: String::new(),
        error: None,
        options: SearchOptions::default(),
        latest: Latest::default(),
        selected: 0,
        scroll: ScrollHandle::new(),
        target,
        replace_all_focus: cx.focus_handle(),
        _search: None,
        _typing: cx.subscribe(
            &input,
            |form: &mut SearchForm, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    form.run(cx);
                }
            },
        ),
        input: input.clone(),
    });
    window.open_dialog(cx, move |dialog, window, _| {
        crate::dialog_frame::place(dialog, window)
            .w(style::resolve(style::PALETTE_WIDTH, window))
            .close_button(false)
            .child(
                div()
                    .id("collection-search")
                    .test_support()
                    .child(form.clone()),
            )
    });
    // The dialog took focus for itself; the query input takes it over so typing searches.
    input.update(cx, |i, cx| i.focus(window, cx));
}
