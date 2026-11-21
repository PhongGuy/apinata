//! The dialog shown when an opened folder is empty: create a collection in it, as New collection
//! would, then open it. Nothing is written without an explicit choice.

use std::path::{Path, PathBuf};

use gpui_kit::{AppContext as _, Context, SharedString, Window};

use crate::confirm::{self, Ask, Choice};
use crate::shell::Shell;

/// Offers to create a collection in the empty folder `root` and opens it once written. Create
/// is the default, so Enter accepts and Escape cancels.
pub fn offer(root: PathBuf, window: &mut Window, cx: &mut Context<Shell>) {
    let ask = Ask {
        title: "Could not open collection".into(),
        detail: Some(offer_detail(&root).into()),
        choices: vec![
            Choice {
                id: "empty-folder-cancel",
                label: "Cancel".into(),
                danger: false,
            },
            Choice {
                id: "empty-folder-create",
                label: "Create a collection here".into(),
                danger: false,
            },
        ],
        default: 1,
        cancel: 0,
    };
    let answer = confirm::ask(ask, window, cx);
    cx.spawn_in(window, async move |shell, cx| {
        if answer.recv().await != Ok(1) {
            return;
        }
        // The New collection path, so the file written is the same and so is any refusal.
        let job = cx.background_spawn(async move { crate::save_draft::create_collection(&root) });
        let result = job.await;
        let _ = shell.update_in(cx, |shell, window, cx| match result {
            Ok(root) => shell.open(root, window, cx),
            Err(message) => {
                let text: SharedString = message.into();
                crate::ui::open_alert_dialog(window, cx, move |dialog, _, _| {
                    dialog
                        .title("Could not create collection")
                        .description(text.clone())
                });
            }
        });
    })
    .detach();
}

/// "{root} is not a collection. The folder is empty, ...": names the folder, so several
/// offers can be told apart.
pub fn offer_detail(root: &Path) -> String {
    format!(
        "{} is not a collection. The folder is empty, so a new collection can be created in it.",
        root.display()
    )
}
