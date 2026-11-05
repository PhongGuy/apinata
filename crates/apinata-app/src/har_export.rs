//! "Export as HAR": ask where to save, then write one sent request and its response as a
//! HAR 1.2 file off the UI thread, with the request's secrets already replaced.

use std::future::Future;
use std::time::SystemTime;

use apinata_core::export::{HarExchange, har, sent_exchange};
use apinata_core::{NamedSecret, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AppContext as _, Window};

use crate::export::write_export;
use crate::ui;

/// Shown on a control that cannot export because the shown response has no request attached.
pub const NO_REQUEST: &str =
    "The request is not kept with a saved response. Send it again to export";

/// Once `secrets` resolve, offers `response`, which ended at `ended` (now if unknown), to
/// [`save`] as core writes every sent exchange: as sent, with secrets `[redacted]`. A response
/// core refuses, or secrets that could not be checked, write nothing and say why.
pub fn save_sent(
    response: Response,
    ended: Option<SystemTime>,
    secrets: impl Future<Output = Result<Vec<NamedSecret>, String>> + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let ended = ended.unwrap_or_else(SystemTime::now);
    window
        .spawn(cx, async move |cx| {
            let failure = match secrets.await {
                Ok(secrets) => match sent_exchange(&response, ended, &secrets) {
                    Ok(exchange) => {
                        cx.update(|window, cx| save(exchange, window, cx)).ok();
                        return;
                    }
                    Err(refusal) => refused(&refusal.to_string()),
                },
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

// Core words a refusal as a clause; the toast makes it a sentence.
fn refused(clause: &str) -> String {
    let mut chars = clause.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>());
    format!(
        "{}{}, so nothing was exported",
        first.unwrap_or_default(),
        chars.as_str()
    )
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
