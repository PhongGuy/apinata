//! The focus ring of the title-bar and status-bar controls: solid, opaque, and 3:1 against
//! the bar it is drawn on, in both themes, drawn for keyboard focus only.

use crate::git_support;
use crate::support;

use apinata_app::focus_mode::{Modality, set};
use git_support::{Sandbox, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, BorderStyle, Hsla, TestAppContext};
use support::*;

fn luminance(c: Hsla) -> f32 {
    let rgb = c.to_rgb();
    let lin = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb.r) + 0.7152 * lin(rgb.g) + 0.0722 * lin(rgb.b)
}

fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

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

/// The ring quads painted around element `id` of `scope` right now: (colour, style).
fn rings_around(
    cx: &mut TestAppContext,
    s: &Started,
    scope: &'static str,
    id: &'static str,
) -> Vec<(Hsla, BorderStyle)> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.within(scope).find(id).bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        let (r, bt) = (b.right().as_f32() * scale, b.bottom().as_f32() * scale);
        w.painted_quads()
            .into_iter()
            .filter(|q| {
                let (x, y) = (q.bounds.origin.x.0, q.bounds.origin.y.0);
                let (x2, y2) = (x + q.bounds.size.width.0, y + q.bounds.size.height.0);
                q.border_widths.left.0 > 0.
                    && x <= l + 1.
                    && y <= t + 1.
                    && x2 >= r - 1.
                    && y2 >= bt - 1.
                    && (x - l).abs() < 6. * scale
                    && (y - t).abs() < 6. * scale
            })
            .map(|q| (q.border_color, q.border_style))
            .collect()
    })
    .unwrap()
}

pub fn one_changed(cx: &mut TestAppContext, sb: &Sandbox) -> (tempfile::TempDir, Started) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    sb.git(col.path(), &["init", "-q", "-b", "main"]);
    sb.commit_all(col.path(), "init");
    write(col.path(), "a.yml", &request("a", "http://x/changed"));
    let s = started(cx, &col);
    let options = sb.options();
    s.ws.update(cx, |w, cx| w.set_git(Some(options), cx));
    frame(cx, s.win);
    (col, s)
}

const CONTROLS: [(&str, &str); 10] = [
    ("title-bar", "git-branch"),
    ("title-bar", "git-changes"),
    ("title-bar", "collection-switcher"),
    ("title-bar", "env-select"),
    ("title-bar", "global-env-select"),
    ("title-bar", "environments"),
    ("title-bar", "app-menu"),
    ("status-bar", "toggle-sidebar"),
    ("status-bar", "toggle-console"),
    ("status-bar", "toggle-layout"),
];

// Keyboard focus on each control draws the theme's ring colour whole (opaque), solid, and at
// 3:1 against the bar behind it, as a tab's ring does.
#[gpui_kit::test]
fn bar_controls_draw_a_solid_ring_at_three_to_one(cx: &mut TestAppContext) {
    use apinata_app::theme::set_mode;
    use gpui_kit::component::{ActiveTheme as _, ThemeMode};
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|cx| {
            set_mode(mode, None, cx);
            set(Modality::Keyboard, cx);
        });
        let (ring, title, status) = cx.update(|cx| {
            let t = cx.theme();
            (t.ring, t.title_bar, t.status_bar)
        });
        for (scope, id) in CONTROLS {
            tab_to(cx, s.win, id);
            let rings = rings_around(cx, &s, scope, id);
            assert!(!rings.is_empty(), "{mode:?} {id}: no ring is drawn");
            let bar = if scope == "title-bar" { title } else { status };
            for (colour, style) in rings {
                assert_eq!(style, BorderStyle::Solid, "{mode:?} {id}");
                assert_eq!(colour, ring, "{mode:?} {id}: not the tab ring colour");
                let seen = contrast(over(colour, bar), bar);
                assert!(seen >= 3.0, "{mode:?} {id}: ring on its bar = {seen:.2}");
            }
        }
    }
}

// A mouse pick leaves no ring on a bar control, the environment selects included.
#[gpui_kit::test]
fn a_mouse_press_leaves_no_ring_on_bar_controls(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    for (scope, id) in CONTROLS {
        tab_to(cx, s.win, id);
        cx.update(|cx| set(Modality::Pointer, cx));
        let rings = rings_around(cx, &s, scope, id);
        assert!(
            rings.is_empty(),
            "{id}: a ring after a mouse press: {rings:?}"
        );
        cx.update(|cx| set(Modality::Keyboard, cx));
    }
}

// The ring's border takes its layout room whether or not it shows, so a control and what sits
// beside it do not move when keyboard focus arrives.
#[gpui_kit::test]
fn bar_controls_keep_their_place_when_the_ring_appears(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let (_col, s) = one_changed(cx, &sb);
    cx.update(|cx| set(Modality::Keyboard, cx));
    let places = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let bar = w.within("title-bar");
            CONTROLS
                .iter()
                .filter(|(scope, _)| *scope == "title-bar")
                .map(|(_, id)| bar.find(*id).bounds())
                .collect::<Vec<_>>()
        })
        .unwrap()
    };
    for (scope, id) in CONTROLS {
        let before = places(cx);
        tab_to(cx, s.win, id);
        assert_eq!(
            places(cx),
            before,
            "{scope} {id}: the bar moved under the ring"
        );
    }
}
