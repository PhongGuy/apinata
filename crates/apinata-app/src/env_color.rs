//! An environment's colour as drawn outside its editor, the contrast rule for text on it, and
//! the pieces the title bar's environment pickers share.

use gpui_kit::assets::IconName;
use gpui_kit::component::select::{SelectDelegate, SelectItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IndexPath, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div,
    hsla, rems,
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

/// A picker trigger's title: its scope `icon` (`icon_id`) in the text colour, then `name`, led
/// by a dot (`dot_id`) when it has a `fill`; kept on one line so a long name still ellipsizes.
pub fn scoped(
    (icon_id, icon): (&'static str, IconName),
    (dot_id, fill): (&'static str, Option<Hsla>),
    name: SharedString,
) -> impl IntoElement {
    h_flex()
        .min_w_0()
        .gap(SPACE_1)
        .child(
            div()
                .id(icon_id)
                .test_support()
                .flex_none()
                .child(Icon::new(icon).xsmall()),
        )
        .children(fill.map(|fill| dot(dot_id, fill)))
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
    // The end padding keeps a cut name's ellipsis off the kit's check mark.
    h_flex()
        .min_w_0()
        .gap(SPACE_1)
        .pr(SPACE_1)
        .children(dot)
        .child(
            div()
                .id(name_id)
                .test_support()
                .min_w_0()
                .truncate()
                .child(name),
        )
}

/// A picker's rows in titled groups. A group's header (`(id, group index)`) reads as a label
/// rather than a row: smaller muted text, and a rule above every group after the first.
#[derive(Clone)]
pub struct PickerGroups<I> {
    id: &'static str,
    groups: Vec<(SharedString, Vec<I>)>,
}

impl<I> PickerGroups<I> {
    /// `groups` as (header, rows) pairs, in order; `id` names the headers.
    pub fn new(id: &'static str, groups: Vec<(SharedString, Vec<I>)>) -> Self {
        PickerGroups { id, groups }
    }
}

impl<I: SelectItem + 'static> SelectDelegate for PickerGroups<I> {
    type Item = I;

    fn sections_count(&self, _: &App) -> usize {
        self.groups.len()
    }

    fn items_count(&self, section: usize) -> usize {
        self.groups.get(section).map_or(0, |(_, rows)| rows.len())
    }

    fn item(&self, ix: IndexPath) -> Option<&I> {
        self.groups.get(ix.section)?.1.get(ix.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        I: SelectItem<Value = V>,
        V: PartialEq,
    {
        self.groups
            .iter()
            .enumerate()
            .find_map(|(section, (_, rows))| {
                let row = rows.iter().position(|r| r.value() == value)?;
                Some(IndexPath::default().section(section).row(row))
            })
    }

    // The list sizes every header alike, so the first keeps a clear rule of the same width.
    fn render_section_header(
        &self,
        section: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let (title, _) = self.groups.get(section)?;
        let theme = cx.theme();
        let rule = if section == 0 {
            theme.transparent
        } else {
            theme.border
        };
        let header = div()
            .id((self.id, section))
            .test_support()
            .aria_label(title.clone())
            .border_t_1()
            .border_color(rule)
            .pt_0p5()
            .px_2()
            .text_xs()
            .line_height(rems(1.))
            .text_color(theme.muted_foreground)
            .child(title.clone());
        Some(header.into_any_element())
    }
}
