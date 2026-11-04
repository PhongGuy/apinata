//! The cue a disabled control carries in the High Contrast themes, where it cannot dim: the
//! muted foreground, and a dim edge never heavier than the enabled one's (a filled button's is
//! inset; ghost and text buttons stay edgeless). Nothing disabled is dashed. The standard
//! themes keep their dimmed look.

use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
use apinata_app::ui::{
    LockButton as _, checkbox_look, disabled_edge, lock_look, menu_item, menu_item_look,
};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::Button;
use gpui_kit::{BorderStyle, Styled as _};

const HIGH: [&str; 2] = [HC_LIGHT, HC_DARK];

/// What a real `Button` of `variant` (0 default, 3 outline) carries for its own text colour
/// and border colour.
fn button_style(
    variant: u8,
    disabled: bool,
    cx: &gpui_kit::App,
) -> (Option<gpui_kit::Hsla>, Option<gpui_kit::Hsla>) {
    let b = Button::new("b").label("Go");
    let b = if variant == 3 { b.outline() } else { b };
    let mut b = b.lock(disabled, cx);
    let s = b.style();
    (s.text.color, s.border_color)
}

#[gpui_kit::test]
fn a_disabled_edged_button_has_the_dim_edge_and_a_muted_label_in_high_contrast_only(
    cx: &mut gpui_kit::TestAppContext,
) {
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            for variant in [0, 3] {
                let enabled = button_style(variant, false, cx);
                let disabled = button_style(variant, true, cx);
                assert_eq!(
                    enabled.1, None,
                    "{name}/{variant}: enabled keeps the kit's border"
                );
                if HIGH.contains(&name) {
                    assert_eq!(disabled.1, Some(dim_edge(cx)), "{name}/{variant}: dim edge");
                    assert_eq!(
                        disabled.0,
                        Some(cx.theme().muted_foreground),
                        "{name}/{variant}: muted"
                    );
                } else {
                    assert_eq!(
                        disabled.1, None,
                        "{name}/{variant}: standard themes keep the kit's border"
                    );
                }
            }
        });
    }
}

/// The disabled button edge is non-text UI, so it clears 3:1 on every surface a button sits
/// on, and never out-weighs the enabled `input` edge there.
#[gpui_kit::test]
fn the_disabled_button_edge_clears_three_to_one_and_is_never_heavier(
    cx: &mut gpui_kit::TestAppContext,
) {
    for name in HIGH {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            let edge = button_style(0, true, cx).1.expect("a disabled edge");
            for under in [t.background, t.sidebar, t.popover] {
                let (r, full) = (contrast(edge, under), contrast(t.input, under));
                assert!(r >= 3.0, "{name}: edge on surface = {r:.2}");
                assert!(
                    r <= full,
                    "{name}: disabled edge {r:.2} over enabled {full:.2}"
                );
            }
        });
    }
}

/// A button variant the painted probe draws.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Variant {
    Default,
    Outline,
    Primary,
    Ghost,
    Text,
}

/// What one button paints alone: its solid visible edges as (colour, left width) and the size
/// of its widest quad, which is the button's own box.
fn painted_button(
    cx: &mut gpui_kit::TestAppContext,
    variant: Variant,
    disabled: bool,
) -> (Vec<(gpui_kit::Hsla, f32)>, (f32, f32)) {
    use gpui_kit::component::button::ButtonVariants as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Context, IntoElement, Render, Window, WindowOptions, open_window,
    };
    struct Host(Variant, bool);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let (variant, d) = (self.0, self.1);
            let b = Button::new("b").label("Go");
            match variant {
                Variant::Default => b.lock(d, cx),
                Variant::Outline => b.outline().lock(d, cx),
                Variant::Primary => b.primary().lock_filled(d, cx),
                Variant::Ghost => b.ghost().lock(d, cx),
                Variant::Text => b.text().lock(d, cx),
            }
        }
    }
    let win = cx
        .update(|cx| {
            open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Host(variant, disabled))
            })
        })
        .unwrap()
        .0;
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let quads = w.painted_quads();
        // GPUI paints one edge as a quad per side, all alike, so they collapse to one entry.
        let mut edges: Vec<_> = quads
            .iter()
            .filter(|q| {
                q.border_style == BorderStyle::Solid
                    && q.border_widths.left.0 > 0.
                    && q.border_color.a > 0.
            })
            .map(|q| (q.border_color, q.border_widths.left.0))
            .collect();
        edges.dedup();
        let widest = quads
            .iter()
            .map(|q| (q.bounds.size.width.0, q.bounds.size.height.0))
            .fold((0., 0.), |a: (f32, f32), b| if b.0 > a.0 { b } else { a });
        (edges, widest)
    })
    .unwrap()
}

/// In High Contrast a disabled default or outline button paints one solid edge in the dim
/// colour, as wide as its enabled edge; a disabled ghost or text button paints none.
#[gpui_kit::test]
fn disabled_high_contrast_buttons_paint_the_dim_edge_only_where_enabled_ones_have_one(
    cx: &mut gpui_kit::TestAppContext,
) {
    crate::support::init(cx);
    for name in HIGH {
        crate::support::use_theme(cx, name);
        let dim = cx.update(|cx| dim_edge(cx));
        for variant in [Variant::Default, Variant::Outline] {
            let (on, _) = painted_button(cx, variant, false);
            let (off, _) = painted_button(cx, variant, true);
            assert_eq!(on.len(), 1, "{name}/{variant:?}: enabled edge {on:?}");
            assert_eq!(
                off,
                vec![(dim, on[0].1)],
                "{name}/{variant:?}: disabled edge"
            );
        }
        for variant in [Variant::Ghost, Variant::Text] {
            for disabled in [false, true] {
                let (edges, _) = painted_button(cx, variant, disabled);
                assert!(edges.is_empty(), "{name}/{variant:?}/{disabled}: {edges:?}");
            }
        }
    }
}

/// One edge colour serves disabled buttons and checkboxes, so the two cannot drift apart.
#[gpui_kit::test]
fn the_shared_disabled_edge_is_the_checkbox_edge_in_high_contrast(
    cx: &mut gpui_kit::TestAppContext,
) {
    for name in HIGH {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            assert_eq!(
                disabled_edge(cx),
                dim_edge(cx),
                "{name}: input toward the page"
            );
            let boxed = checkbox_look(false, true, cx).border;
            assert_eq!(boxed, disabled_edge(cx), "{name}: checkbox edge");
        });
    }
}

/// The shadows a primary button through `lock_filled` carries itself.
fn filled_shadow(disabled: bool, cx: &gpui_kit::App) -> Option<Vec<gpui_kit::BoxShadow>> {
    use gpui_kit::component::button::ButtonVariants as _;
    let mut b = Button::new("b")
        .label("Go")
        .primary()
        .lock_filled(disabled, cx);
    b.style().box_shadow.clone()
}

/// A disabled High Contrast primary keeps its shape with one 1px inset edge in the dim colour,
/// which takes no layout space; an enabled one carries no shadow and paints the same box.
#[gpui_kit::test]
fn a_disabled_high_contrast_primary_has_an_inset_dim_edge_and_keeps_its_width(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::{BoxShadow, px};
    crate::support::init(cx);
    for name in HIGH {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let inset = BoxShadow::new(px(0.), px(0.), disabled_edge(cx))
                .spread_radius(px(1.))
                .inset();
            assert_eq!(
                filled_shadow(true, cx),
                Some(vec![inset]),
                "{name}: inset edge"
            );
            assert_eq!(filled_shadow(false, cx), None, "{name}: enabled has none");
        });
        let (_, on) = painted_button(cx, Variant::Primary, false);
        let (_, off) = painted_button(cx, Variant::Primary, true);
        assert!(on.0 > 0., "{name}: the button paints a box");
        assert_eq!(on, off, "{name}: no width jump");
    }
}

/// The standard themes keep the kit's own disabled look from `lock` and `lock_filled` alike.
#[gpui_kit::test]
fn standard_themes_keep_the_kits_disabled_button_style(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::component::button::ButtonVariants as _;
    for name in [LIGHT, DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            for disabled in [false, true] {
                let mut plain = Button::new("b").label("Go").lock(disabled, cx);
                let mut filled = Button::new("b")
                    .label("Go")
                    .primary()
                    .lock_filled(disabled, cx);
                for (what, s) in [("lock", plain.style()), ("lock_filled", filled.style())] {
                    assert_eq!(
                        s.border_color, None,
                        "{name}/{what}/{disabled}: border colour"
                    );
                    assert_eq!(
                        s.border_widths,
                        Default::default(),
                        "{name}/{what}/{disabled}: border width"
                    );
                    assert_eq!(s.box_shadow, None, "{name}/{what}/{disabled}: shadow");
                }
            }
        });
    }
}

/// The edge a disabled checkbox box takes in High Contrast: `input` leaned toward the page.
fn dim_edge(cx: &gpui_kit::App) -> gpui_kit::Hsla {
    let t = cx.theme();
    t.input.blend(t.background.opacity(0.25))
}

#[gpui_kit::test]
fn a_disabled_checkbox_box_has_a_dim_solid_edge_in_high_contrast_only(
    cx: &mut gpui_kit::TestAppContext,
) {
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let on = checkbox_look(true, false, cx);
            let off = checkbox_look(true, true, cx);
            assert_eq!(on.border, cx.theme().input, "{name}: enabled edge");
            if HIGH.contains(&name) {
                assert_eq!(off.border, dim_edge(cx), "{name}: dim edge");
                assert_ne!(off.border, on.border, "{name}: edge differs");
                assert_ne!(on.tick, off.tick, "{name}: tick differs");
                assert_ne!(on.fill, off.fill, "{name}: fill differs");
            } else {
                assert_eq!(off.border, on.border, "{name}: unchanged edge");
                assert_eq!(off.opacity, 0.5, "{name}: still dimmed");
            }
        });
    }
}

/// The glyph a disabled off lock takes in High Contrast: the muted foreground leaned toward
/// the page, so it is not the enabled off glyph.
fn dim_glyph(cx: &gpui_kit::App) -> gpui_kit::Hsla {
    let t = cx.theme();
    t.muted_foreground.blend(t.background.opacity(0.25))
}

#[gpui_kit::test]
fn a_disabled_lock_toggle_has_no_outline_in_any_theme(cx: &mut gpui_kit::TestAppContext) {
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            for on in [true, false] {
                let disabled = lock_look(on, true, cx);
                assert_eq!(disabled.outline, None, "{name}: on={on} no outline");
                if HIGH.contains(&name) {
                    // An off glyph is dimmed; an on one is a chip whose fill is the border grey.
                    let shown = if on {
                        disabled.fill
                    } else {
                        Some(disabled.color)
                    };
                    let want = if on {
                        cx.theme().border.blend(cx.theme().foreground.opacity(0.15))
                    } else {
                        dim_glyph(cx)
                    };
                    assert_eq!(shown, Some(want), "{name}: on={on} muted");
                }
            }
            assert_eq!(lock_look(false, false, cx).outline, None, "{name}: off");
        });
    }
}

/// The four lock states must all differ in High Contrast: the glyph tells on from off, only the
/// enabled padlock is boxed, and a disabled one is muted.
#[gpui_kit::test]
fn the_four_lock_states_differ_in_high_contrast(cx: &mut gpui_kit::TestAppContext) {
    for name in HIGH {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let looks = [(true, false), (true, true), (false, false), (false, true)]
                .map(|(on, dis)| lock_look(on, dis, cx));
            for (i, a) in looks.iter().enumerate() {
                for b in &looks[i + 1..] {
                    assert_ne!(a, b, "{name}: two lock states look alike");
                }
            }
            let (on, on_dis) = (looks[0], looks[1]);
            assert_eq!(on.outline, Some(cx.theme().input), "{name}: on is boxed");
            assert_eq!(on_dis.outline, None, "{name}: disabled on has no box");
            assert_ne!(on.fill, on_dis.fill, "{name}: disabled on is muted");
            assert_eq!(looks[2].outline, None, "{name}: enabled off has no box");
            assert_eq!(looks[3].outline, None, "{name}: disabled off has no box");
            assert_ne!(
                looks[2].color, looks[3].color,
                "{name}: disabled off is dimmed"
            );
        });
    }
    // The standard themes draw every lock state exactly as before.
    for name in [LIGHT, DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            for on in [true, false] {
                for dis in [true, false] {
                    assert_eq!(lock_look(on, dis, cx).outline, None, "{name}: no outline");
                }
            }
        });
    }
}

/// A disabled menu entry is drawn unlike an enabled one in High Contrast only.
#[gpui_kit::test]
fn a_disabled_menu_item_is_drawn_unlike_an_enabled_one_in_high_contrast_only(
    cx: &mut gpui_kit::TestAppContext,
) {
    use gpui_kit::component::menu::PopupMenuItem;
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let (on, off) = (menu_item_look(false, cx), menu_item_look(true, cx));
            let built = menu_item("Rename", true, cx);
            let custom = matches!(built, PopupMenuItem::ElementItem { .. });
            assert!(!on.italic, "{name}: enabled upright");
            assert!(matches!(
                menu_item("Rename", false, cx),
                PopupMenuItem::Item { .. }
            ));
            if HIGH.contains(&name) {
                assert_ne!(on, off, "{name}: drawn differently");
                assert!(custom, "{name}: disabled item carries the look");
            } else {
                assert_eq!(on, off, "{name}: standard themes unchanged");
                assert!(!custom, "{name}: kit's own item");
            }
        });
    }
}

/// A control the painted-quad probe can draw, enabled or disabled (locked for the fields).
#[derive(Debug, Clone, Copy)]
enum Probe {
    /// A Button of variant 0 (default), 1 (primary), 2 (ghost) or 3 (outline).
    Button(u8),
    /// A checkbox: checked, unchecked or mixed.
    Checkbox { checked: bool, mixed: bool },
    /// A lock toggle, on or off.
    Lock(bool),
    /// An input through `lock_input`.
    Input,
    /// A textarea through `lock_textarea`.
    Textarea,
}

const PROBES: [Probe; 11] = [
    Probe::Button(0),
    Probe::Button(1),
    Probe::Button(2),
    Probe::Button(3),
    Probe::Checkbox {
        checked: true,
        mixed: false,
    },
    Probe::Checkbox {
        checked: false,
        mixed: false,
    },
    Probe::Checkbox {
        checked: false,
        mixed: true,
    },
    Probe::Lock(true),
    Probe::Lock(false),
    Probe::Input,
    Probe::Textarea,
];

/// Which painted quads are dashed, as (border colour, bounds), for one probe drawn alone.
fn dashed_quads(
    cx: &mut gpui_kit::TestAppContext,
    probe: Probe,
    disabled: bool,
) -> Vec<(gpui_kit::Hsla, f32, f32)> {
    use apinata_app::ui::{checkbox, lock_input, lock_textarea, lock_toggle};
    use gpui_kit::component::button::ButtonVariants as _;
    use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Context, Entity, IntoElement, Render, Window, WindowOptions, open_window,
    };
    struct Host {
        probe: Probe,
        disabled: bool,
        input: Entity<InputState>,
        area: Entity<TextareaState>,
    }
    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let d = self.disabled;
            match self.probe {
                Probe::Button(variant) => {
                    let b = Button::new("b").label("Go");
                    let b = match variant {
                        0 => b,
                        1 => b.primary(),
                        2 => b.ghost(),
                        _ => b.outline(),
                    };
                    b.lock(d, cx).into_any_element()
                }
                Probe::Checkbox { checked, mixed } => checkbox("c", checked)
                    .mixed(mixed)
                    .disabled(d)
                    .into_any_element(),
                Probe::Lock(on) => lock_toggle("l", on).disabled(d).into_any_element(),
                Probe::Input => lock_input(Input::new(&self.input), &self.input, d, window, cx)
                    .into_any_element(),
                Probe::Textarea => {
                    lock_textarea(Textarea::new(&self.area), &self.area, d, window, cx)
                        .into_any_element()
                }
            }
        }
    }
    let win = cx
        .update(|cx| {
            open_window(WindowOptions::default(), cx, |w, cx| {
                let input = cx.new(|cx| InputState::new(w, cx).default_value("value"));
                let area = cx.new(|cx| TextareaState::new(w, cx));
                cx.new(|_| Host {
                    probe,
                    disabled,
                    input,
                    area,
                })
            })
        })
        .unwrap()
        .0;
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.painted_quads()
            .into_iter()
            .filter(|q| q.border_style == BorderStyle::Dashed && q.border_widths.left.0 > 0.)
            .map(|q| {
                (
                    q.border_color,
                    q.bounds.size.width.0,
                    q.bounds.size.height.0,
                )
            })
            .collect::<Vec<_>>()
    })
    .map(|mut v| {
        // GPUI paints one dashed outline as a quad per side, all of the same bounds.
        v.dedup();
        v
    })
    .unwrap()
}

/// No button variant, checkbox, lock toggle, locked input or locked textarea paints a dashed
/// outline in any theme, disabled or not, so a disabled one never resembles a focus ring.
#[gpui_kit::test]
fn no_disabled_or_locked_control_draws_a_dashed_outline(cx: &mut gpui_kit::TestAppContext) {
    crate::support::init(cx);
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        for probe in PROBES {
            for disabled in [true, false] {
                let quads = dashed_quads(cx, probe, disabled);
                assert!(
                    quads.is_empty(),
                    "{name}/{probe:?}/{disabled}: dashed quads {quads:?}"
                );
            }
        }
    }
}

/// The disabled checkbox edge is non-text UI, so it clears 3:1 on every surface it sits on,
/// yet it never out-weighs the enabled edge there.
#[gpui_kit::test]
fn the_dim_edge_clears_three_to_one_and_is_never_heavier_in_high_contrast(
    cx: &mut gpui_kit::TestAppContext,
) {
    for name in HIGH {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            let enabled = checkbox_look(false, false, cx).border;
            let edge = checkbox_look(false, true, cx).border;
            for under in [t.background, t.sidebar, t.popover] {
                let (r, full) = (contrast(edge, under), contrast(enabled, under));
                assert!(r >= 3.0, "{name}: edge on surface = {r:.2}");
                assert!(
                    r <= full,
                    "{name}: disabled edge {r:.2} over enabled {full:.2}"
                );
            }
        });
    }
}

/// A disabled off lock's dimmed glyph still reads as a control on every surface.
#[gpui_kit::test]
fn a_disabled_off_lock_glyph_clears_three_to_one_in_high_contrast(
    cx: &mut gpui_kit::TestAppContext,
) {
    for name in HIGH {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            let glyph = lock_look(false, true, cx).color;
            for under in [t.background, t.sidebar, t.popover] {
                let r = contrast(glyph, under);
                assert!(r >= 3.0, "{name}: disabled off glyph on surface = {r:.2}");
            }
        });
    }
}

/// What a styled element carries for its font style and text colour.
fn cue_of<E: gpui_kit::Styled>(mut e: E) -> (Option<gpui_kit::FontStyle>, Option<gpui_kit::Hsla>) {
    let s = e.style();
    (s.text.font_style, s.text.color)
}

/// The label a disabled menu entry really draws is italic in the muted foreground in High
/// Contrast, and untouched elsewhere; the environment menu's locked Rename carries it too.
#[gpui_kit::test]
fn disabled_menu_labels_are_italic_and_muted_in_high_contrast_only(
    cx: &mut gpui_kit::TestAppContext,
) {
    use apinata_app::env_editor::locked_rename_label;
    use apinata_app::ui::menu_label_cue;
    use gpui_kit::{FontStyle, div};
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let muted = cx.theme().muted_foreground;
            let generic = cue_of(menu_label_cue(div(), cx));
            let rename = cue_of(locked_rename_label(cx));
            if HIGH.contains(&name) {
                assert_eq!(
                    generic,
                    (Some(FontStyle::Italic), Some(muted)),
                    "{name}: entry"
                );
                assert_eq!(
                    rename,
                    (Some(FontStyle::Italic), Some(muted)),
                    "{name}: Rename"
                );
            } else {
                assert_eq!(generic, (None, None), "{name}: entry unchanged");
                assert_eq!(rename, (None, None), "{name}: Rename unchanged");
            }
        });
    }
}

/// A menu the OS draws takes text only, so a disabled Copy there is marked in the label in
/// High Contrast; an enabled one and any standard-theme one keep the plain word.
#[gpui_kit::test]
fn a_disabled_native_menu_entry_is_marked_in_its_label_in_high_contrast_only(
    cx: &mut gpui_kit::TestAppContext,
) {
    use apinata_app::ui::native_menu_label;
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            assert_eq!(
                native_menu_label("Copy", false, cx),
                "Copy",
                "{name}: enabled"
            );
            let off = native_menu_label("Copy", true, cx);
            if HIGH.contains(&name) {
                assert_ne!(off, "Copy", "{name}: marked");
                assert!(off.starts_with("Copy"), "{name}: still names the entry");
            } else {
                assert_eq!(off, "Copy", "{name}: unchanged");
            }
        });
    }
}

/// WCAG contrast of two opaque colours.
fn contrast(a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> f64 {
    let lum = |c: gpui_kit::Hsla| {
        let c = c.to_rgb();
        let ch = |v: f64| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r as f64) + 0.7152 * ch(c.g as f64) + 0.0722 * ch(c.b as f64)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// A lock that is on is a filled box and one that is off is not, so on and off differ in the
/// drawn shape (not only the glyph) whether enabled or disabled. All four states differ with
/// the icon ignored, and the filled glyph reads at 4.5:1 on its fill.
#[gpui_kit::test]
fn a_secret_lock_is_filled_and_the_four_states_differ_without_the_icon(
    cx: &mut gpui_kit::TestAppContext,
) {
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let states = [(true, false), (true, true), (false, false), (false, true)]
                .map(|(on, dis)| (on, dis, lock_look(on, dis, cx)));
            for (on, dis, look) in states {
                assert_eq!(look.fill.is_some(), on, "{name}: on={on} dis={dis} fill");
            }
            for (i, (_, _, a)) in states.iter().enumerate() {
                for (_, _, b) in &states[i + 1..] {
                    let (mut a, b) = (*a, *b);
                    a.icon = b.icon;
                    assert_ne!(a, b, "{name}: two lock states look alike without the icon");
                }
            }
            let (on_dis, off_dis) = (states[1].2, states[3].2);
            assert!(
                on_dis.fill != off_dis.fill || on_dis.outline != off_dis.outline,
                "{name}: disabled on and off differ in shape"
            );
            let on = states[0].2;
            let fill = on.fill.expect("an enabled secret is filled");
            let ratio = contrast(on.color, fill);
            assert!(ratio >= 4.5, "{name}: filled glyph is {ratio:.2}:1");
            // The box must stand out from the surface it sits on, drawn at its effective
            // opacity; the glyph is part of the same dimmed group.
            let t = cx.theme();
            let high = HIGH.contains(&name);
            assert_ne!(on_dis.fill, off_dis.fill, "{name}: disabled fills differ");
            for (surface_name, surface) in [
                ("background", t.background),
                ("sidebar", t.sidebar),
                ("popover", t.popover),
            ] {
                for look in [on, on_dis] {
                    let f = look.fill.expect("on is filled");
                    let a = look.opacity;
                    let boxed = surface.blend(f.opacity(a));
                    let r = contrast(boxed, surface);
                    assert!(r >= 3.0, "{name}: fill on {surface_name} is {r:.2}:1");
                    let glyph = surface.blend(f.blend(look.color).opacity(a));
                    let g = contrast(glyph, boxed);
                    let floor = if look == on_dis && !high { 3.0 } else { 4.5 };
                    assert!(
                        g >= floor,
                        "{name}: glyph on fill {g:.2}:1 ({surface_name})"
                    );
                }
            }
        });
    }
}

/// In High Contrast a disabled secret lock is a grey chip: filled with the border grey leaned toward the text, so
/// it is neither the enabled chip nor the surface, with no box and a readable glyph.
/// Light and Dark keep the dimmed filled chip.
#[gpui_kit::test]
fn a_disabled_secret_lock_is_a_grey_chip_in_high_contrast_only(cx: &mut gpui_kit::TestAppContext) {
    for name in [LIGHT, DARK, HC_LIGHT, HC_DARK] {
        crate::support::use_theme(cx, name);
        cx.update(|cx| {
            let t = cx.theme();
            let enabled = lock_look(true, false, cx);
            let disabled = lock_look(true, true, cx);
            assert!(enabled.fill.is_some(), "{name}: enabled secret is a chip");
            if HIGH.contains(&name) {
                let fill = disabled.fill.expect("a disabled secret is a chip");
                let grey = t.border.blend(t.foreground.opacity(0.15));
                assert_eq!(fill, grey, "{name}: grey fill");
                assert_ne!(
                    Some(fill),
                    Some(t.foreground),
                    "{name}: not the enabled chip"
                );
                for (what, other) in [("foreground", t.foreground), ("background", t.background)] {
                    let r = contrast(fill, other);
                    assert!(r >= 1.5, "{name}: grey fill vs {what} is {r:.2}:1");
                }
                let g = contrast(disabled.color, fill);
                assert!(g >= 4.5, "{name}: glyph on grey fill is {g:.2}:1");
                assert_eq!(disabled.outline, None, "{name}: no box");
                assert_eq!(disabled.opacity, 1., "{name}: not dimmed");
                assert_eq!(lock_look(false, true, cx).fill, None, "{name}: off hollow");
            } else {
                assert_eq!(disabled.fill, Some(t.foreground), "{name}: still a chip");
                assert_eq!(disabled.opacity, 0.5, "{name}: still dimmed");
            }
        });
    }
}
