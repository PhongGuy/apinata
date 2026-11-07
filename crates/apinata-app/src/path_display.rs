//! Shows a folder path in little room: home as "~", the middle elided, the tail kept.

use crate::style;
use gpui_kit::{Pixels, Window};
use std::path::Path;
use std::sync::Arc;

/// `path` with the `home` prefix replaced by "~"; other paths are shown as they are.
pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    if let Some(rest) = home.and_then(|h| path.strip_prefix(h).ok()) {
        let rest = rest.display().to_string();
        return if rest.is_empty() {
            "~".to_owned()
        } else {
            format!("~/{rest}")
        };
    }
    path.display().to_string()
}

/// Whether `shown` leaves part of `path` out, so the full path belongs in a tooltip.
pub fn is_shortened(path: &Path, home: Option<&Path>, shown: &str) -> bool {
    shown != tilde(path, home)
}

/// `path` as it should read within `max` width units, as `measure` counts them.
///
/// A path that fits is whole. Otherwise leading components give way first, in the middle,
/// so the folder and its parent survive; only a last component wider than `max` is cut,
/// at its end.
pub fn display(
    path: &Path,
    home: Option<&Path>,
    max: f32,
    measure: &dyn Fn(&str) -> f32,
) -> String {
    let text = tilde(path, home);
    if measure(&text) <= max {
        return text;
    }
    let parts: Vec<&str> = text.split('/').collect();
    let n = parts.len();
    // Prefer the candidate that keeps the most components, then the longest tail; the tail
    // is at least the last component, and two when they fit.
    let mut best: Option<(usize, usize, String)> = None;
    for tail in 1..n {
        for head in 0..=(n - tail - 1).min(n) {
            if head + tail >= n {
                continue;
            }
            let mut pieces: Vec<&str> = parts[..head].to_vec();
            pieces.push("…");
            pieces.extend_from_slice(&parts[n - tail..]);
            let shown = pieces.join("/");
            if measure(&shown) > max {
                continue;
            }
            // Two kept tail components outrank any head.
            let rank = (usize::from(tail >= 2), head + tail, tail);
            if best
                .as_ref()
                .is_none_or(|(a, b, _)| (rank.0, rank.1) > (*a, *b))
            {
                best = Some((rank.0, rank.1, shown));
            }
        }
    }
    if let Some((_, _, shown)) = best {
        return shown;
    }
    let last = parts[n - 1];
    if measure(last) <= max {
        return last.to_owned();
    }
    cut_end(last, max, measure)
}

// The longest prefix of `text` that, with a closing "…", fits `max`.
fn cut_end(text: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> String {
    let chars: Vec<char> = text.chars().collect();
    for keep in (0..chars.len()).rev() {
        let shown = format!("{}…", chars[..keep].iter().collect::<String>());
        if measure(&shown) <= max {
            return shown;
        }
    }
    "…".to_owned()
}

/// A width function for text at `size` in the window's current font, in pixels; a glyph the
/// text system cannot measure counts as half an em.
pub fn measurer(window: &Window, size: Pixels) -> impl Fn(&str) -> f32 + use<> {
    let system = Arc::clone(window.text_system());
    let font = system.resolve_font(&window.text_style().font());
    move |text: &str| {
        text.chars()
            .map(|c| {
                system
                    .advance(font, size, c)
                    .map_or(f32::from(size) * 0.5, |a| f32::from(a.width))
            })
            .sum()
    }
}

/// `path` fitted to `room` pixels at the small text size.
pub fn fitted(path: &Path, home: Option<&Path>, room: Pixels, window: &Window) -> String {
    let size = style::resolve(style::TEXT_SMALL, window);
    display(path, home, f32::from(room), &measurer(window, size))
}
