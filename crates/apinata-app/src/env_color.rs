//! An environment's colour as drawn outside its editor, and the contrast rule for text on it.

use gpui_kit::component::h_flex;
use gpui_kit::{
    AnyElement, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, TestSupportExt as _, div, hsla,
};

use crate::style::{self, SPACE_1};
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

/// `name` led by a dot of `fill` (`dot_id`), kept on one line so a long name still ellipsizes.
pub fn dotted(dot_id: &'static str, fill: Hsla, name: SharedString) -> impl IntoElement {
    h_flex()
        .min_w_0()
        .gap(SPACE_1)
        .child(dot(dot_id, fill))
        .child(div().min_w_0().truncate().child(name))
}

/// A picker row's content: `name` (`name_id`) led by its dot (`dot_id`) when it has a `fill`,
/// or by a dot's space when another row has one (`any`), so every name starts in line.
pub fn picker_row(
    (dot_id, name_id): (&'static str, &'static str),
    fill: Option<Hsla>,
    any: bool,
    name: SharedString,
) -> impl IntoElement {
    let dot = (fill.is_some() || any).then(|| dot_or_space(dot_id, fill));
    h_flex().min_w_0().gap(SPACE_1).children(dot).child(
        div()
            .id(name_id)
            .test_support()
            .min_w_0()
            .truncate()
            .child(name),
    )
}
