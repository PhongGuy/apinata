//! The rows for stored secrets a pulled rename left under an old name, shared by the team sync
//! dialog, and the Renamed secrets dialog that shows them for a collection outside team mode.

use std::path::{Path, PathBuf};

use apinata_core::{SecretName, SecretRename, TargetState};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Entity, Global, SharedString, Subscription, TestSupportExt, WeakEntity,
    Window, actions, div,
};

use crate::style::SPACE_2;
use crate::team_renames::row_text;
use crate::ui::SolidRing as _;
use crate::ui::dialog_footer;
use crate::workspace::Workspace;

actions!(
    apinata,
    [
        /// Looks for stored secrets the last pull of the current collection left under a
        /// renamed variable.
        CheckSecretRenames
    ]
);

/// The open rows of the collection at `root`, each with its answers, in order.
pub(crate) fn rename_rows(workspace: &Entity<Workspace>, root: &Path, cx: &App) -> Vec<AnyElement> {
    let Some(rows) = workspace.read(cx).secret_renames(root) else {
        return Vec::new();
    };
    (rows.open_rows().into_iter().enumerate())
        .map(|(ix, row)| rename_row(workspace, root, ix, row, rows.error(&row.from), cx))
        .collect()
}

/// One muted line when the last ask failed, so it never reads as nothing to move.
pub(crate) fn failed_line(
    workspace: &Entity<Workspace>,
    root: &Path,
    cx: &App,
) -> Option<AnyElement> {
    let rows = workspace.read(cx).secret_renames(root)?;
    let line = format!("Could not check for secrets to move: {}", rows.failed()?);
    Some(
        div()
            .id("team-rename-failed")
            .test_support()
            .aria_label(line.clone())
            .text_color(cx.theme().muted_foreground)
            .child(line)
            .into_any_element(),
    )
}

/// Move all, offered when two or more open rows are sure and free to move.
pub(crate) fn move_all(workspace: &Entity<Workspace>, root: &Path, cx: &App) -> Option<Button> {
    let rows = workspace.read(cx).secret_renames(root)?;
    let sure = rows.movable_sure();
    (sure.len() >= 2).then(|| {
        let (workspace, root) = (workspace.clone(), root.to_path_buf());
        Button::new("team-rename-move-all")
            .solid_ring(cx)
            .label("Move all")
            .on_click(move |_, _, cx| {
                workspace.update(cx, |w, cx| w.move_secret_renames(&root, sure.clone(), cx));
            })
    })
}

// What a row's button does to the workspace for the row's collection.
type Answer = dyn Fn(&mut Workspace, &Path, &mut gpui_kit::Context<Workspace>);

// One rename row: its text, any note, then its answers. A single free or same target moves
// or is typed again; several targets each get their own Move; a row with nothing it may
// move to is only dismissed, so a different stored value is never overwritten.
fn rename_row(
    workspace: &Entity<Workspace>,
    root: &Path,
    ix: usize,
    row: &SecretRename,
    error: Option<&str>,
    cx: &App,
) -> AnyElement {
    let movable: Vec<(usize, &SecretName)> = (row.to.iter().enumerate())
        .filter(|(_, (_, state))| matches!(state, TargetState::Free | TargetState::Same))
        .map(|(c, (to, _))| (c, to))
        .collect();
    // Each answer runs on the workspace for this collection.
    let on = |act: Box<Answer>| {
        let (workspace, root) = (workspace.clone(), root.to_path_buf());
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            workspace.update(cx, |w, cx| act(w, &root, cx));
        }
    };
    let move_to = |id: gpui_kit::ElementId, label: String, to: &SecretName| {
        let moves = vec![(row.from.clone(), to.clone())];
        Button::new(id)
            .solid_ring(cx)
            .label(label)
            .on_click(on(Box::new(move |w, root, cx| {
                w.move_secret_renames(root, moves.clone(), cx)
            })))
    };
    let mut buttons = Vec::new();
    match (row.to.len(), movable.as_slice()) {
        (_, []) => {
            let from = row.from.clone();
            buttons.push(
                Button::new(("team-rename-dismiss", ix))
                    .solid_ring(cx)
                    .label("Dismiss")
                    .on_click(on(Box::new(move |w, root, cx| {
                        w.dismiss_secret_rename(root, &from, cx)
                    }))),
            );
        }
        (1, [(_, to)]) => {
            buttons.push(move_to(("team-rename-move", ix).into(), "Move".into(), to));
            let (from, to) = (row.from.clone(), (*to).clone());
            buttons.push(
                Button::new(("team-rename-enter", ix))
                    .solid_ring(cx)
                    .label("Enter again")
                    .on_click(on(Box::new(move |w, root, cx| {
                        w.reenter_secret(root, from.clone(), to.clone(), cx)
                    }))),
            );
        }
        (_, candidates) => {
            for &(c, to) in candidates {
                let id = SharedString::from(format!("team-rename-pick-{ix}-{c}"));
                let label = format!("Move to {} / {}", to.environment, to.variable);
                buttons.push(move_to(id.into(), label, to));
            }
        }
    }
    let note = match error {
        Some(error) => Some(format!("Could not move it: {error}")),
        None => movable
            .is_empty()
            .then(|| "a different value is already stored".to_owned()),
    };
    let text = row_text(row, 0);
    h_flex()
        .id(("team-rename", ix))
        .test_support()
        .aria_label(text.clone())
        .gap(SPACE_2)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(text)
                .children(note.map(|note| {
                    div()
                        .id(("team-rename-note", ix))
                        .test_support()
                        .aria_label(note.clone())
                        .text_color(cx.theme().muted_foreground)
                        .child(note)
                })),
        )
        .children(buttons)
        .into_any_element()
}

/// The open Renamed secrets dialog for one collection.
pub struct RenamesView {
    workspace: Entity<Workspace>,
    root: PathBuf,
    _watch: Subscription,
}

// The most recently opened dialog; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<RenamesView>);
impl Global for OpenView {}

/// The open Renamed secrets dialog, for tests and the pilot; None when none is open.
pub fn current(cx: &App) -> Option<Entity<RenamesView>> {
    cx.try_global::<OpenView>().and_then(|v| v.0.upgrade())
}

/// Opens the dialog over `window` for the rename rows of the collection at `root`.
pub fn open(
    workspace: &Entity<Workspace>,
    root: PathBuf,
    window: &mut Window,
    cx: &mut App,
) -> Entity<RenamesView> {
    let workspace = workspace.clone();
    let view = cx.new(|cx| RenamesView {
        _watch: cx.observe(&workspace, |_, _, cx| cx.notify()),
        workspace,
        root,
    });
    cx.set_global(OpenView(view.downgrade()));
    let body = view.clone();
    crate::ui::open_dialog(window, cx, move |dialog, window, _| {
        crate::ui::with_close(
            crate::dialog_frame::place(dialog, window),
            "Renamed secrets",
        )
        .child(body.clone())
    });
    view
}

impl RenamesView {
    /// The text of each open row, in order.
    pub fn rename_lines(&self, cx: &App) -> Vec<String> {
        (self.workspace.read(cx).secret_renames(&self.root))
            .map(|rows| {
                (rows.open_rows().into_iter())
                    .map(|r| row_text(r, 0))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Render for RenamesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = rename_rows(&self.workspace, &self.root, cx);
        let failed = failed_line(&self.workspace, &self.root, cx);
        // Once every row is answered the dialog says so instead of going blank.
        let done = (rows.is_empty() && failed.is_none()).then(|| {
            div()
                .id("rename-done")
                .test_support()
                .aria_label("Nothing left to move")
                .text_color(cx.theme().muted_foreground)
                .child("Nothing left to move")
        });
        let close = Button::new("rename-close")
            .solid_ring(cx)
            .label("Close")
            .on_click(|_, window, cx| window.close_dialog(cx));
        let footer = match move_all(&self.workspace, &self.root, cx) {
            Some(all) => dialog_footer(Some(close), [], all.primary()),
            None => dialog_footer(None, [], close.primary()),
        };
        let cap =
            crate::dialog_frame::titled_body_room(window) * crate::team_sync_dialog::OUTPUT_SHARE;
        v_flex()
            .id("renames-dialog")
            .test_support()
            .gap(SPACE_2)
            .child(
                v_flex()
                    .id("rename-lines")
                    .test_support()
                    .gap(SPACE_2)
                    .max_h(cap)
                    .overflow_y_scroll()
                    .children(rows)
                    .children(failed)
                    .children(done),
            )
            .child(footer)
    }
}
