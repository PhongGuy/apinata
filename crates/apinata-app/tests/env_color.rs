//! The active environment's colour beyond its swatches: a stripe along the window's top edge
//! and a dot on the environment select, plus the contrast rule for text drawn on the colour.

use crate::support;

use apinata_app::command::{Command, Edit, Target};
use apinata_app::env_color::{env_fill, text_on};
use apinata_app::env_selector::option_id;
use apinata_app::style::{ENV_STRIPE_HEIGHT, at_zoom};
use gpui_kit::component::Colorize as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Hsla, TestAppContext, hsla, px};
use support::*;

fn ratio(a: Hsla, b: Hsla) -> f32 {
    let lum = |c: Hsla| {
        let c = c.to_rgb();
        let ch = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn hex(text: &str) -> Hsla {
    Hsla::parse_hex(text).unwrap()
}

#[test]
fn a_hex_colour_fills_and_anything_else_does_not() {
    assert_eq!(env_fill(Some("#ce4f3b")), Some(hex("#ce4f3b")));
    assert_eq!(env_fill(Some(" #CE4F3B ")), Some(hex("#ce4f3b")));
    assert_eq!(env_fill(Some("crimson")), None);
    assert_eq!(env_fill(Some("")), None);
    assert_eq!(env_fill(None), None);
}

#[test]
fn readable_text_keeps_its_colour_on_a_contrasting_fill() {
    let white = hsla(0., 0., 1., 1.);
    let navy = hex("#102040");
    assert_eq!(text_on(navy, white), white);
}

#[test]
fn unreadable_text_falls_back_to_a_neutral_that_passes() {
    for (fill, text) in [
        ("#808080", "#909090"),
        ("#ce4f3b", "#ce4f3b"),
        ("#ffffff", "#eeeeee"),
        ("#000000", "#222222"),
        ("#1f6feb", "#ffffff"),
    ] {
        let (fill, text) = (hex(fill), hex(text));
        let shown = text_on(fill, text);
        assert!(ratio(shown, fill) >= 4.5, "{shown:?} on {fill:?}");
        if ratio(text, fill) < 4.5 {
            let rgb = shown.to_rgb();
            assert!(
                rgb.r == rgb.g && rgb.g == rgb.b,
                "the fallback is a neutral grey, not {shown:?}"
            );
        }
    }
}

const DEV: &str = "name: dev\ncolor: \"#ce4f3b\"\n";

fn coloured() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/plain.yml", "name: plain\n"),
        ("environments/prod.yml", "name: prod\ncolor: \"#2f9e44\"\n"),
    ])
}

fn select(cx: &mut TestAppContext, s: &Started, name: Option<&str>) {
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, name.map(str::to_owned), cx)
    });
    frame(cx, s.win);
}

fn stripe(cx: &mut TestAppContext, s: &Started) -> Option<Hsla> {
    frame(cx, s.win);
    let drawn = present(cx, s.win, "env-stripe");
    let colour = s.shell.read_with(cx, |v, cx| v.env_stripe(cx));
    assert_eq!(
        drawn,
        colour.is_some(),
        "the drawn stripe matches its colour"
    );
    colour
}

#[gpui_kit::test]
fn no_environment_draws_no_stripe(cx: &mut TestAppContext) {
    let col = coloured();
    let s = started(cx, &col);
    assert_eq!(stripe(cx, &s), None);
}

#[gpui_kit::test]
fn a_coloured_environment_draws_a_thin_stripe_across_the_top(cx: &mut TestAppContext) {
    let col = coloured();
    let s = started(cx, &col);
    select(cx, &s, Some("dev"));
    assert_eq!(stripe(cx, &s), Some(hex("#ce4f3b")));
    cx.update_window(s.win, |_, w, _| {
        let bounds = w.find("env-stripe").bounds();
        let height = at_zoom(ENV_STRIPE_HEIGHT, 1.0);
        assert!((px(2.)..=px(3.)).contains(&height), "{height:?}");
        assert_eq!(bounds.origin.y, px(0.));
        assert_eq!(bounds.origin.x, px(0.));
        assert_eq!(bounds.size.height, height);
        assert_eq!(bounds.size.width, w.viewport_size().width);
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_environment_without_a_colour_draws_no_stripe(cx: &mut TestAppContext) {
    let col = coloured();
    let s = started(cx, &col);
    select(cx, &s, Some("plain"));
    assert_eq!(stripe(cx, &s), None);
}

#[gpui_kit::test]
fn the_stripe_follows_a_switch_and_a_colour_edit(cx: &mut TestAppContext) {
    let col = coloured();
    let s = started(cx, &col);
    select(cx, &s, Some("dev"));
    assert_eq!(stripe(cx, &s), Some(hex("#ce4f3b")));
    select(cx, &s, Some("prod"));
    assert_eq!(stripe(cx, &s), Some(hex("#2f9e44")));
    let recolour = |c: Option<&str>| Command::Edit {
        target: Target::Environment("prod".into()),
        edit: Edit::Color(c.map(str::to_owned)),
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, recolour(Some("#1f6feb")), cx));
    frame(cx, s.win);
    assert_eq!(stripe(cx, &s), Some(hex("#1f6feb")));
    s.ws.update(cx, |w, cx| w.send(&s.root, recolour(None), cx));
    frame(cx, s.win);
    assert_eq!(stripe(cx, &s), None);
    select(cx, &s, None);
    assert_eq!(stripe(cx, &s), None);
}

#[gpui_kit::test]
fn the_trigger_shows_a_dot_for_a_coloured_choice(cx: &mut TestAppContext) {
    let col = coloured();
    let s = started(cx, &col);
    let sel = s.shell.read_with(cx, |v, _| v.env_selector().clone());
    assert!(!present(cx, s.win, "env-trigger-dot"));
    select(cx, &s, Some("dev"));
    assert!(present(cx, s.win, "env-trigger-dot"));
    cx.update_window(s.win, |_, w, _| {
        assert!(w.within("env-select").try_find("env-trigger-dot").is_some());
    })
    .unwrap();
    assert_eq!(sel.read_with(cx, |v, _| v.dot("dev")), Some(hex("#ce4f3b")));
    select(cx, &s, Some("plain"));
    assert!(!present(cx, s.win, "env-trigger-dot"));
}

#[gpui_kit::test]
fn the_dot_leaves_the_accessible_value_alone(cx: &mut TestAppContext) {
    let col = coloured();
    let s = started(cx, &col);
    select(cx, &s, Some("dev"));
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("env-select").value(), Some("dev"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn dropdown_rows_show_a_dot_only_when_coloured(cx: &mut TestAppContext) {
    let col = coloured();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click("env-select", cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        for (name, dotted) in [("dev", true), ("prod", true), ("plain", false)] {
            let found = w.within(option_id(name)).try_find("env-row-dot").is_some();
            assert_eq!(found, dotted, "{name}");
        }
        assert!(w.within("env-none").try_find("env-row-dot").is_none());
    })
    .unwrap();
    let sel = s.shell.read_with(cx, |v, _| v.env_selector().clone());
    assert_eq!(
        sel.read_with(cx, |v, _| v.dot("prod")),
        Some(hex("#2f9e44"))
    );
    assert_eq!(sel.read_with(cx, |v, _| v.dot("plain")), None);
}
