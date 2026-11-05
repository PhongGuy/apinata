//! Convert to GraphQL and Convert to HTTP: the collection actor rewrites a saved request as the
//! other kind, and a toast reports it with an Undo that puts the file back as it was.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use apinata_core::{ItemPath, KindConversion, RequestKind};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Entity, Window};

use crate::command::Command;
use crate::workspace::Workspace;

/// The id type of a conversion's toast, so a later conversion replaces it.
pub struct ConvertToast;

/// Asks the actor of `root` to convert `item` to `to`, then toasts what happened. Unsaved
/// edits refuse the conversion, as the toast says, so nothing unsaved is ever written.
pub fn convert(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: ItemPath,
    to: RequestKind,
    window: &mut Window,
    cx: &mut App,
) {
    let (reply, answer) = async_channel::bounded(1);
    let command = Command::ConvertKind {
        item: item.clone(),
        kind: to,
        reply,
    };
    let root = root.to_path_buf();
    if workspace
        .update(cx, |w, cx| w.send(&root, command, cx))
        .is_none()
    {
        return;
    }
    let workspace = workspace.clone();
    window
        .spawn(cx, async move |cx| {
            let Ok(result) = answer.recv().await else {
                return;
            };
            cx.update(|window, cx| {
                let toast = match result {
                    Ok(done) => converted(workspace, root, item, done),
                    Err(message) => Notification::error(message),
                };
                window.push_notification(toast, cx);
            })
            .ok();
        })
        .detach();
}

/// The kind a request of `kind` converts to.
pub fn other_kind(kind: RequestKind) -> RequestKind {
    match kind {
        RequestKind::Graphql => RequestKind::Http,
        _ => RequestKind::Graphql,
    }
}

// Names the new kind and anything the conversion dropped; its Undo sends the conversion back.
fn converted(
    workspace: Entity<Workspace>,
    root: PathBuf,
    item: ItemPath,
    done: KindConversion,
) -> Notification {
    let kind = match done.kind() {
        RequestKind::Graphql => "GraphQL",
        _ => "HTTP",
    };
    let message = std::iter::once(format!("Converted to {kind}."))
        .chain(done.dropped().iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let undo = Rc::new(move |window: &mut Window, cx: &mut App| {
        undo(&workspace, &root, item.clone(), done.clone(), window, cx)
    });
    Notification::new()
        .id::<ConvertToast>()
        .message(message)
        .action(move |_, _, cx| {
            let (undo, this) = (undo.clone(), cx.entity());
            Button::new("undo-convert")
                .ghost()
                .label("Undo")
                .on_click(move |_, window, cx| {
                    undo(window, cx);
                    this.update(cx, |n, cx| n.dismiss(window, cx));
                })
        })
        // An action turns the timer off; the Undo is only offered for a few seconds.
        .autohide(true)
}

fn undo(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: ItemPath,
    conversion: KindConversion,
    window: &mut Window,
    cx: &mut App,
) {
    let (reply, answer) = async_channel::bounded(1);
    let command = Command::UndoConvert {
        item,
        conversion,
        reply,
    };
    if workspace
        .update(cx, |w, cx| w.send(root, command, cx))
        .is_none()
    {
        return;
    }
    window
        .spawn(cx, async move |cx| {
            if let Ok(Err(message)) = answer.recv().await {
                cx.update(|window, cx| window.push_notification(Notification::error(message), cx))
                    .ok();
            }
        })
        .detach();
}
