//! "Export as HAR": ask where to save, then write one sent request and its response as a
//! HAR 1.2 file off the UI thread, with the request's secrets already replaced.

use std::future::Future;
use std::time::SystemTime;

use apinata_core::export::{HarExchange, har};
use apinata_core::{NamedSecret, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AppContext as _, Window};

use crate::export::write_export;
use crate::ui;

/// Shown on a control that cannot export because the shown response has no request attached.
pub const NO_REQUEST: &str =
    "The request is not kept with a saved response. Send it again to export";

/// The exchange `response` ended at `ended` (now if unknown), or `None` when it has no
/// request attached. Both HAR controls build their file from this, so they agree.
pub fn exchange_for_response(
    response: &Response,
    ended: Option<SystemTime>,
) -> Option<HarExchange> {
    if response.timeline().is_empty() {
        return None;
    }
    let ended = ended.unwrap_or_else(SystemTime::now);
    let started = ended.checked_sub(response.response_time()).unwrap_or(ended);
    Some(HarExchange::from_response(response, started))
}

/// Redacts `exchange` once `secrets` resolve, then offers it to [`save`]. A binary body that
/// holds a secret, or secrets that could not be checked, write nothing and say why.
pub fn save_redacted(
    exchange: HarExchange,
    secrets: impl Future<Output = Result<Vec<NamedSecret>, String>> + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    window
        .spawn(cx, async move |cx| {
            let failure = match secrets.await {
                Ok(secrets) if exchange.binary_body_holds_secret(&secrets) => {
                    "The response body is binary and holds a secret value that cannot be \
                     replaced inside it, so nothing was exported"
                        .to_owned()
                }
                Ok(secrets) => {
                    let exchange = exchange.redacted(&secrets);
                    cx.update(|window, cx| save(exchange, window, cx)).ok();
                    return;
                }
                Err(reason) => format!(
                    "Could not check the request for secrets, so nothing was exported: {reason}"
                ),
            };
            let note = Notification::error(failure);
            cx.update(|window, cx| window.push_notification(note, cx))
                .ok();
        })
        .detach();
}

/// The save dialog's suggested name: the host of the request, else `exchange`, plus `.har`.
pub fn file_name(exchange: &HarExchange) -> String {
    let host = url::Url::parse(&exchange.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "exchange".to_owned());
    format!("{host}.har")
}

/// Prompts for a file, then writes `exchange` there as HAR and toasts the outcome. The caller
/// has already redacted it.
pub fn save(exchange: HarExchange, window: &mut Window, cx: &mut App) {
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let reply = cx.prompt_for_new_path(&dir, Some(file_name(&exchange).as_str()));
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(path))) = reply.await else {
                return;
            };
            let job = cx.background_spawn(async move {
                write_export(&path, har(&exchange).as_bytes())?;
                Ok::<_, String>(path)
            });
            let note = match job.await {
                Ok(path) => ui::info_toast(format!("Exported to {}", path.display())),
                Err(e) => Notification::error(format!("Could not export the HAR: {e}")),
            };
            cx.update(|window, cx| window.push_notification(note, cx))
                .ok();
        })
        .detach();
}
