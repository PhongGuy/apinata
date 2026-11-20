//! Awaiting a platform file dialog: a cancelled dialog is silent, a failed one says so.

use std::fmt::Display;
use std::future::Future;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{App, AsyncWindowContext, Window};

use crate::ui::{self, ToastKind};

/// The toast text for a dialog that could not open while trying to `action`.
pub fn failed_text(action: &str, error: impl Display) -> String {
    format!("Could not open the file dialog to {action}: {error}")
}

/// What a dialog answered: its paths, or `None` when it was cancelled. A dialog that failed,
/// whether the platform reported an error or the reply channel closed, also answers `None`
/// after an error toast in `window` naming `action` (a verb phrase, "open a collection").
pub fn answer<T, F: Display, E: Display>(
    reply: Result<Result<Option<T>, F>, E>,
    action: &str,
    window: &mut Window,
    cx: &mut App,
) -> Option<T> {
    let error = match reply {
        Ok(Ok(paths)) => return paths,
        Ok(Err(e)) => format!("{e:#}"),
        Err(e) => format!("{e:#}"),
    };
    let toast = ui::kind_toast(ToastKind::Error, failed_text(action, error));
    window.push_notification(toast, cx);
    None
}

/// [`answer`] for a dialog reply still to arrive, toasting in the window `cx` belongs to.
pub async fn picked<T, F: Display, E: Display>(
    reply: impl Future<Output = Result<Result<Option<T>, F>, E>>,
    action: &str,
    cx: &mut AsyncWindowContext,
) -> Option<T> {
    let reply = reply.await;
    cx.update(|window, cx| answer(reply, action, window, cx))
        .ok()
        .flatten()
}

/// The first path a single-path dialog answered with, as [`picked`] reports it.
pub async fn picked_one<T: IntoIterator, F: Display, E: Display>(
    reply: impl Future<Output = Result<Result<Option<T>, F>, E>>,
    action: &str,
    cx: &mut AsyncWindowContext,
) -> Option<T::Item> {
    picked(reply, action, cx).await?.into_iter().next()
}
