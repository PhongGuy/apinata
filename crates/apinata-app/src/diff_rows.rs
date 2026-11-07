//! The line-diff row renderer shared by the Changes dialog and the Compare tab.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, FontWeight, Hsla, Pixels, SharedString, TestSupportExt, Window, div};

use crate::body_view::{gutter_digits, mono_gutter};
use crate::diff_doc::{DiffRow, RowKind};
use crate::style;
use crate::theme::{self, Status};

/// The theme reads and gutter width every row of one diff shares.
#[derive(Clone)]
pub struct DiffPalette {
    mono: SharedString,
    mono_size: Pixels,
    muted: Hsla,
    foreground: Hsla,
    added_glyph: Hsla,
    removed_glyph: Hsla,
    added: Hsla,
    removed: Hsla,
    number: Pixels,
}

impl DiffPalette {
    /// Reads the theme and sizes the line-number gutter for the widest number in `rows`.
    pub fn new(rows: &[DiffRow], window: &Window, cx: &App) -> DiffPalette {
        let theme = cx.theme();
        let (mono, mono_size) = (theme.mono_font_family.clone(), theme.mono_font_size);
        let widest = rows
            .iter()
            .filter_map(|r| r.old.max(r.new))
            .max()
            .unwrap_or(0);
        let number = mono_gutter(gutter_digits(widest), mono_size, window, cx);
        DiffPalette {
            mono,
            mono_size,
            muted: theme.muted_foreground,
            foreground: theme.foreground,
            added_glyph: theme::diff_glyph(Status::Success, cx),
            removed_glyph: theme::diff_glyph(Status::Danger, cx),
            added: theme::diff_fill(Status::Success, cx),
            removed: theme::diff_fill(Status::Danger, cx),
            number,
        }
    }
}

impl DiffPalette {
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
        match kind {
            RowKind::Added => self.added_glyph,
            RowKind::Removed => self.removed_glyph,
            _ => self.muted,
        }
    }

    /// The colour a row's line numbers draw in: full foreground on a tint, muted elsewhere.
    pub fn number_color(&self, kind: RowKind) -> Hsla {
        match kind {
            RowKind::Added | RowKind::Removed => self.foreground,
            _ => self.muted,
        }
    }
}

/// One numbered, tinted diff row; `ix` keys its element ids.
pub fn diff_row(ix: usize, row: &DiffRow, palette: &DiffPalette) -> impl IntoElement + use<> {
    let (muted, number) = (palette.muted, palette.number);
    let number_color = palette.number_color(row.kind);
    let glyph_color = palette.glyph_color(row.kind);
    let cell = |side: &'static str, n: Option<usize>| {
        h_flex()
            .flex_none()
            .w(number)
            .pr(style::SPACE_2)
            .justify_end()
            .text_color(number_color)
            .child(
                div()
                    .id((side, ix))
                    .test_support()
                    .child(n.map(|n| n.to_string()).unwrap_or_default()),
            )
    };
    h_flex()
        .id(("diff-line", ix))
        .test_support()
        // The tint spans the pane, not just the text; the inset keeps a
        // short line number off the pane's left border.
        .min_w_full()
        .pl(style::SPACE_1)
        .whitespace_nowrap()
        .font_family(palette.mono.clone())
        .text_size(palette.mono_size)
        .map(|r| match row.kind {
            RowKind::Added => r.bg(palette.added),
            RowKind::Removed => r.bg(palette.removed),
            RowKind::Header | RowKind::NoNewline => r.text_color(muted),
            RowKind::Context => r,
        })
        .child(cell("diff-old", row.old))
        .child(cell("diff-new", row.new))
        .child(
            div()
                .id(("diff-gutter", ix))
                .test_support()
                .flex_none()
                .w(style::SPACE_3)
                .text_color(glyph_color)
                .font_weight(FontWeight::BOLD)
                .child(row.kind.gutter()),
        )
        .child(row.text.clone())
}
