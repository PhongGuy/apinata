//! The identity the desktop shell uses to match the window to its installed desktop entry.

use image::{ImageFormat, RgbaImage};
use std::sync::{Arc, OnceLock};

/// The window's app id: the stem of the desktop file the Linux packages install, and the
/// X11 window class.
pub const APP_ID: &str = "apinata-app";

/// The 256 px app icon for X11's `_NET_WM_ICON`, decoded from the committed PNG on first
/// use and shared after; `None` if it fails to decode (the window then has no icon).
pub fn window_icon() -> Option<Arc<RgbaImage>> {
    static ICON: OnceLock<Option<Arc<RgbaImage>>> = OnceLock::new();
    let png = include_bytes!("../../../assets/icon/generated/png/256x256.png");
    ICON.get_or_init(|| {
        image::load_from_memory_with_format(png, ImageFormat::Png)
            .ok()
            .map(|img| Arc::new(img.to_rgba8()))
    })
    .clone()
}
