//! An environment's colour as drawn outside its editor, and the contrast rule for text on it.

use gpui_kit::{
    AnyElement, ElementId, Hsla, InteractiveElement as _, IntoElement, Styled as _,
    TestSupportExt as _, div, hsla,
};

use crate::style;
use crate::ui::luminance_ratio;

/// The WCAG AA ratio text drawn on an environment colour must reach.
pub const TEXT_CONTRAST: f32 = 4.5;

/// The fill for an environment colour, or none when there is no colour or it is no hex colour.
pub fn env_fill(color: Option<&str>) -> Option<Hsla> {
    crate::ui::parse_color(color?.trim())
}

/// `text` when it reaches 4.5:1 on `fill`, else black or white, whichever contrasts more;
/// one of the two always passes.
pub fn text_on(fill: Hsla, text: Hsla) -> Hsla {
    if luminance_ratio(text, fill) >= TEXT_CONTRAST {
        return text;
    }
    let (black, white) = (hsla(0., 0., 0., 1.), hsla(0., 0., 1., 1.));
    if luminance_ratio(black, fill) >= luminance_ratio(white, fill) {
        black
    } else {
        white
    }
}

/// A small round dot of `fill` registered under `id`; decorative, so it carries no label.
pub fn dot(id: impl Into<ElementId>, fill: Hsla) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .flex_none()
        .size(style::DOT)
        .rounded_full()
        .bg(fill)
}

/// A dot of `fill`, or bare space of a dot's size without one, so names beside it line up.
pub fn dot_or_space(id: impl Into<ElementId>, fill: Option<Hsla>) -> AnyElement {
    match fill {
        Some(fill) => dot(id, fill).into_any_element(),
        None => div().flex_none().size(style::DOT).into_any_element(),
    }
}
