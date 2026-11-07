//! Completing `{{name}}` placeholders: the partial name at the cursor, the names the
//! request can use, their ranking, the edit that accepts one, and the list that offers them.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use apinata_core::{VarScope, VariableSet};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Enter, Escape, IndentInline, MoveDown, MoveUp};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, AnyElement, App, Div, ElementId, Entity, MouseButton, MouseDownEvent, Pixels, Point,
    Role, ScrollHandle, SharedString, Stateful, TestSupportExt, Window, anchored, deferred, div,
};

use crate::header_complete::{self, Hint, offers};
use crate::style;
use crate::var_tokens::{SECRET_DOTS, Source, scope_label};

/// The longest value a list row previews before cutting it with an ellipsis, in characters.
pub const PREVIEW_CHARS: usize = 40;

/// An open `{{` before the cursor and the partial name typed after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Byte offset of the `{{`.
    pub start: usize,
    /// The name characters around the cursor, which an accepted name replaces.
    pub replace: Range<usize>,
    /// The part of the name typed before the cursor; the list filters on it.
    pub typed: String,
}

/// Characters a placeholder name is made of; anything else, such as an auto-closed quote,
/// ends it and survives an accepted completion.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '-')
}

/// The query at `cursor` when the text before it ends with `{{` and a partial name (only
/// name characters); `None` otherwise, or when `cursor` is not a char boundary.
pub fn query_at(text: &str, cursor: usize) -> Option<Query> {
    let before = text.get(..cursor)?;
    let start = before.rfind("{{")?;
    let typed = &before[start + 2..];
    if !typed.chars().all(is_name_char) {
        return None;
    }
    // The name runs on past the cursor to the first character that is not a name character.
    let after = &text[cursor..];
    let rest = after.find(|c| !is_name_char(c)).unwrap_or(after.len());
    Some(Query {
        start,
        replace: start + 2..cursor + rest,
        typed: typed.to_owned(),
    })
}

/// One name the list offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The placeholder name, as inserted.
    pub name: String,
    /// `var_tokens::scope_label` of its scope.
    pub scope: &'static str,
    /// The value cut to PREVIEW_CHARS, SECRET_DOTS for a secret, empty for a dynamic name.
    pub preview: String,
    /// A `{{$name}}` generated per send.
    pub dynamic: bool,
}

/// Every name `vars.names()` lists that `vars.get` defines, in that order.
pub fn choices(vars: &VariableSet) -> Vec<Choice> {
    vars.names()
        .into_iter()
        .filter_map(|name| {
            let info = vars.get(&name)?;
            let dynamic = info.scope() == VarScope::Dynamic;
            let preview = if dynamic {
                String::new()
            } else if info.is_secret() {
                SECRET_DOTS.to_owned()
            } else {
                preview(info.value().unwrap_or_default())
            };
            Some(Choice {
                name,
                scope: scope_label(info.scope()),
                preview,
                dynamic,
            })
        })
        .collect()
}

/// `value` cut after PREVIEW_CHARS characters, with an ellipsis when cut.
fn preview(value: &str) -> String {
    let mut shown: String = value.chars().take(PREVIEW_CHARS).collect();
    if value.chars().count() > PREVIEW_CHARS {
        shown.push('…');
    }
    shown
}

/// Matches of `typed`, ignoring case: non-dynamic first, then prefix before substring,
/// then input order.
pub fn filter<'a>(all: &'a [Choice], typed: &str) -> Vec<&'a Choice> {
    let typed = typed.to_lowercase();
    let mut hits: Vec<(bool, bool, usize, &Choice)> = all
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let name = c.name.to_lowercase();
            name.contains(&typed)
                .then(|| (c.dynamic, !name.starts_with(&typed), i, c))
        })
        .collect();
    hits.sort_by_key(|&(dynamic, substring, i, _)| (dynamic, substring, i));
    hits.into_iter().map(|(.., c)| c).collect()
}

/// The edit accepting `name`: range to replace, replacement, cursor after.
pub fn accept(text: &str, query: &Query, name: &str) -> (Range<usize>, String, usize) {
    // Close the pair, adding only the braces not already right after the replaced name.
    let present = text.get(query.replace.end..).map_or(0, |rest| {
        rest.chars().take(2).take_while(|&c| c == '}').count()
    });
    let mut replacement = name.to_owned();
    replacement.push_str(&"}".repeat(2 - present));
    let cursor = query.replace.start + name.len() + 2;
    (query.replace.clone(), replacement, cursor)
}

/// What a list completes, which sets how its rows are drawn and how an accepted row edits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `{{name}}` placeholders.
    Vars,
    /// Header names or values, replacing the whole cell.
    Hints,
}

impl Mode {
    fn list_id(self) -> &'static str {
        match self {
            Mode::Vars => "var-complete",
            Mode::Hints => "hint-complete",
        }
    }

    fn row_id(self) -> &'static str {
        match self {
            Mode::Vars => "var-complete-row",
            Mode::Hints => "hint-complete-row",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::Vars => "Variables",
            Mode::Hints => "Header suggestions",
        }
    }
}

/// The open list: the `{{` it belongs to, the partial name it last filtered on, and the
/// highlighted row.
struct Menu {
    start: usize,
    typed: String,
    selected: usize,
}

/// Per-field list state, kept apart from the field's token state.
#[derive(Default)]
struct ListState {
    /// The text of the last frame, so the list opens on typing and not on cursor moves.
    last_text: Option<SharedString>,
    menu: Option<Menu>,
    /// The `{{` whose list Escape closed; it stays closed until the cursor leaves it.
    dismissed: Option<usize>,
    /// The choices for one set and revision.
    choices: Option<(Arc<VariableSet>, u64, Rc<[Choice]>)>,
    scroll: ScrollHandle,
}

/// An open list as one frame shows it: the query, the matching choices and the highlighted row.
struct OpenMenu {
    query: Query,
    shown: Rc<[Choice]>,
    selected: usize,
}

/// Give the field `root` (whose id is `id`) its list: shown under the `{{` while the user
/// types a name, with its keys taken from the field. Returns whether it shows this frame.
pub(crate) fn attach(
    root: Stateful<Div>,
    id: &ElementId,
    source: &Source,
    vars: &Arc<VariableSet>,
    window: &mut Window,
    cx: &mut App,
) -> (Stateful<Div>, bool) {
    let key = ElementId::NamedChild(Arc::new(id.clone()), "var-complete".into());
    let state = window.use_keyed_state(key, cx, |_, _| ListState::default());
    let text = source.text(cx);
    let query = source
        .caret(window, cx)
        .and_then(|caret| query_at(&text, caret));
    let Some(open) = sync(&state, &text, query, vars, cx) else {
        return (root, false);
    };
    // Before the field has laid out the `{{` there is nowhere to show the list, so it
    // neither shows nor takes the field's keys this frame.
    let Some(anchor) = source.bounds(&(open.query.start..open.query.replace.start), cx) else {
        return (root, false);
    };
    // Where the list would run past the window's right edge, it hangs left from the pair's end.
    let width = style::VAR_MENU_WIDTH.to_pixels(window.rem_size());
    let at_edge = anchor.left() + width > window.viewport_size().width;
    let (corner, at) = if at_edge {
        (Anchor::TopRight, anchor.bottom_right())
    } else {
        (Anchor::TopLeft, anchor.bottom_left())
    };
    (
        with_list(root, open, (corner, at), &state, source, Mode::Vars, cx),
        true,
    )
}

/// Give the header cell `root` (whose id is `id`) its list of names or values for the text
/// typed. It stays closed while a `{{` is open at the cursor, which belongs to the variable
/// list. Returns whether it shows this frame.
pub(crate) fn attach_hints(
    root: Stateful<Div>,
    id: &ElementId,
    source: &Source,
    hint: &Hint,
    window: &mut Window,
    cx: &mut App,
) -> (Stateful<Div>, bool) {
    let key = ElementId::NamedChild(Arc::new(id.clone()), "hint-complete".into());
    let state = window.use_keyed_state(key, cx, |_, _| ListState::default());
    let text = source.text(cx);
    let caret = source.caret(window, cx);
    let in_placeholder = caret.is_some_and(|c| query_at(&text, c).is_some());
    // The whole cell is the query, so an accepted text replaces all of it.
    let query = caret.filter(|_| !in_placeholder).map(|_| Query {
        start: 0,
        replace: 0..text.len(),
        typed: text.to_string(),
    });
    let Some(query) = follow_query(&state, &text, query, cx) else {
        return (root, false);
    };
    let shown: Rc<[Choice]> = offers(hint, &query.typed)
        .into_iter()
        .map(|name| Choice {
            name,
            scope: "",
            preview: String::new(),
            dynamic: false,
        })
        .collect();
    let Some(open) = open_with(&state, query, shown, cx) else {
        return (root, false);
    };
    let Some(anchor) = source.bounds(&open.query.replace, cx) else {
        return (root, false);
    };
    let width = style::VAR_MENU_WIDTH.to_pixels(window.rem_size());
    let at_edge = anchor.left() + width > window.viewport_size().width;
    let (corner, at) = if at_edge {
        (Anchor::TopRight, anchor.bottom_right())
    } else {
        (Anchor::TopLeft, anchor.bottom_left())
    };
    (
        with_list(root, open, (corner, at), &state, source, Mode::Hints, cx),
        true,
    )
}

/// The choices of `vars`, from `state`'s cache while the set and its revision hold.
fn cached_choices(
    state: &Entity<ListState>,
    vars: &Arc<VariableSet>,
    cx: &mut App,
) -> Rc<[Choice]> {
    let revision = vars.revision();
    if let Some((v, r, c)) = &state.read(cx).choices
        && Arc::ptr_eq(v, vars)
        && *r == revision
    {
        return Rc::clone(c);
    }
    let found: Rc<[Choice]> = choices(vars).into();
    state.update(cx, |s, _| {
        s.choices = Some((Arc::clone(vars), revision, Rc::clone(&found)))
    });
    found
}

/// Bring the list in line with this frame's text and cursor; `Some` when it shows. It
/// opens when typing leaves the cursor after `{{` and a partial name, follows that partial
/// name, and closes when the cursor leaves it or nothing matches. Choices are built only
/// while it is open, so fields without a `{{` never resolve names.
fn sync(
    state: &Entity<ListState>,
    text: &SharedString,
    query: Option<Query>,
    vars: &Arc<VariableSet>,
    cx: &mut App,
) -> Option<OpenMenu> {
    let query = follow_query(state, text, query, cx)?;
    let all = cached_choices(state, vars, cx);
    let shown: Rc<[Choice]> = filter(&all, &query.typed).into_iter().cloned().collect();
    open_with(state, query, shown, cx)
}

/// The query the list is open on this frame, or `None` while it is closed. The list opens
/// when typing leaves the cursor in a query, follows it, and closes with it.
fn follow_query(
    state: &Entity<ListState>,
    text: &SharedString,
    query: Option<Query>,
    cx: &mut App,
) -> Option<Query> {
    state.update(cx, |s, _| {
        let typed = s.last_text.as_ref().is_some_and(|t| t != text);
        s.last_text = Some(text.clone());
        let Some(query) = query else {
            s.menu = None;
            s.dismissed = None;
            return None;
        };
        if s.dismissed.is_some_and(|d| d != query.start) {
            s.dismissed = None;
        }
        match &mut s.menu {
            Some(m) if m.start == query.start => {
                if m.typed != query.typed {
                    m.typed = query.typed.clone();
                    m.selected = 0;
                }
            }
            _ if typed && s.dismissed.is_none() => {
                s.menu = Some(Menu {
                    start: query.start,
                    typed: query.typed.clone(),
                    selected: 0,
                })
            }
            _ => {
                s.menu = None;
                return None;
            }
        }
        Some(query)
    })
}

/// The open list for `query` showing `shown`, with the highlight kept inside it.
fn open_with(
    state: &Entity<ListState>,
    query: Query,
    shown: Rc<[Choice]>,
    cx: &mut App,
) -> Option<OpenMenu> {
    if shown.is_empty() {
        return None;
    }
    let selected = state.update(cx, |s, _| {
        let m = s.menu.as_mut()?;
        m.selected = m.selected.min(shown.len() - 1);
        Some(m.selected)
    })?;
    Some(OpenMenu {
        query,
        shown,
        selected,
    })
}

/// A row's right-hand detail: `Scope · preview`, the scope alone for an empty value, and
/// nothing for a dynamic name.
fn detail(choice: &Choice) -> String {
    if choice.dynamic {
        String::new()
    } else if choice.preview.is_empty() {
        choice.scope.to_owned()
    } else {
        format!("{} · {}", choice.scope, choice.preview)
    }
}

/// Accept the highlighted row: close the list, then replace the partial name at the cursor
/// with it. The query is read again now, since the text may have moved since the frame.
fn accept_selected(
    state: &Entity<ListState>,
    source: &Source,
    shown: &[Choice],
    mode: Mode,
    window: &mut Window,
    cx: &mut App,
) {
    let text = source.text(cx);
    if mode == Mode::Hints {
        // The accepted text is a whole header name or value, so the list stays closed until
        // the cell loses focus; otherwise `Accept` would offer `Accept-Encoding` at once.
        let Some(ix) = state.update(cx, |s, _| {
            s.dismissed = Some(0);
            s.menu.take().map(|m| m.selected)
        }) else {
            return;
        };
        let (range, insert, cursor) = header_complete::accept(&text, &shown[ix].name);
        source.edit(range, insert, cursor, window, cx);
        return;
    }
    let Some(query) = source
        .caret(window, cx)
        .and_then(|caret| query_at(&text, caret))
    else {
        return;
    };
    let Some(ix) = state.update(cx, |s, _| s.menu.take().map(|m| m.selected)) else {
        return;
    };
    let (range, insert, cursor) = accept(&text, &query, &shown[ix].name);
    source.edit(range, insert, cursor, window, cx);
}

/// Move the highlight one row, wrapping, and scroll it into view. The scroll handle counts
/// the headings too: one before the first row, and one more before the dynamic rows.
fn step(state: &Entity<ListState>, shown: &[Choice], down: bool, mode: Mode, cx: &mut App) {
    let n = shown.len();
    state.update(cx, |s, cx| {
        if let Some(m) = &mut s.menu {
            m.selected = if down {
                (m.selected + 1) % n
            } else {
                (m.selected + n - 1) % n
            };
            // Only the variable list has headings.
            let child = match mode {
                Mode::Vars => {
                    m.selected + 1 + usize::from(!shown[0].dynamic && shown[m.selected].dynamic)
                }
                Mode::Hints => m.selected,
            };
            s.scroll.scroll_to_item(child);
        }
        cx.notify();
    });
}

/// The field with its list shown under the `{{`, and the list's keys taken from the field
/// in the capture phase so the input never sees them.
fn with_list(
    root: Stateful<Div>,
    open: OpenMenu,
    (corner, at): (Anchor, Point<Pixels>),
    state: &Entity<ListState>,
    source: &Source,
    mode: Mode,
    cx: &App,
) -> Stateful<Div> {
    let OpenMenu {
        shown, selected, ..
    } = open;
    let accept_with = {
        let (state, source, shown) = (state.clone(), source.clone(), Rc::clone(&shown));
        Rc::new(move |window: &mut Window, cx: &mut App| {
            accept_selected(&state, &source, &shown, mode, window, cx)
        })
    };
    let mut rows: Vec<AnyElement> = Vec::new();
    for (ix, choice) in shown.iter().enumerate() {
        // Each group opens with its heading: the filter keeps variables before dynamic names.
        if mode == Mode::Vars && (ix == 0 || shown[ix - 1].dynamic != choice.dynamic) {
            let (key, heading) = if choice.dynamic {
                (1usize, "Dynamic")
            } else {
                (0usize, "Variables")
            };
            rows.push(
                div()
                    .id(("var-complete-heading", key))
                    .test_support()
                    .aria_label(heading)
                    .px(style::SPACE_2)
                    .py(style::SPACE_0)
                    .text_color(cx.theme().muted_foreground)
                    .child(heading)
                    .into_any_element(),
            );
        }
        let detail = detail(choice);
        let label = if detail.is_empty() {
            choice.name.clone()
        } else {
            format!("{}, {detail}", choice.name)
        };
        // A click selects the row and accepts it; stopping the event keeps the field's focus.
        let on_click = {
            let (state, accept_with) = (state.clone(), Rc::clone(&accept_with));
            move |_: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                state.update(cx, |s, _| {
                    if let Some(m) = &mut s.menu {
                        m.selected = ix;
                    }
                });
                accept_with(window, cx);
                cx.stop_propagation();
            }
        };
        rows.push(
            div()
                .id((mode.row_id(), ix))
                .test_support()
                .role(Role::ListBoxOption)
                .aria_label(label)
                .aria_selected(ix == selected)
                .flex()
                .justify_between()
                .gap(style::SPACE_2)
                .px(style::SPACE_2)
                .py(style::SPACE_0)
                // The palette's opaque selected-row fill; the hover fill is too faint on a popover.
                .when(ix == selected, |d| {
                    d.bg(cx.theme().list_active)
                        .text_color(cx.theme().foreground)
                })
                .on_mouse_down(MouseButton::Left, on_click)
                .child(choice.name.clone())
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(detail),
                )
                .into_any_element(),
        );
    }
    let scroll = state.read(cx).scroll.clone();
    let list = div()
        .id(mode.list_id())
        .test_support()
        .role(Role::ListBox)
        .aria_label(mode.label())
        .w(style::VAR_MENU_WIDTH)
        .max_h(style::VAR_MENU_MAX_HEIGHT)
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .py(style::SPACE_1)
        .rounded(style::POPOVER_RADIUS)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .text_size(style::TEXT_SMALL)
        .children(rows);
    let (down, up) = ((state.clone(), Rc::clone(&shown)), (state.clone(), shown));
    let (enter, tab) = (Rc::clone(&accept_with), accept_with);
    let esc = state.clone();
    root.capture_action(move |_: &MoveDown, _, cx| {
        step(&down.0, &down.1, true, mode, cx);
        cx.stop_propagation();
    })
    .capture_action(move |_: &MoveUp, _, cx| {
        step(&up.0, &up.1, false, mode, cx);
        cx.stop_propagation();
    })
    // Only a plain Enter accepts, so Ctrl+Enter still sends the request.
    .capture_action(move |a: &Enter, window, cx| {
        if !a.secondary && !a.shift {
            enter(window, cx);
            cx.stop_propagation();
        }
    })
    .capture_action(move |_: &IndentInline, window, cx| {
        tab(window, cx);
        cx.stop_propagation();
    })
    .capture_action(move |_: &Escape, _, cx| {
        esc.update(cx, |s, cx| {
            s.dismissed = s.menu.take().map(|m| m.start);
            cx.notify();
        });
        cx.stop_propagation();
    })
    .child(deferred(
        anchored()
            .anchor(corner)
            .position(at)
            .snap_to_window()
            .child(list),
    ))
}
