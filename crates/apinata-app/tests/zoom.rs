use crate::support;

use apinata_app::app::AppState;
use apinata_app::body_view::BodyView;
use apinata_app::persist::{Paths, save_json};
use apinata_app::settings::{Settings, ThemeChoice};
use apinata_app::zoom::{
    ZoomStep, mono_font_size, response_font_size, step_offset, step_zoom, ui_font_size,
};
use apinata_core::MemoryStore;
use gpui_kit::component::Theme;
use gpui_kit::{AppContext, Pixels, TestAppContext, px};
use std::sync::Arc;
use support::*;

fn set(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let s = cx.global::<AppState>().settings.clone();
        s.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

fn theme_sizes(cx: &mut TestAppContext) -> (Pixels, Pixels) {
    cx.update(|cx| {
        let t = Theme::global(cx);
        (t.font_size, t.mono_font_size)
    })
}

#[test]
fn zoom_steps_by_tenths_within_half_and_double() {
    assert_eq!(step_zoom(1.0, ZoomStep::In), 1.1);
    assert_eq!(step_zoom(1.0, ZoomStep::Out), 0.9);
    let mut z = 1.0;
    for _ in 0..3 {
        z = step_zoom(z, ZoomStep::In);
    }
    assert_eq!(z, 1.3, "steps land on tenths, not drifting sums");
    for _ in 0..30 {
        z = step_zoom(z, ZoomStep::In);
    }
    assert_eq!(z, 2.0);
    for _ in 0..30 {
        z = step_zoom(z, ZoomStep::Out);
    }
    assert_eq!(z, 0.5);
    assert_eq!(step_zoom(1.7, ZoomStep::Reset), 1.0);
    assert_eq!(step_zoom(f32::NAN, ZoomStep::In), 1.1);
    assert_eq!(
        step_zoom(9.0, ZoomStep::Out),
        1.9,
        "an out-of-range zoom is clamped first"
    );
}

#[test]
fn the_mono_size_is_the_url_bars_small_text_at_every_zoom() {
    assert_eq!(ui_font_size(1.0), px(13.));
    assert_eq!(mono_font_size(1.0), px(11.375));
    assert_eq!(ui_font_size(2.0), px(26.));
    assert_eq!(mono_font_size(2.0), px(22.75));
    assert_eq!(ui_font_size(0.5), px(6.5));
    assert_eq!(ui_font_size(5.0), px(26.), "the zoom is clamped");
}

#[test]
fn the_response_offset_steps_by_one_pixel_and_keeps_the_size_in_8_to_32() {
    let mono = 11.375;
    assert_eq!(step_offset(mono, 0.0, ZoomStep::In), 1.0);
    assert_eq!(step_offset(mono, 0.0, ZoomStep::Out), -1.0);
    assert_eq!(step_offset(mono, 5.0, ZoomStep::Reset), 0.0);
    assert_eq!(step_offset(mono, 20.0, ZoomStep::In), 20.625, "stops at 32");
    assert_eq!(step_offset(mono, -3.0, ZoomStep::Out), -3.375, "stops at 8");
    assert_eq!(
        step_offset(22.75, 20.0, ZoomStep::Out),
        8.25,
        "after a zoom pushed the size past 32, Out steps down from the shown 32"
    );
    assert_eq!(step_offset(mono, f32::NAN, ZoomStep::In), 1.0);
    assert_eq!(ui_font_size(f32::NAN), px(13.));
    assert_eq!(response_font_size(mono, f32::NAN), px(11.375));
    assert_eq!(response_font_size(mono, 2.0), px(13.375));
    assert_eq!(
        response_font_size(22.75, 20.0),
        px(32.),
        "a later zoom cannot push it past 32"
    );
    assert_eq!(response_font_size(5.6875, -3.0), px(8.));
}

#[test]
fn an_old_settings_file_with_an_editor_font_size_still_loads() {
    let s: Settings = serde_json::from_str(r#"{"theme":"dark","editor_font_size":18.0}"#).unwrap();
    assert_eq!(
        (s.theme, s.zoom, s.response_font_offset),
        (ThemeChoice::Dark, 1.0, 0.0)
    );
}

#[gpui_kit::test]
fn editors_default_to_the_url_bar_size_and_the_zoom_scales_both_sizes(cx: &mut TestAppContext) {
    let col = two_requests();
    let _s = started(cx, &col);
    assert_eq!(theme_sizes(cx), (px(13.), px(11.375)));
    set(cx, |s| s.zoom = 2.0);
    assert_eq!(theme_sizes(cx), (px(26.), px(22.75)));
    // A mode change reloads the theme config, which must not undo the zoom.
    set(cx, |s| s.theme = ThemeChoice::Dark);
    assert_eq!(theme_sizes(cx), (px(26.), px(22.75)));
}

#[gpui_kit::test]
fn start_applies_the_saved_zoom(cx: &mut TestAppContext) {
    support::init(cx);
    let files = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(files.path());
    let saved = Settings {
        zoom: 1.5,
        ..Settings::default()
    };
    save_json(&paths.settings, &saved).unwrap();
    cx.update(|cx| apinata_app::app::start(&paths, Arc::new(MemoryStore::new()), Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(theme_sizes(cx), (px(19.5), px(17.0625)));
}

#[gpui_kit::test]
fn app_text_grows_with_the_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap();
    frame(cx, s.win);
    let heading = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            use gpui_kit::test::TestWindowExt as _;
            w.render_frame(cx);
            w.find("prefs-appearance-heading").bounds().size.height
        })
        .unwrap()
    };
    let before = heading(cx);
    set(cx, |s| s.zoom = 2.0);
    frame(cx, s.win);
    let after = heading(cx);
    assert!(after > before * 1.5, "{before:?} -> {after:?}");
}

#[gpui_kit::test]
fn the_response_size_is_the_mono_size_plus_the_offset(cx: &mut TestAppContext) {
    let col = two_requests();
    let _s = started(cx, &col);
    let size = |cx: &mut TestAppContext| cx.update(|cx| BodyView::font_size(cx));
    assert_eq!(size(cx), px(11.375));
    set(cx, |s| s.response_font_offset = 3.0);
    assert_eq!(size(cx), px(14.375));
    set(cx, |s| s.zoom = 2.0);
    assert_eq!(
        size(cx),
        px(25.75),
        "the zoom scales the response and keeps its offset"
    );
}
