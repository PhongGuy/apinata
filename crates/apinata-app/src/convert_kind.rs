//! Convert to GraphQL and Convert to HTTP: the collection actor rewrites a saved request as the
//! other kind, and a toast reports it with an Undo that puts the file back as it was.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use apinata_core::{ConversionPreview, ItemPath, KindConversion, RequestKind};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Entity, Window};

use crate::command::Command;
use crate::confirm::{Ask, Choice};
use crate::workspace::Workspace;

/// The id type of a conversion's toast, so a later conversion replaces it.
pub struct ConvertToast;

/// Converts `item` of `root` to `to` and toasts what happened. A conversion that would drop
/// the body or saved examples asks first, naming them; unsaved edits refuse it with a toast.
pub fn convert(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: ItemPath,
    to: RequestKind,
    window: &mut Window,
    cx: &mut App,
) {
    let (reply, answer) = async_channel::bounded(1);
    let command = Command::ConvertPreview {
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
            let preview = match answer.recv().await {
                Ok(Ok(preview)) => preview,
                Ok(Err(message)) => {
                    cx.update(|window, cx| {
                        window.push_notification(Notification::error(message), cx)
                    })
                    .ok();
                    return;
                }
                Err(_) => return,
            };
            if !preview.dropped().is_empty() {
                let Ok(answer) =
                    cx.update(|window, cx| crate::confirm::ask(loss_ask(to, &preview), window, cx))
                else {
                    return;
                };
                if answer.recv().await != Ok(1) {
                    return;
                }
            }
            cx.update(|window, cx| run(&workspace, root, item, to, window, cx))
                .ok();
        })
        .detach();
}

// The prompt before a lossy conversion: what goes, and anything else worth knowing.
fn loss_ask(to: RequestKind, preview: &ConversionPreview) -> Ask {
    let detail = preview
        .dropped()
        .iter()
        .chain(preview.notes())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    Ask {
        title: format!("Convert to {}?", kind_name(to)).into(),
        detail: Some(detail.into()),
        choices: vec![
            Choice {
                id: "confirm-cancel",
                label: "Cancel".into(),
                danger: false,
            },
            Choice {
                id: "confirm-convert",
                label: "Convert".into(),
                danger: true,
            },
        ],
        default: 1,
        cancel: 0,
    }
}

// Sends the conversion itself and toasts its result.
fn run(
    workspace: &Entity<Workspace>,
    root: PathBuf,
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

fn kind_name(kind: RequestKind) -> &'static str {
    match kind {
        RequestKind::Graphql => "GraphQL",
        _ => "HTTP",
    }
}

/// The kind a request of `kind` converts to.
pub fn other_kind(kind: RequestKind) -> RequestKind {
    match kind {
        RequestKind::Graphql => RequestKind::Http,
        _ => RequestKind::Graphql,
    }
}

// Names the new kind, anything the conversion dropped and anything else it notes; its Undo sends the conversion back.
fn converted(
    workspace: Entity<Workspace>,
    root: PathBuf,
    item: ItemPath,
    done: KindConversion,
) -> Notification {
    let message = std::iter::once(format!("Converted to {}.", kind_name(done.kind())))
        .chain(done.dropped().iter().cloned())
        .chain(done.notes().iter().cloned())
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
