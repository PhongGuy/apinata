//! The Vim key map of the code editors: a Normal-mode state machine over plain text, and the
//! [`Vim`] entity that feeds it the keys of one editor and applies what it returns.
//!
//! Normal mode reads keys in a keystroke interceptor, which runs before any binding, so the
//! editor's own Enter, Tab and Backspace never edit there. Insert mode leaves every key to
//! the editor; only Escape, bound in [`KEY_CONTEXT`], returns to Normal.

use std::ops::Range;

use gpui_kit::component::input::{EditorState, Undo};
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable as _, InteractiveElement, KeyContext,
    KeystrokeEvent, Subscription, WeakEntity, Window, actions,
};

use crate::settings::EditorKeymap;

/// The key context around a code editor while the Vim key map is on.
pub const KEY_CONTEXT: &str = "VimEditor";

/// The predicate Escape binds to [`EnterNormal`] in.
pub const INSERT_PREDICATE: &str = "VimEditor && vim_mode == insert";

actions!(
    vim,
    [
        /// Leaves Insert mode for Normal mode.
        EnterNormal,
    ]
);

/// The two Vim modes the editors offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VimMode {
    /// Keys are commands and motions.
    Normal,
    /// Keys type text.
    Insert,
}

/// One change the state machine asks of the editor, in UTF-8 byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VimOp {
    /// Put the caret at this offset.
    MoveTo(usize),
    /// Replace `range` with `text`, then put the caret at `cursor`.
    Edit {
        /// The bytes replaced.
        range: Range<usize>,
        /// What replaces them.
        text: String,
        /// Where the caret goes after the edit.
        cursor: usize,
    },
    /// Undo the last edit.
    Undo,
}

/// Normal-mode parsing: the mode, a count being typed and an operator awaiting its motion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VimState {
    mode: VimMode,
    count: Option<u32>,
    pending: Option<char>,
}

impl Default for VimState {
    fn default() -> Self {
        Self::new()
    }
}

impl VimState {
    /// A state in Normal mode with nothing typed.
    pub fn new() -> Self {
        VimState {
            mode: VimMode::Normal,
            count: None,
            pending: None,
        }
    }

    /// The current mode.
    pub fn mode(&self) -> VimMode {
        self.mode
    }

    /// Reads one Normal-mode key (a character, or a key name such as `escape`) with the caret
    /// at `cursor` in `text`, and returns the edits and moves it makes.
    pub fn feed(&mut self, key: &str, text: &str, cursor: usize) -> Vec<VimOp> {
        let mut chars = key.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            self.count = None;
            self.pending = None;
            return Vec::new();
        };
        let cursor = cursor.min(text.len());
        // A count is digits before the command; a leading 0 is the line-start motion.
        if let Some(digit) = ch.to_digit(10)
            && (digit != 0 || self.count.is_some())
        {
            let count = self.count.unwrap_or(0).saturating_mul(10) + digit;
            self.count = Some(count.min(MAX_COUNT));
            return Vec::new();
        }
        if let Some(operator) = self.pending.take() {
            let count = self.count.take();
            return match (operator, ch) {
                ('g', 'g') => vec![VimOp::MoveTo(go_to_line(text, count.unwrap_or(1)))],
                ('d', 'd') => delete_lines(text, cursor, count.unwrap_or(1)),
                _ => Vec::new(),
            };
        }
        if matches!(ch, 'g' | 'd') {
            self.pending = Some(ch);
            return Vec::new();
        }
        let count = self.count.take();
        let n = count.unwrap_or(1);
        let start = line_start(text, cursor);
        let end = line_end(text, cursor);
        let repeat = |step: &dyn Fn(usize) -> usize| (0..n).fold(cursor, |at, _| step(at));
        let to = |at: usize| vec![VimOp::MoveTo(at)];
        match ch {
            'h' => to(repeat(&|at| if at > start { prev(text, at) } else { at })),
            'l' => {
                let last = last_char(text, start, end);
                to(repeat(&|at| if at < last { next(text, at) } else { at }))
            }
            'j' => to(vertical(text, cursor, i64::from(n))),
            'k' => to(vertical(text, cursor, -i64::from(n))),
            '0' => to(start),
            '^' => to(first_non_blank(text, start)),
            '$' => to(last_char(text, start, end)),
            'w' => to(clamp_end(text, repeat(&|at| word_forward(text, at)))),
            'b' => to(repeat(&|at| word_backward(text, at))),
            'e' => to(clamp_end(text, repeat(&|at| word_end(text, at)))),
            'G' => to(go_to_line(text, count.unwrap_or(u32::MAX))),
            'x' => delete_chars(text, cursor, start, end, n),
            'D' if cursor < end => {
                let caret = if cursor > start {
                    prev(text, cursor)
                } else {
                    cursor
                };
                vec![edit(cursor..end, "", caret)]
            }
            'u' => (0..n).map(|_| VimOp::Undo).collect(),
            'i' => self.insert(Vec::new()),
            'a' => self.insert(to(if cursor < end {
                next(text, cursor)
            } else {
                cursor
            })),
            'A' => self.insert(to(end)),
            'I' => self.insert(to(first_non_blank(text, start))),
            'o' => self.insert(vec![edit(end..end, "\n", end + 1)]),
            'O' => self.insert(vec![edit(start..start, "\n", start)]),
            _ => Vec::new(),
        }
    }

    /// Leaves Insert mode with the caret at `cursor`, stepping back one character as Vim does.
    pub fn leave_insert(&mut self, text: &str, cursor: usize) -> Vec<VimOp> {
        self.mode = VimMode::Normal;
        self.count = None;
        self.pending = None;
        let cursor = cursor.min(text.len());
        let at = if cursor > line_start(text, cursor) {
            prev(text, cursor)
        } else {
            cursor
        };
        vec![VimOp::MoveTo(at)]
    }

    fn insert(&mut self, ops: Vec<VimOp>) -> Vec<VimOp> {
        self.mode = VimMode::Insert;
        ops
    }
}

// A count past this is a typo, and capping it keeps the repeat loops short.
const MAX_COUNT: u32 = 10_000;

fn edit(range: Range<usize>, text: &str, cursor: usize) -> VimOp {
    VimOp::Edit {
        range,
        text: text.to_string(),
        cursor,
    }
}

// Characters fall in three classes, and a word is a run of one class other than space.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class_at(text: &str, at: usize) -> Class {
    match text[at..].chars().next() {
        Some(c) if c.is_whitespace() => Class::Space,
        Some(c) if c.is_alphanumeric() || c == '_' => Class::Word,
        Some(_) => Class::Punct,
        None => Class::Space,
    }
}

fn prev(text: &str, at: usize) -> usize {
    text[..at]
        .chars()
        .next_back()
        .map_or(at, |c| at - c.len_utf8())
}

fn next(text: &str, at: usize) -> usize {
    text[at..].chars().next().map_or(at, |c| at + c.len_utf8())
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

// The offset of the line's newline, or the text's end on the last line.
fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

// Normal mode's caret sits on a character, so the last one of a non-empty line.
fn last_char(text: &str, start: usize, end: usize) -> usize {
    if end > start { prev(text, end) } else { start }
}

fn first_non_blank(text: &str, start: usize) -> usize {
    let end = line_end(text, start);
    text[start..end]
        .char_indices()
        .find(|(_, c)| !matches!(c, ' ' | '\t'))
        .map_or(last_char(text, start, end), |(i, _)| start + i)
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

// The first non-blank of line `line` (1-based), held to the last line.
fn go_to_line(text: &str, line: u32) -> usize {
    let starts = line_starts(text);
    let ix = (line.max(1) as usize - 1).min(starts.len() - 1);
    first_non_blank(text, starts[ix])
}

// Moves `delta` lines, keeping the caret's column where the target line is long enough.
fn vertical(text: &str, cursor: usize, delta: i64) -> usize {
    let starts = line_starts(text);
    let line = starts.partition_point(|&s| s <= cursor) - 1;
    let column = text[starts[line]..cursor].chars().count();
    let last = starts.len() as i64 - 1;
    let target = (line as i64 + delta).clamp(0, last) as usize;
    let start = starts[target];
    let end = line_end(text, start);
    let at = text[start..end]
        .char_indices()
        .nth(column)
        .map_or(end, |(i, _)| start + i);
    at.min(last_char(text, start, end))
}

fn word_forward(text: &str, at: usize) -> usize {
    let mut i = at;
    let class = class_at(text, i);
    if class != Class::Space {
        while i < text.len() && class_at(text, i) == class {
            i = next(text, i);
        }
    }
    while i < text.len() && class_at(text, i) == Class::Space {
        i = next(text, i);
    }
    i
}

fn word_backward(text: &str, at: usize) -> usize {
    let mut i = at;
    while i > 0 && class_at(text, prev(text, i)) == Class::Space {
        i = prev(text, i);
    }
    if i == 0 {
        return 0;
    }
    let class = class_at(text, prev(text, i));
    while i > 0 && class_at(text, prev(text, i)) == class {
        i = prev(text, i);
    }
    i
}

fn word_end(text: &str, at: usize) -> usize {
    let mut i = next(text, at);
    while i < text.len() && class_at(text, i) == Class::Space {
        i = next(text, i);
    }
    if i >= text.len() {
        return i;
    }
    let class = class_at(text, i);
    while next(text, i) < text.len() && class_at(text, next(text, i)) == class {
        i = next(text, i);
    }
    i
}

// A motion run off the end stops on the text's last character.
fn clamp_end(text: &str, at: usize) -> usize {
    if at >= text.len() {
        prev(text, text.len())
    } else {
        at
    }
}

// Deletes up to `n` characters of the line from the caret; the caret stays on the line.
fn delete_chars(text: &str, cursor: usize, start: usize, end: usize, n: u32) -> Vec<VimOp> {
    if cursor >= end {
        return Vec::new();
    }
    let to = (0..n).fold(cursor, |at, _| if at < end { next(text, at) } else { at });
    let new_end = end - (to - cursor);
    let caret = if cursor < new_end || cursor == start {
        cursor
    } else {
        prev(text, cursor)
    };
    vec![edit(cursor..to, "", caret)]
}

// Deletes `n` whole lines from the caret's; past the last line it also takes the newline
// before them, so no empty line is left behind.
fn delete_lines(text: &str, cursor: usize, n: u32) -> Vec<VimOp> {
    let starts = line_starts(text);
    let line = starts.partition_point(|&s| s <= cursor) - 1;
    let last = (line + n as usize - 1).min(starts.len() - 1);
    let mut from = starts[line];
    let to = starts.get(last + 1).copied().unwrap_or(text.len());
    let at_end = to == text.len() && from > 0;
    if at_end {
        from -= 1;
    }
    let rest = format!("{}{}", &text[..from], &text[to..]);
    let line_at = if at_end {
        line_start(&rest, from)
    } else {
        from.min(rest.len())
    };
    vec![edit(from..to, "", first_non_blank(&rest, line_at))]
}

/// The Vim key map of one code editor; inert while Preferences picks another key map.
pub struct Vim {
    editor: WeakEntity<EditorState>,
    focus: FocusHandle,
    state: Option<VimState>,
    _subs: Vec<Subscription>,
}

impl Vim {
    /// Follows the Preferences key map for `editor`, reading its keys while it holds focus.
    pub fn new(editor: &Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        let this = cx.entity().downgrade();
        let mut subs = vec![cx.intercept_keystrokes(move |event, window, cx| {
            if let Some(this) = this.upgrade() {
                this.update(cx, |vim, cx| vim.intercept(event, window, cx));
            }
        })];
        if let Some(app) = cx.try_global::<crate::app::AppState>() {
            let settings = app.settings.clone();
            subs.push(cx.observe(&settings, |vim, _, cx| {
                let on = crate::settings::editor_keymap(cx) == EditorKeymap::Vim;
                vim.set_enabled(on, cx);
            }));
        }
        let on = crate::settings::editor_keymap(cx) == EditorKeymap::Vim;
        Vim {
            editor: editor.downgrade(),
            focus: editor.read(cx).focus_handle(cx),
            state: on.then(VimState::new),
            _subs: subs,
        }
    }

    /// The editor's mode, or `None` while the key map is not Vim.
    pub fn mode(&self) -> Option<VimMode> {
        self.state.as_ref().map(VimState::mode)
    }

    /// Turns the Vim key map on, in Normal mode, or off; a repeat keeps the current mode.
    pub fn set_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        if on != self.state.is_some() {
            self.state = on.then(VimState::new);
            cx.notify();
        }
    }

    // In Normal mode every unmodified key except Escape and the arrow-like keys is a command,
    // so none of them reaches the editor; modified keys stay with the app's shortcuts.
    fn intercept(&mut self, event: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.mode() != VimMode::Normal || !self.focus.is_focused(window) {
            return;
        }
        let keystroke = &event.keystroke;
        let m = keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return;
        }
        let key = match keystroke.key.as_str() {
            "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown" => return,
            "escape" => {
                state.feed("escape", "", 0);
                return;
            }
            name => keystroke
                .key_char
                .clone()
                .filter(|c| c.chars().count() == 1)
                .unwrap_or_else(|| name.to_string()),
        };
        cx.stop_propagation();
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let (text, cursor) = {
            let e = editor.read(cx);
            (e.value().to_string(), e.cursor())
        };
        let before = state.mode();
        let ops = state.feed(&key, &text, cursor);
        let changed = state.mode() != before;
        apply(&editor, ops, window, cx);
        if changed {
            cx.notify();
        }
    }

    /// Returns to Normal mode from Insert mode; does nothing in Normal mode or with Vim off.
    pub fn leave_insert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.mode() != VimMode::Insert {
            return;
        }
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let (text, cursor) = {
            let e = editor.read(cx);
            (e.value().to_string(), e.cursor())
        };
        let ops = state.leave_insert(&text, cursor);
        apply(&editor, ops, window, cx);
        cx.notify();
    }
}

// Edits go through the editor's own replace, one undo step each; a read-only editor only moves.
fn apply(editor: &Entity<EditorState>, ops: Vec<VimOp>, window: &mut Window, cx: &mut App) {
    for op in ops {
        match op {
            VimOp::MoveTo(at) => editor.update(cx, |e, cx| e.set_selected_range(at..at, cx)),
            VimOp::Edit {
                range,
                text,
                cursor,
            } => editor.update(cx, |e, cx| {
                if e.is_editable() {
                    e.set_selected_range(range, cx);
                    e.replace(text, window, cx);
                    e.set_selected_range(cursor..cursor, cx);
                }
            }),
            VimOp::Undo => window.dispatch_action(Box::new(Undo), cx),
        }
    }
}

/// Puts `element` in the Vim key context with its mode, while the key map is Vim, and
/// handles Insert mode's Escape there. Wrap the element that holds the editor.
pub fn scope<E: InteractiveElement>(element: E, vim: &Entity<Vim>, cx: &App) -> E {
    let mut element = match vim.read(cx).mode() {
        Some(mode) => {
            let mut context = KeyContext::default();
            context.add(KEY_CONTEXT);
            let name = match mode {
                VimMode::Normal => "normal",
                VimMode::Insert => "insert",
            };
            context.set("vim_mode", name);
            element.key_context(context)
        }
        None => element,
    };
    let vim = vim.clone();
    element = element.on_action(move |_: &EnterNormal, window, cx| {
        vim.update(cx, |v, cx| v.leave_insert(window, cx));
    });
    element
}

/// A small label naming the editor's Vim mode, with element id `id`; `None` while the key
/// map is not Vim.
pub fn mode_label(id: &'static str, vim: &Entity<Vim>, cx: &App) -> Option<gpui_kit::AnyElement> {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{IntoElement as _, ParentElement as _, Styled as _, TestSupportExt as _, div};
    let label = match vim.read(cx).mode()? {
        VimMode::Normal => "NORMAL",
        VimMode::Insert => "INSERT",
    };
    Some(
        div()
            .id(id)
            .test_support()
            .text_size(crate::style::TEXT_SMALL)
            .text_color(cx.theme().muted_foreground)
            .child(label)
            .into_any_element(),
    )
}
