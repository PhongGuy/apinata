//! Large bodies show a fixed-width line-number gutter.

use crate::support;

use apinata_app::body_view::{BodyView, Shown, gutter_digits};
use gpui_kit::component::Theme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, WindowOptions};
use support::*;
use wiremock::ResponseTemplate;

#[test]
fn digits_grow_with_the_line_count() {
    for (lines, digits) in [
        (0, 1),
        (1, 1),
        (9, 1),
        (10, 2),
        (99, 2),
        (100, 3),
        (30_000, 5),
    ] {
        assert_eq!(gutter_digits(lines), digits, "{lines}");
    }
}

#[gpui_kit::test]
fn every_row_has_a_line_number_of_one_width(cx: &mut TestAppContext) {
    let body = vec!["line"; 30_000].join("\n");
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string(body), "");
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(view.read_with(cx, |v, _| v.shown()), Shown::Lines(30_000));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let first = w.find(("line-number", 0usize)).bounds();
        let second = w.find(("line-number", 1usize)).bounds();
        assert_eq!(first.size.width, second.size.width);
        assert!(first.size.width > gpui_kit::px(0.));
        assert!(second.top() >= first.bottom(), "one number per row");
    })
    .unwrap();
}

#[gpui_kit::test]
fn rows_follow_the_mono_font_size(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200).set_body_string(vec!["line"; 30_000].join("\n")),
        "",
    );
    support::init(cx);
    // The gutter is sized from the mono font size, so the digits must draw at it too.
    let mut heights = Vec::new();
    for size in [10., 30.] {
        cx.update(|cx| Theme::update(cx, |t| t.mono_font_size = gpui_kit::px(size)));
        let (win, view) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                    cx.new(|cx| BodyView::new(w, cx))
                })
            })
            .unwrap();
        let r = s.result.clone();
        cx.update_window(win, |_, w, cx| {
            view.update(cx, |v, cx| v.show(Some(r), w, cx))
        })
        .unwrap();
        frame(cx, win);
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            heights.push(w.find(("line-number", 0usize)).bounds().size.height);
        })
        .unwrap();
    }
    assert!(heights[1] > heights[0], "{heights:?}");
}

// A text system whose digits are 0.8 em wide, where the stock test one gives 0.6 em.
struct WideDigits(gpui_kit::NoopTextSystem);

impl gpui_kit::PlatformTextSystem for WideDigits {
    fn add_fonts(&self, f: Vec<std::borrow::Cow<'static, [u8]>>) -> gpui_kit::Result<()> {
        self.0.add_fonts(f)
    }
    fn all_font_names(&self) -> Vec<String> {
        self.0.all_font_names()
    }
    fn font_id(&self, d: &gpui_kit::Font) -> gpui_kit::Result<gpui_kit::FontId> {
        self.0.font_id(d)
    }
    fn font_metrics(&self, id: gpui_kit::FontId) -> gpui_kit::FontMetrics {
        self.0.font_metrics(id)
    }
    fn typographic_bounds(
        &self,
        id: gpui_kit::FontId,
        g: gpui_kit::GlyphId,
    ) -> gpui_kit::Result<gpui_kit::Bounds<f32>> {
        self.0.typographic_bounds(id, g)
    }
    fn advance(
        &self,
        _id: gpui_kit::FontId,
        g: gpui_kit::GlyphId,
    ) -> gpui_kit::Result<gpui_kit::Size<f32>> {
        Ok(gpui_kit::size(800.0 * g.0 as f32, 0.0))
    }
    fn glyph_for_char(&self, id: gpui_kit::FontId, ch: char) -> Option<gpui_kit::GlyphId> {
        self.0.glyph_for_char(id, ch)
    }
    fn glyph_raster_bounds(
        &self,
        p: &gpui_kit::RenderGlyphParams,
    ) -> gpui_kit::Result<gpui_kit::Bounds<gpui_kit::DevicePixels>> {
        self.0.glyph_raster_bounds(p)
    }
    fn rasterize_glyph(
        &self,
        p: &gpui_kit::RenderGlyphParams,
        b: gpui_kit::Bounds<gpui_kit::DevicePixels>,
    ) -> gpui_kit::Result<(gpui_kit::Size<gpui_kit::DevicePixels>, Vec<u8>)> {
        self.0.rasterize_glyph(p, b)
    }
    fn layout_line(
        &self,
        t: &str,
        s: gpui_kit::Pixels,
        r: &[gpui_kit::FontRun],
    ) -> gpui_kit::LineLayout {
        self.0.layout_line(t, s, r)
    }
    fn recommended_rendering_mode(
        &self,
        id: gpui_kit::FontId,
        s: gpui_kit::Pixels,
    ) -> gpui_kit::TextRenderingMode {
        self.0.recommended_rendering_mode(id, s)
    }
}

#[test]
fn the_gutter_follows_the_fonts_digit_advance_not_an_estimate() {
    use apinata_app::body_view::digit_gutter;
    use gpui_kit::{TestDispatcher, px};
    let mut cx = TestAppContext::build_with_text_system(
        TestDispatcher::new(0),
        None,
        std::sync::Arc::new(WideDigits(gpui_kit::NoopTextSystem)),
    );
    let s = send_once(
        &mut cx,
        ResponseTemplate::new(200).set_body_string(vec!["line"; 30_000].join("\n")),
        "",
    );
    support::init(&mut cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| BodyView::new(w, cx))
            })
        })
        .unwrap();
    let r = s.result.clone();
    cx.update_window(win, |_, w, cx| {
        view.update(cx, |v, cx| v.show(Some(r), w, cx))
    })
    .unwrap();
    frame(&mut cx, win);
    cx.update_window(win, |_, w, cx| {
        use gpui_kit::component::ActiveTheme as _;
        let size = cx.theme().mono_font_size;
        let pad = apinata_app::style::resolve(apinata_app::style::SPACE_2, w);
        w.render_frame(cx);
        let got = w.find(("line-number", 0usize)).bounds().size.width;
        let want = digit_gutter(size * 0.8, 5, pad);
        assert!((got - want).abs() <= px(0.5), "{got:?} against {want:?}");
    })
    .unwrap();
}
