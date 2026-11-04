//! An open `ui::select` draws the fill `theme::select_open_fill` names, so the contrast test
//! measures what the user sees. Painted quads are read, not the style the code intended.

use crate::support;

use apinata_app::theme::{HC_DARK, select_open_fill};
use apinata_app::ui;
use gpui_kit::AppContext as _;
use gpui_kit::ParentElement as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Context, Hsla, IntoElement, Render, SharedString, Styled as _, TestAppContext,
    Window, WindowOptions, div, open_window,
};

struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels: Vec<SharedString> = vec!["One".into(), "Two".into()];
        div().size_full().child(ui::select(
            "sel",
            "Choice",
            "One".into(),
            labels,
            0,
            |_, _, _| {},
            cx,
        ))
    }
}

fn open_select(cx: &mut TestAppContext, theme: &str) -> AnyWindowHandle {
    support::init(cx);
    support::use_theme(cx, theme);
    let (win, _view) = cx
        .update(|cx| open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Host)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("sel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    win
}

/// The solid fills painted over exactly the select button's bounds.
fn fills_over_button(cx: &mut TestAppContext, win: AnyWindowHandle) -> Vec<Hsla> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let scale = w.scale_factor();
        let b = w.find("sel").bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        w.painted_quads()
            .into_iter()
            .filter(|q| {
                (q.bounds.origin.x.0 - l).abs() < 1. && (q.bounds.origin.y.0 - t).abs() < 1.
            })
            .filter_map(|q| q.background.as_solid())
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn an_open_select_draws_the_open_fill_in_high_contrast_dark(cx: &mut TestAppContext) {
    let win = open_select(cx, HC_DARK);
    let want = cx
        .update(|cx| select_open_fill(cx.theme()))
        .expect("High Contrast Dark overrides the kit's open fill");
    let fills = fills_over_button(cx, win);
    assert!(
        fills.contains(&want),
        "open select painted {fills:?}, wanted {want:?}"
    );
}
