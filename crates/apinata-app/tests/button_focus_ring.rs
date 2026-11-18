//! Keyboard focus on a kit button draws a solid ring at 3:1 against the surface behind it in
//! both High Contrast themes, as the app's own controls do.

use crate::support;

use apinata_app::focus_mode::{Modality, set};
use apinata_app::request_editor::Section;
use apinata_app::theme;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Hsla, TestAppContext};
use support::*;

/// `top` laid over `under`, as the renderer blends it.
fn over(top: Hsla, under: Hsla) -> Hsla {
    let (t, u) = (top.to_rgb(), under.to_rgb());
    let mix = |a: f32, b: f32| a * top.a + b * (1. - top.a);
    gpui_kit::Rgba {
        r: mix(t.r, u.r),
        g: mix(t.g, u.g),
        b: mix(t.b, u.b),
        a: 1.,
    }
    .into()
}

/// What keyboard focus on `id` draws: the border colour of every quad outlining it in the
/// `ring` hue at any alpha, and the opaque surface painted under it.
fn ring_and_surface(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: &str,
    ring: Hsla,
) -> (Vec<Hsla>, Hsla) {
    let id = id.to_string();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find(id).bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        let (r, bt) = (b.right().as_f32() * scale, b.bottom().as_f32() * scale);
        let quads = w.painted_quads();
        let rings = quads
            .iter()
            .filter(|q| {
                let (x, y) = (q.bounds.origin.x.0, q.bounds.origin.y.0);
                let (x2, y2) = (x + q.bounds.size.width.0, y + q.bounds.size.height.0);
                q.border_widths.left.0 > 0.
                    && q.border_color.alpha(1.) == ring.alpha(1.)
                    && x <= l + 1.
                    && y <= t + 1.
                    && x2 >= r - 1.
                    && y2 >= bt - 1.
                    && (x - l).abs() < 6. * scale
                    && (y - t).abs() < 6. * scale
            })
            .map(|q| q.border_color)
            .collect();
        // The surface is the last opaque fill that reaches past the ring on every side.
        let reach = 6. * scale;
        let surface = quads
            .iter()
            .filter(|q| {
                let (x, y) = (q.bounds.origin.x.0, q.bounds.origin.y.0);
                let (x2, y2) = (x + q.bounds.size.width.0, y + q.bounds.size.height.0);
                x <= l - reach && y <= t - reach && x2 >= r + reach && y2 >= bt + reach
            })
            .filter_map(|q| {
                q.background
                    .as_solid()
                    .filter(|c| c.a >= 1.)
                    .map(|c| (q.order, c))
            })
            .max_by_key(|(order, _)| *order)
            .map(|(_, c)| c)
            .expect("no surface under the button");
        (rings, surface)
    })
    .unwrap()
}

// Tab onto a Tests snippet button, a ghost icon button and a filled primary one: each draws
// its ring opaque and at 3:1 over the surface behind it, in both High Contrast themes.
#[gpui_kit::test]
fn a_tab_focused_button_rings_opaque_at_three_to_one(cx: &mut TestAppContext) {
    use gpui_kit::component::ActiveTheme as _;
    let (mut seen_all, mut bad) = (Vec::new(), false);
    for name in [theme::HC_DARK, theme::HC_LIGHT] {
        for id in ["snippet-status-200", "generate-code-button", "send-button"] {
            let col = write_tree(&[
                ("opencollection.yml", ROOT),
                ("a.yml", &request("a", "http://x")),
            ]);
            let (_state, ws, root) = open_dir(cx, &col);
            let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
            use_theme(cx, name);
            ed.update(cx, |e, cx| e.show(Section::Tests, cx));
            frame(cx, win);
            cx.update(|cx| set(Modality::Keyboard, cx));
            tab_to(cx, win, id);
            frame(cx, win);
            let ring = cx.update(|cx| cx.theme().ring);
            let (rings, surface) = ring_and_surface(cx, win, id, ring);
            if rings.is_empty() {
                seen_all.push(format!("{name} {id}: no ring"));
                bad = true;
            }
            for ring in rings {
                let seen = contrast(over(ring, surface), surface);
                seen_all.push(format!("{name} {id}: alpha {} = {seen:.2}", ring.a));
                bad |= ring.a < 1. || seen < 3.0;
            }
        }
    }
    assert!(!bad, "a translucent or faint ring: {seen_all:#?}");
}
