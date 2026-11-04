//! The app zoom and the response body's font offset: step and size math, and the one
//! settings write every zoom key and scroll goes through.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::{App, DispatchPhase, Pixels, Point, ScrollDelta, ScrollWheelEvent, Window, px};

use crate::app::AppState;

/// The theme's UI font size at 100% zoom (`assets/theme/apinata.json` "font.size").
pub const UI_FONT_SIZE: f32 = 13.0;
/// The smallest app zoom.
pub const ZOOM_MIN: f32 = 0.5;
/// The largest app zoom.
pub const ZOOM_MAX: f32 = 2.0;
/// The smallest response body font size in pixels.
pub const RESPONSE_MIN: f32 = 8.0;
/// The largest response body font size in pixels.
pub const RESPONSE_MAX: f32 = 32.0;

// The URL bar's `text_sm` is 0.875 rem, and the code editors match it.
const MONO_RATIO: f32 = 0.875;

/// One zoom key: larger, smaller, or back to the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomStep {
    /// One step larger.
    In,
    /// One step smaller.
    Out,
    /// Back to the default.
    Reset,
}

/// Which level a zoom key acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomTarget {
    /// The whole app's zoom.
    App,
    /// The response body's own font offset.
    Response,
}

// Marks that `install_dialog_zoom` ran, so a second start does not step twice per key.
struct DialogZoomInstalled;
impl gpui_kit::Global for DialogZoomInstalled {}

/// Steps the app zoom on a zoom action no element handled: one bound in the kit's dialog
/// context, since dialogs sit outside the Shell and its handlers. Installs once per app.
pub fn install_dialog_zoom(cx: &mut App) {
    use crate::keymap::{ResetZoom, ZoomIn, ZoomOut};
    if cx.has_global::<DialogZoomInstalled>() {
        return;
    }
    cx.set_global(DialogZoomInstalled);
    let app = ZoomTarget::App;
    cx.on_action(move |_: &ZoomIn, cx| apply_step(app, ZoomStep::In, cx));
    cx.on_action(move |_: &ZoomOut, cx| apply_step(app, ZoomStep::Out, cx));
    cx.on_action(move |_: &ResetZoom, cx| apply_step(app, ZoomStep::Reset, cx));
}

/// Steps `target`'s saved level by `step`; does nothing without an `AppState`. The response
/// offset steps against the zoomed mono size, so its clamp follows the app zoom and not the
/// editor font offset.
pub fn apply_step(target: ZoomTarget, step: ZoomStep, cx: &mut App) {
    let Some(settings) = cx.try_global::<AppState>().map(|s| s.settings.clone()) else {
        return;
    };
    settings.update(cx, |p, cx| {
        p.update(cx, |s| match target {
            ZoomTarget::App => s.zoom = step_zoom(s.zoom, step),
            ZoomTarget::Response => {
                let mono: f32 = mono_font_size(s.zoom).into();
                s.response_font_offset = step_offset(mono, s.response_font_offset, step)
            }
        })
    });
}

/// Pixel scrolls step once per this many pixels.
const SCROLL_PX_PER_STEP: f32 = 50.0;

/// Registers, from a paint closure, a capture-phase listener that turns a ctrl+scroll whose
/// position satisfies `inside` into a `target` step and stops it, so nothing scrolls.
/// `pending` carries the pixel scroll not yet stepped across events.
pub fn paint_scroll_listener(
    target: ZoomTarget,
    pending: Rc<Cell<f32>>,
    inside: impl Fn(Point<Pixels>) -> bool + 'static,
    window: &mut Window,
) {
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture
            || !event.modifiers.secondary()
            || !inside(event.position)
        {
            return;
        }
        if let Some(step) = scroll_step(event.delta, &pending) {
            apply_step(target, step, cx);
        }
        cx.stop_propagation();
    });
}

/// The step one ctrl+scroll makes, wheel up zooming in. A line scroll steps once per event;
/// pixels add up in `pending` and step per 50 px, starting over when the direction flips.
pub fn scroll_step(delta: ScrollDelta, pending: &Cell<f32>) -> Option<ZoomStep> {
    let y = match delta {
        ScrollDelta::Lines(lines) => lines.y,
        ScrollDelta::Pixels(pixels) => {
            let y: f32 = pixels.y.into();
            let sum = if pending.get() * y < 0.0 {
                y
            } else {
                pending.get() + y
            };
            if sum.abs() < SCROLL_PX_PER_STEP {
                pending.set(sum);
                return None;
            }
            pending.set(sum - SCROLL_PX_PER_STEP.copysign(sum));
            sum
        }
    };
    if y > 0.0 {
        Some(ZoomStep::In)
    } else if y < 0.0 {
        Some(ZoomStep::Out)
    } else {
        None
    }
}

// A NaN zoom counts as 100%; anything else is kept within 50% to 200%.
fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_nan() {
        1.0
    } else {
        zoom.clamp(ZOOM_MIN, ZOOM_MAX)
    }
}

/// The zoom after `step`: whole tenths from 0.5 to 2.0, starting from the clamped `zoom`.
pub fn step_zoom(zoom: f32, step: ZoomStep) -> f32 {
    // Counting in integer tenths keeps repeated steps on exact literals like 1.3.
    let tenths = (clamp_zoom(zoom) * 10.0).round() as i32;
    let next = match step {
        ZoomStep::In => tenths + 1,
        ZoomStep::Out => tenths - 1,
        ZoomStep::Reset => return 1.0,
    };
    next.clamp(5, 20) as f32 / 10.0
}

/// The app zoom from the settings; 1.0 where no `AppState` is installed.
pub fn current(cx: &App) -> f32 {
    cx.try_global::<AppState>()
        .map_or(1.0, |s| s.settings.read(cx).get().zoom)
}

/// The UI font size at `zoom`, which is clamped first.
pub fn ui_font_size(zoom: f32) -> Pixels {
    px(UI_FONT_SIZE * clamp_zoom(zoom))
}

/// The code editors' font size at `zoom`: the URL bar's small monospace text.
pub fn mono_font_size(zoom: f32) -> Pixels {
    ui_font_size(zoom) * MONO_RATIO
}

// The offset range that keeps `mono + offset` within the response size limits.
fn clamp_offset(mono: f32, offset: f32) -> f32 {
    let offset = if offset.is_nan() { 0.0 } else { offset };
    offset.clamp(RESPONSE_MIN - mono, RESPONSE_MAX - mono)
}

/// The response offset after `step`: one pixel from the clamped `offset`, clamped again so
/// the shown size stays within 8 to 32 px.
pub fn step_offset(mono: f32, offset: f32, step: ZoomStep) -> f32 {
    let from = clamp_offset(mono, offset);
    match step {
        ZoomStep::In => clamp_offset(mono, from + 1.0),
        ZoomStep::Out => clamp_offset(mono, from - 1.0),
        ZoomStep::Reset => 0.0,
    }
}

/// The response body's font size: `mono + offset`, clamped to 8 to 32 px.
pub fn response_font_size(mono: f32, offset: f32) -> Pixels {
    let offset = if offset.is_nan() { 0.0 } else { offset };
    px((mono + offset).clamp(RESPONSE_MIN, RESPONSE_MAX))
}
