//! Paints the Compare tab's rows: section titles, notes, status, header and JSON changes, and
//! the shared numbered diff rows for a text body.

use std::rc::Rc;

use apinata_core::{HeaderChangeKind, JsonChangeKind};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, FontWeight, Hsla, SharedString, TestSupportExt, Window, div};

use crate::body_view::SCRUBBED_TIP;
use crate::compare_view::{CompareRow, HeaderEye};
use crate::diff_doc::{DiffRow, RowKind};
use crate::diff_rows::{DiffPalette, diff_row};
use crate::kv_grid::{ActionShow, action_slot};
use crate::style;
use crate::theme::{self, Status};
use crate::ui::{self, LockButton as _};

/// What a click on the volatile-headers row does.
pub type Toggle = Rc<dyn Fn(&mut Window, &mut App)>;

/// What a click on a header row's eye does, given the header's name.
pub type RevealHeader = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// The clicks a row can answer.
#[derive(Clone)]
pub struct RowActions {
    /// Opens or folds the volatile headers.
    pub toggle: Toggle,
    /// Reveals or masks one header row.
    pub reveal: RevealHeader,
}

/// The theme reads every row of one comparison shares.
#[derive(Clone)]
pub struct ComparePalette {
    diff: DiffPalette,
    muted: Hsla,
    added: Hsla,
    removed: Hsla,
    changed: Hsla,
    changed_glyph: Hsla,
}

impl ComparePalette {
    /// Reads the theme; `diff_rows` size the line-number gutter of a text diff.
    pub fn new(diff_rows: &[DiffRow], window: &Window, cx: &App) -> ComparePalette {
        ComparePalette {
            diff: DiffPalette::new(diff_rows, window, cx),
            muted: cx.theme().muted_foreground,
            added: theme::diff_fill(Status::Success, cx),
            removed: theme::diff_fill(Status::Danger, cx),
            changed: theme::status_fill(Status::Warning, cx),
            changed_glyph: theme::diff_glyph(Status::Warning, cx),
        }
    }

    /// The fill behind an added row.
    pub fn added(&self) -> Hsla {
        self.added
    }

    /// The fill behind a removed row.
    pub fn removed(&self) -> Hsla {
        self.removed
    }

    /// The colour a row's +/- mark draws in.
    pub fn glyph_color(&self, kind: RowKind) -> Hsla {
        self.diff.glyph_color(kind)
    }

    /// The colour the "~" mark of a changed row draws in.
    pub fn changed_glyph(&self) -> Hsla {
        self.changed_glyph
    }

    /// The fill behind a changed-value row.
    pub fn changed(&self) -> Hsla {
        self.changed
    }
}

// The mark and tint a change reads with: added green, removed red, changed amber.
fn mark(kind: Option<bool>) -> &'static str {
    match kind {
        Some(true) => "+",
        Some(false) => "-",
        None => "~",
    }
}

// The mark as its own bold span in the change's accent, so it reads against the wash.
fn change_mark(ix: usize, kind: Option<bool>, p: &ComparePalette) -> impl IntoElement + use<> {
    let color = match kind {
        Some(true) => p.diff.glyph_color(RowKind::Added),
        Some(false) => p.diff.glyph_color(RowKind::Removed),
        None => p.changed_glyph,
    };
    div()
        .id(("compare-mark", ix))
        .test_support()
        .flex_none()
        // A fixed column so the text after +, - and ~ starts at one x.
        .w(style::SPACE_3)
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .child(mark(kind))
}

/// The volatile-headers toggle text, with the noun and verb agreeing with `count`.
pub fn volatile_label(count: usize, open: bool) -> String {
    let verb = if open { "Hide" } else { "Show" };
    match count {
        1 => format!("{verb} 1 header that changes on every send"),
        n => format!("{verb} {n} headers that change on every send"),
    }
}

// The Headers tab's eye at a header row's end: one button across the flip, keyed by row, that
// reveals or masks both sides; a locked one only explains why it cannot.
fn header_eye(ix: usize, name: &str, eye: HeaderEye, actions: &RowActions, cx: &App) -> AnyElement {
    let (face, icon, words) = match eye {
        HeaderEye::Hidden => ("compare-reveal", IconName::Eye, "Show value"),
        HeaderEye::Revealed => ("compare-hide", IconName::EyeOff, "Hide value"),
        HeaderEye::Locked => ("compare-reveal", IconName::Eye, SCRUBBED_TIP),
    };
    let button = ui::swap_icon_button(("compare-eye", ix), (face, ix), icon, words);
    let button = match eye {
        HeaderEye::Locked => button.lock(true, cx),
        _ => {
            let (reveal, name) = (Rc::clone(&actions.reveal), name.to_owned());
            button.on_click(move |_, window, cx| reveal(&name, window, cx))
        }
    };
    action_slot(button, ActionShow::Always)
}

/// One row; `ix` keys its element ids and `actions` answer its clicks.
pub fn compare_row(
    ix: usize,
    row: &CompareRow,
    p: &ComparePalette,
    actions: &RowActions,
    cx: &App,
) -> AnyElement {
    let line = || {
        h_flex()
            .id(("compare-row", ix))
            .test_support()
            .w_full()
            .px(style::SPACE_1)
            .gap(style::SPACE_2)
            .whitespace_nowrap()
    };
    match row {
        CompareRow::Group(title) => line()
            .pt(style::SPACE_2)
            .font_weight(FontWeight::SEMIBOLD)
            .child(*title)
            .into_any_element(),
        CompareRow::Note(text) => line()
            .text_color(p.muted)
            .child(text.clone())
            .into_any_element(),
        CompareRow::Status { old, new } => {
            line().child(format!("{old} -> {new}")).into_any_element()
        }
        CompareRow::Header {
            kind,
            name,
            old,
            new,
            eye,
        } => {
            let (tint, flag, text) = match kind {
                HeaderChangeKind::Added => (p.added, Some(true), format!("{name}: {new}")),
                HeaderChangeKind::Removed => (p.removed, Some(false), format!("{name}: {old}")),
                _ => (p.changed, None, format!("{name}: {old} -> {new}")),
            };
            // The label reads what is drawn, so a masked row names no real value.
            let row = line()
                .aria_label(SharedString::from(text.clone()))
                .bg(tint)
                .child(change_mark(ix, flag, p));
            match eye {
                Some(eye) => row
                    .child(div().flex_1().min_w_0().overflow_hidden().child(text))
                    .child(header_eye(ix, name, *eye, actions, cx))
                    .into_any_element(),
                None => row.child(text).into_any_element(),
            }
        }
        CompareRow::Volatile { count, open } => {
            let label = volatile_label(*count, *open);
            line()
                .id(("compare-volatile", ix))
                .test_support()
                .cursor_pointer()
                .text_color(p.muted)
                .on_click({
                    let toggle = Rc::clone(&actions.toggle);
                    move |_, window, cx| toggle(window, cx)
                })
                .child(label)
                .into_any_element()
        }
        CompareRow::Json {
            kind,
            path,
            old,
            new,
        } => {
            let shown = |v: &Option<String>| v.clone().unwrap_or_default();
            let (tint, flag, text) = match kind {
                JsonChangeKind::Added => (p.added, Some(true), shown(new)),
                JsonChangeKind::Removed => (p.removed, Some(false), shown(old)),
                _ => (p.changed, None, format!("{} -> {}", shown(old), shown(new))),
            };
            line()
                .bg(tint)
                .child(change_mark(ix, flag, p))
                .child(format!("{path}: {text}"))
                .into_any_element()
        }
        CompareRow::Diff(r) => diff_row(ix, r, &p.diff).into_any_element(),
    }
}
