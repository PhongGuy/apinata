//! The dialog shown when an opened folder is a legacy .bru collection: convert it into a new
//! folder or in place, then open the result. Nothing is written without an explicit choice.

use std::collections::VecDeque;
use std::fmt::Display;
use std::path::{Path, PathBuf};

use apinata_core::legacy::{ConversionPlan, ConvertError, ConvertTarget, convert, plan_conversion};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AppContext as _, AsyncWindowContext, Context, Entity, PathPromptOptions, SharedString, Window,
};

use crate::confirm::{self, Ask, Choice};
use crate::shell::{Shell, first_path};
use crate::workspace::Workspace;

/// How many failing files or paths an alert lists before "and N more".
const LISTED: usize = 10;

/// Legacy roots waiting for their offer, and whether one is being shown.
#[derive(Default)]
pub(crate) struct Offers {
    waiting: VecDeque<PathBuf>,
    /// The root whose offer is on screen, if any.
    showing: Option<PathBuf>,
}

/// Offers to convert the legacy collection at `root` and opens the result in the shell's
/// workspace. Offers are shown one at a time, so a session restoring several legacy roots
/// asks about each in turn once the previous one is resolved. A root already waiting or on
/// screen is not queued again, since after converting it the repeat would find nothing legacy.
pub fn offer(shell: &mut Shell, root: PathBuf, window: &mut Window, cx: &mut Context<Shell>) {
    let root = identity(&root);
    let offers = &mut shell.legacy_offers;
    if offers.showing.as_ref() == Some(&root) || offers.waiting.contains(&root) {
        return;
    }
    offers.waiting.push_back(root);
    show_next(shell, window, cx);
}

/// The directory's identity for dedupe: canonical when it resolves, else the absolute path
/// with `.` and trailing separators dropped, so a failure never drops an offer.
fn identity(root: &Path) -> PathBuf {
    if let Ok(real) = root.canonicalize() {
        return real;
    }
    let abs = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    abs.components().collect()
}

fn show_next(shell: &mut Shell, window: &mut Window, cx: &mut Context<Shell>) {
    if shell.legacy_offers.showing.is_some() {
        return;
    }
    let Some(root) = shell.legacy_offers.waiting.pop_front() else {
        return;
    };
    shell.legacy_offers.showing = Some(root.clone());
    show(shell.workspace().clone(), root, window, cx);
}

// Runs one offer to its end (cancelled, converted, or its failure alert closed), then
// shows the next waiting one.
fn show(workspace: Entity<Workspace>, root: PathBuf, window: &mut Window, cx: &mut Context<Shell>) {
    let ask = Ask {
        title: "Convert legacy collection?".into(),
        detail: Some(offer_detail(&root).into()),
        choices: vec![
            choice("legacy-cancel", "Cancel", false),
            choice("legacy-in-place", "Convert in place...", false),
            choice("legacy-new-folder", "Convert to a new folder...", false),
        ],
        default: 2,
        cancel: 0,
    };
    let answer = confirm::ask(ask, window, cx);
    cx.spawn_in(window, async move |shell, cx| {
        let result = match answer.recv().await {
            Ok(1) => in_place(root, cx).await,
            Ok(2) => new_folder(root, cx).await,
            _ => None,
        };
        match result {
            Some(Ok(new_root)) => workspace.update(cx, |w, cx| w.open(new_root, cx)),
            Some(Err(text)) => alert(text, cx).await,
            None => {}
        }
        let _ = shell.update_in(cx, |shell, window, cx| {
            shell.legacy_offers.showing = None;
            show_next(shell, window, cx);
        });
    })
    .detach();
}

// Shows the failure and waits until the alert is closed, by any button or by being dropped.
async fn alert(text: String, cx: &mut AsyncWindowContext) {
    let (closed, wait) = async_channel::bounded::<()>(1);
    let shown = cx.update(|window, cx| {
        let text: SharedString = text.into();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let closed = closed.clone();
            dialog
                .title("Could not convert collection")
                .description(text.clone())
                .on_close(move |_, _, _| {
                    let _ = closed.try_send(());
                })
        });
    });
    if shown.is_ok() {
        let _ = wait.recv().await;
    }
}

// Asks for the parent directory, then plans and converts off the main thread; the original is
// left untouched. `None` when the prompt was cancelled.
async fn new_folder(root: PathBuf, cx: &mut AsyncWindowContext) -> Option<Result<PathBuf, String>> {
    let reply = cx
        .update(|_, cx| {
            cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Convert into".into()),
            })
        })
        .ok()?;
    let parent = first_path(reply.await)?;
    let job = cx.background_spawn(async move {
        let plan = plan_conversion(&root)?;
        convert(&plan, ConvertTarget::NewFolderIn(parent))
    });
    Some(job.await.map_err(|e| failure_text(&e)))
}

// Plans first, so a collection that cannot convert fails before the second confirmation, which
// names what is created and deleted. `None` when that confirmation is declined.
async fn in_place(root: PathBuf, cx: &mut AsyncWindowContext) -> Option<Result<PathBuf, String>> {
    let plan = match cx
        .background_spawn(async move { plan_conversion(&root) })
        .await
    {
        Ok(plan) => plan,
        Err(e) => return Some(Err(failure_text(&e))),
    };
    let ask = Ask {
        title: "Convert in place?".into(),
        detail: Some(
            in_place_detail(plan.created().len(), plan.removed_in_place().len() - 1).into(),
        ),
        choices: vec![
            choice("legacy-cancel", "Cancel", false),
            choice("legacy-convert", "Convert", true),
        ],
        default: 0,
        cancel: 0,
    };
    let answer = cx.update(|window, cx| confirm::ask(ask, window, cx)).ok()?;
    if answer.recv().await != Ok(1) {
        return None;
    }
    let job = cx.background_spawn(async move {
        convert(&plan, ConvertTarget::InPlace).map_err(|e| conversion_failure(&plan, &e))
    });
    Some(job.await)
}

fn choice(id: &'static str, label: &'static str, danger: bool) -> Choice {
    Choice {
        id,
        label: label.into(),
        danger,
    }
}

/// "{root} uses the legacy .bru format. ...": names the folder, so queued offers can be told apart.
pub fn offer_detail(root: &Path) -> String {
    format!(
        "{} uses the legacy .bru format. Apinata stores collections as YAML files.",
        root.display()
    )
}

/// "{c} YAML file(s) will be created. {r} .bru file(s) and the legacy collection file will be deleted. Other files are left untouched."
pub fn in_place_detail(created: usize, removed_bru: usize) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    format!(
        "{created} YAML file{} will be created. {removed_bru} .bru file{} and the legacy \
         collection file will be deleted. Other files are left untouched.",
        plural(created),
        plural(removed_bru)
    )
}

/// `error`'s Display, then one line per BruError or existing path (the first ten), then "and N more".
pub fn failure_text(error: &ConvertError) -> String {
    let mut text = error.to_string();
    match error {
        ConvertError::Parse { errors } => push_listed(&mut text, errors),
        ConvertError::TargetExists { paths } => {
            push_listed(
                &mut text,
                &paths.iter().map(|p| p.display()).collect::<Vec<_>>(),
            );
        }
        _ => {}
    }
    text
}

/// [`failure_text`] for an in-place conversion of `plan`; a failed delete adds that the YAML
/// collection is complete and lists the legacy files left to remove by hand.
pub fn conversion_failure(plan: &ConversionPlan, error: &ConvertError) -> String {
    let mut text = failure_text(error);
    if let Some(left) = plan.left_over(error) {
        text.push_str(
            "\nThe YAML collection is complete, but some legacy files were not deleted.\n\
             Remove these leftover legacy files by hand; converting again stops at the YAML files:",
        );
        push_listed(
            &mut text,
            &left.iter().map(|p| p.display()).collect::<Vec<_>>(),
        );
    }
    text
}

// One line per item up to the cap, then a count of the rest.
fn push_listed(text: &mut String, items: &[impl Display]) {
    for item in items.iter().take(LISTED) {
        text.push_str(&format!("\n{item}"));
    }
    if items.len() > LISTED {
        text.push_str(&format!("\nand {} more", items.len() - LISTED));
    }
}
