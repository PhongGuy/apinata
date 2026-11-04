//! Copy as curl: plan a request on its collection actor as Send does, resolve it, and put the
//! curl command on the clipboard, asking first when it would carry secret values. The plan,
//! resolve and confirm steps are shared with the Generate Code dialog.

use std::path::Path;

use apinata_core::{
    AuthorizeOptions, CancelHandle, ItemPath, RequestFile, ResolveError, ResolvedRequest, Target,
    VarContext,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AsyncWindowContext, ClipboardItem, Entity, Window};

use crate::command::Command;
use crate::confirm::{self, Ask, Choice};
use crate::send_model::plan_message;
use crate::workspace::Workspace;
use crate::{executor, tokio_bridge, ui};

/// Resolves `item` as Send would (drafts and the selected environment included), asks before
/// copying secret values, then writes the curl command to the clipboard and toasts.
pub fn copy_as_curl(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: ItemPath,
    window: &mut Window,
    cx: &mut App,
) {
    let planned = plan_and_resolve(workspace, root, item, window, cx);
    window
        .spawn(cx, async move |cx| {
            let curl = match planned.recv().await {
                Ok(Ok(r)) => Ok((
                    apinata_core::generate(&r, Target::Curl),
                    !r.sensitive_values().is_empty(),
                )),
                Ok(Err(message)) => Err(message),
                Err(_) => return,
            };
            match curl {
                Ok((text, true)) => {
                    let title = "Copy secrets?";
                    if confirm_secrets(title, "confirm-copy", "Copy with secrets", cx).await {
                        cx.update(|window, cx| copy(text, window, cx)).ok();
                    }
                }
                Ok((text, false)) => drop(cx.update(|window, cx| copy(text, window, cx))),
                Err(message) => drop(cx.update(|window, cx| fail(message, window, cx))),
            }
        })
        .detach();
}

/// Plans `item` on its collection actor and resolves its first request on `window`; the
/// receiver yields the resolved request or a message saying why there is none.
pub(crate) fn plan_and_resolve(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: ItemPath,
    window: &mut Window,
    cx: &mut App,
) -> async_channel::Receiver<Result<ResolvedRequest, String>> {
    let no_token = |e: &ResolveError| matches!(e, ResolveError::OAuth2NoToken { .. });
    plan_and_resolve_with(
        workspace,
        root,
        item,
        apinata_core::resolve,
        no_token,
        window,
        cx,
    )
}

/// `plan_and_resolve`, building the request with `build`; `no_token` says which of its errors
/// means an oauth2 token is missing from memory, so the cached token is loaded and tried again.
pub(crate) fn plan_and_resolve_with<E: std::fmt::Display + 'static>(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: ItemPath,
    build: fn(&RequestFile, &VarContext) -> Result<ResolvedRequest, E>,
    no_token: fn(&E) -> bool,
    window: &mut Window,
    cx: &mut App,
) -> async_channel::Receiver<Result<ResolvedRequest, String>> {
    let (done, out) = async_channel::bounded(1);
    let (reply, answer) = async_channel::bounded(1);
    let sent = workspace.update(cx, |w, cx| w.send(root, Command::Plan { item, reply }, cx));
    if sent.is_none() {
        done.try_send(Err("the collection is not open".into())).ok();
        return out;
    }
    window
        .spawn(cx, async move |cx| {
            let resolved = match answer.recv().await {
                Ok(Ok(planned)) => match planned.plan.requests().next() {
                    Some(file) => resolve(file, &planned.context, build, no_token, cx).await,
                    None => Err("there is no request in this item".into()),
                },
                Ok(Err(e)) => Err(plan_message(&e)),
                Err(_) => Err("the collection was closed".into()),
            };
            done.send(resolved).await.ok();
        })
        .detach();
    out
}

// An oauth2 token missing from memory may sit in the persisted cache, so that is loaded and
// the request built again; this never fetches or refreshes a token.
async fn resolve<E: std::fmt::Display>(
    file: &RequestFile,
    context: &VarContext,
    build: fn(&RequestFile, &VarContext) -> Result<ResolvedRequest, E>,
    no_token: fn(&E) -> bool,
    cx: &mut AsyncWindowContext,
) -> Result<ResolvedRequest, String> {
    match build(file, context) {
        Err(e) if no_token(&e) => {}
        other => return other.map_err(|e| e.to_string()),
    }
    let (owned, ctx) = (file.clone(), context.clone());
    let task = cx
        .update(|_, cx| {
            let exec = executor::shared(cx)?;
            let how = AuthorizeOptions::new().cache_only(true);
            let load = async move {
                let cancel = CancelHandle::new();
                exec.prepare_auth(&owned, &ctx, &how, &cancel).await
            };
            Some(tokio_bridge::spawn(cx, load))
        })
        .ok()
        .flatten()
        .ok_or("the network is not available")?;
    match task.await {
        Ok(Ok(())) => build(file, context).map_err(|e| e.to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("the token lookup stopped".into()),
    }
}

/// Asks before secret values leave the app in plain text; Cancel ("confirm-cancel") is the
/// default. True only when the `go_id` button was chosen.
pub(crate) async fn confirm_secrets(
    title: &str,
    go_id: &'static str,
    go_label: &str,
    cx: &mut AsyncWindowContext,
) -> bool {
    let ask = Ask {
        title: title.to_owned().into(),
        detail: Some("This request contains secret values in plain text.".into()),
        choices: vec![
            Choice {
                id: go_id,
                label: go_label.to_owned().into(),
                danger: true,
            },
            Choice {
                id: "confirm-cancel",
                label: "Cancel".into(),
                danger: false,
            },
        ],
        default: 1,
        cancel: 1,
    };
    let Ok(answer) = cx.update(|window, cx| confirm::ask(ask, window, cx)) else {
        return false;
    };
    answer.recv().await == Ok(0)
}

fn copy(text: String, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    window.push_notification(ui::info_toast("Copied as curl"), cx);
}

fn fail(message: String, window: &mut Window, cx: &mut App) {
    let note = Notification::error(format!("Could not copy as curl: {message}"));
    window.push_notification(note, cx);
}
