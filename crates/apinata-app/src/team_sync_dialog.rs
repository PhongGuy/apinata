//! The team sync dialog: what stands in the way of a team collection's sync, line by line, with
//! the action that clears it, then one row per clash with its answers, then one row per stored
//! secret a pulled rename left behind. It keeps no state of its own; it renders the workspace's
//! live status and redraws whenever the workspace notifies.

use crate::ui::SolidRing as _;
use std::path::{Path, PathBuf};

use apinata_core::team::{Block, ConflictCopies, TeamState, TeamStore, TeamSync};
use apinata_core::{SecretName, SecretRename, TargetState};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Entity, Global, SharedString, Subscription, TestSupportExt, WeakEntity,
    Window, div,
};

use crate::app::AppState;
use crate::style::{SPACE_2, TEXT_SMALL};
use crate::team_renames::row_text;
use crate::team_text::problem_lines;
use crate::ui::dialog_footer;
use crate::workspace::{Clash, ClashAnswer, ClashKind, Workspace};

/// The share of a titled dialog body's room the lines and clashes may take before they scroll,
/// leaving the rest for the footer.
pub const OUTPUT_SHARE: f32 = 0.6;

/// The open dialog for one team collection.
pub struct SyncView {
    workspace: Entity<Workspace>,
    root: PathBuf,
    _watch: Subscription,
}

// The most recently opened dialog; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<SyncView>);
impl Global for OpenView {}

/// Settles the clash at `rel` of the team collection at `root`, toasting why when it cannot.
pub(crate) fn settle(
    workspace: &Entity<Workspace>,
    root: &Path,
    rel: &str,
    answer: ClashAnswer,
    window: &mut Window,
    cx: &mut App,
) {
    if let Err(e) = workspace.update(cx, |w, cx| w.settle_clash(root, rel, answer, cx)) {
        window.push_notification(
            Notification::error(format!("Could not settle {rel}: {e}")),
            cx,
        );
    }
}

/// The open sync dialog, for tests and the pilot; None when none is open.
pub fn current(cx: &App) -> Option<Entity<SyncView>> {
    cx.try_global::<OpenView>().and_then(|v| v.0.upgrade())
}

/// Opens the dialog over `window` for the team collection at `root`.
pub fn open(
    workspace: &Entity<Workspace>,
    root: PathBuf,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SyncView> {
    let workspace = workspace.clone();
    let view = cx.new(|cx| SyncView {
        _watch: cx.observe(&workspace, |_, _, cx| cx.notify()),
        workspace,
        root,
    });
    cx.set_global(OpenView(view.downgrade()));
    let body = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        crate::dialog_frame::place(dialog, window)
            .title("Team sync")
            .close_button(false)
            .child(body.clone())
    });
    view
}

impl SyncView {
    /// The collection the dialog is about.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The problem lines of the collection's live status; empty once its loop is gone.
    pub fn lines(&self, cx: &App) -> Vec<String> {
        let workspace = self.workspace.read(cx);
        (workspace.team_status(&self.root))
            .map(|status| {
                let clashes = workspace.team_clashes(&self.root).len();
                let renames =
                    (workspace.team_renames(&self.root)).map_or(0, |r| r.open_rows().len());
                problem_lines(&status, clashes, renames)
            })
            .unwrap_or_default()
    }

    /// The text of each open secret rename row, in order.
    pub fn rename_lines(&self, cx: &App) -> Vec<String> {
        (self.workspace.read(cx).team_renames(&self.root))
            .map(|rows| {
                rows.open_rows()
                    .into_iter()
                    .map(|r| row_text(r, 0))
                    .collect()
            })
            .unwrap_or_default()
    }

    // Swaps this dialog for the join dialog, prefilled with the server and user the
    // collection is bound to; nothing when the binding is gone.
    fn sign_in_again(&self, window: &mut Window, cx: &mut App) {
        let Some(file) = cx.try_global::<AppState>().map(|s| s.team.clone()) else {
            return;
        };
        let Some(binding) =
            (TeamStore::load(&file).ok()).and_then(|store| store.binding_at(&self.root).cloned())
        else {
            return;
        };
        window.close_dialog(cx);
        crate::team_join::open_for(
            &self.workspace,
            &binding.server,
            &binding.username,
            window,
            cx,
        );
    }

    // Opens Compare for a clashing copy: the file now against the copy. The file reads as empty
    // when missing or when it is not a regular file reached without a symlink, so a teammate's
    // link to a device or a huge file is never followed.
    fn compare(&self, rel: &str, window: &mut Window, cx: &mut App) {
        let Some(dir) = self.workspace.read(cx).team_copies(&self.root) else {
            return;
        };
        let lossy = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
        let file = self.root.join(rel);
        let regular = crate::workspace::refuse_symlinks(&self.root, rel).is_ok()
            && std::fs::symlink_metadata(&file).is_ok_and(|m| m.is_file());
        let disk = (regular.then(|| std::fs::read(&file).ok()).flatten())
            .map(lossy)
            .unwrap_or_default();
        let mine = match ConflictCopies::new(dir.to_path_buf()).read(rel) {
            Ok(bytes) => lossy(bytes),
            Err(e) => {
                let note = format!("Could not read your saved side of {rel}: {e}");
                return window.push_notification(Notification::error(note), cx);
            }
        };
        let (workspace, root) = (self.workspace.clone(), self.root.clone());
        crate::compare_dialog::open_copy(workspace, root, rel.to_owned(), disk, mine, window, cx);
    }

    // One clash row: its path, then its answers. A read-only folder only keeps theirs, since
    // settling any other way writes.
    fn clash_row(
        &self,
        ix: usize,
        clash: Clash,
        read_only: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let answer = |id: &'static str, label: &'static str, answer: ClashAnswer| {
            let rel = clash.rel.clone();
            Button::new((id, ix))
                .solid_ring(cx)
                .label(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    settle(&this.workspace, &this.root, &rel, answer, window, cx)
                }))
        };
        let writes = match clash.kind {
            ClashKind::Copy => ("team-use-mine", "Use mine", ClashAnswer::UseMine),
            ClashKind::DeletedLocally => (
                "team-delete-again",
                "Delete again",
                ClashAnswer::DeleteAgain,
            ),
        };
        let writing = (!read_only).then(|| answer(writes.0, writes.1, writes.2));
        let keep = answer("team-keep-theirs", "Keep theirs", ClashAnswer::KeepTheirs);
        let compare = (clash.kind == ClashKind::Copy).then(|| {
            let rel = clash.rel.clone();
            Button::new(("team-compare", ix))
                .solid_ring(cx)
                .label("Compare")
                .on_click(cx.listener(move |this, _, window, cx| this.compare(&rel, window, cx)))
        });
        h_flex()
            .id(("team-clash", ix))
            .test_support()
            .aria_label(clash.rel.clone())
            .gap(SPACE_2)
            .child(div().flex_1().child(clash.rel.clone()))
            .children(compare)
            .children(writing)
            .child(keep)
            .into_any_element()
    }

    // One rename row: its text, any note, then its answers. A single free or same target moves
    // or is typed again; several targets each get their own Move; a row with nothing it may
    // move to is only dismissed, so a different stored value is never overwritten.
    fn rename_row(
        &self,
        ix: usize,
        row: &SecretRename,
        error: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let movable: Vec<(usize, &SecretName)> = (row.to.iter().enumerate())
            .filter(|(_, (_, state))| matches!(state, TargetState::Free | TargetState::Same))
            .map(|(c, (to, _))| (c, to))
            .collect();
        let move_to =
            |id: gpui_kit::ElementId, label: String, to: &SecretName, cx: &mut Context<Self>| {
                let moves = vec![(row.from.clone(), to.clone())];
                Button::new(id)
                    .solid_ring(cx)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let (workspace, root) = (this.workspace.clone(), this.root.clone());
                        workspace
                            .update(cx, |w, cx| w.move_secret_renames(&root, moves.clone(), cx));
                    }))
            };
        let mut buttons = Vec::new();
        match (row.to.len(), movable.as_slice()) {
            (_, []) => {
                let from = row.from.clone();
                buttons.push(
                    Button::new(("team-rename-dismiss", ix))
                        .solid_ring(cx)
                        .label("Dismiss")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let (workspace, root) = (this.workspace.clone(), this.root.clone());
                            workspace.update(cx, |w, cx| w.dismiss_secret_rename(&root, &from, cx));
                        })),
                );
            }
            (1, [(_, to)]) => {
                buttons.push(move_to(
                    ("team-rename-move", ix).into(),
                    "Move".into(),
                    to,
                    cx,
                ));
                let (from, to) = (row.from.clone(), (*to).clone());
                buttons.push(
                    Button::new(("team-rename-enter", ix))
                        .solid_ring(cx)
                        .label("Enter again")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let (workspace, root) = (this.workspace.clone(), this.root.clone());
                            let (from, to) = (from.clone(), to.clone());
                            workspace.update(cx, |w, cx| w.reenter_secret(&root, from, to, cx));
                        })),
                );
            }
            (_, candidates) => {
                for &(c, to) in candidates {
                    let id = SharedString::from(format!("team-rename-pick-{ix}-{c}"));
                    let label = format!("Move to {} / {}", to.environment, to.variable);
                    buttons.push(move_to(id.into(), label, to, cx));
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

    // Runs `act` on the collection's sync loop, if it still has one.
    fn with_sync(&self, cx: &App, act: impl FnOnce(&TeamSync)) {
        if let Some(sync) = self.workspace.read(cx).team_sync(&self.root) {
            act(&sync);
        }
    }
}

impl Render for SyncView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = (self.workspace.read(cx).team_status(&self.root)).map(|s| s.state);
        // Retry is the fix for a refusal the user can correct here; Discard unblocks pulls in a
        // read-only folder held back by outside edits; a lapsed session needs a new sign-in.
        let retry = matches!(
            state,
            Some(TeamState::Blocked(
                Block::Refused { .. } | Block::Hook { .. } | Block::Failed { .. }
            ))
        )
        .then(|| {
            Button::new("team-retry")
                .solid_ring(cx)
                .label("Retry")
                .on_click(cx.listener(|this, _, _, cx| this.with_sync(cx, TeamSync::retry)))
        });
        let discard = matches!(state, Some(TeamState::ReadOnly { paused: true })).then(|| {
            Button::new("team-discard")
                .solid_ring(cx)
                .label("Discard")
                .on_click(cx.listener(|this, _, _, cx| this.with_sync(cx, TeamSync::discard)))
        });
        let sign_in = matches!(state, Some(TeamState::Blocked(Block::SignedOut))).then(|| {
            Button::new("team-sign-in-again")
                .solid_ring(cx)
                .label("Sign in again")
                .on_click(cx.listener(|this, _, window, cx| this.sign_in_again(window, cx)))
        });
        // A hook's output or git's message follows its heading line in a mono font.
        let output_at = matches!(
            state,
            Some(TeamState::Blocked(
                Block::Hook { .. } | Block::Failed { .. }
            ))
        )
        .then_some(1);
        let mono = cx.theme().mono_font_family.clone();
        let lines = self.lines(cx).into_iter().enumerate().map(|(ix, line)| {
            div()
                .id(("team-line", ix))
                .test_support()
                .aria_label(line.clone())
                .when(output_at == Some(ix), |el| {
                    el.font_family(mono.clone()).text_size(TEXT_SMALL)
                })
                .child(line)
        });
        let (clashes, read_only) = {
            let w = self.workspace.read(cx);
            (w.team_clashes(&self.root), w.is_team_read_only(&self.root))
        };
        let clashes: Vec<_> = (clashes.into_iter().enumerate())
            .map(|(ix, clash)| self.clash_row(ix, clash, read_only, cx))
            .collect();
        let rows = self.workspace.read(cx).team_renames(&self.root);
        let renames: Vec<_> = (rows.iter())
            .flat_map(|rows| rows.open_rows().into_iter().enumerate())
            .map(|(ix, row)| {
                let error = rows.as_ref().and_then(|r| r.error(&row.from));
                self.rename_row(ix, row, error, cx)
            })
            .collect();
        // A failed ask says so in one muted line, so it never reads as nothing to move.
        let failed = (rows.as_ref().and_then(|r| r.failed())).map(|why| {
            let line = format!("Could not check for secrets to move: {why}");
            div()
                .id("team-rename-failed")
                .test_support()
                .aria_label(line.clone())
                .text_color(cx.theme().muted_foreground)
                .child(line)
        });
        let sure = rows.as_ref().map_or(Vec::new(), |r| r.movable_sure());
        let move_all = (sure.len() >= 2).then(|| {
            Button::new("team-rename-move-all")
                .solid_ring(cx)
                .label("Move all")
                .on_click(cx.listener(move |this, _, _, cx| {
                    let (workspace, root) = (this.workspace.clone(), this.root.clone());
                    workspace.update(cx, |w, cx| w.move_secret_renames(&root, sure.clone(), cx));
                }))
        });
        // Long output and many clashes scroll inside the lines, so the footer with its fix stays
        // in the window.
        let cap = crate::dialog_frame::titled_body_room(window) * OUTPUT_SHARE;
        let close = Button::new("team-sync-close")
            .solid_ring(cx)
            .label("Close")
            .on_click(|_, window, cx| window.close_dialog(cx));
        // The fix is the primary button when there is one, then Move all; otherwise Close is.
        let footer = match (retry.or(discard).or(sign_in), move_all) {
            (Some(fix), move_all) => dialog_footer(Some(close), move_all, fix.primary()),
            (None, Some(move_all)) => dialog_footer(Some(close), [], move_all.primary()),
            (None, None) => dialog_footer(None, [], close.primary()),
        };
        v_flex()
            .id("team-sync-dialog")
            .test_support()
            .gap(SPACE_2)
            .child(
                v_flex()
                    .id("team-lines")
                    .test_support()
                    .gap(SPACE_2)
                    .max_h(cap)
                    .overflow_y_scroll()
                    .children(lines)
                    .children(clashes)
                    .children(renames)
                    .children(failed),
            )
            .child(footer)
    }
}
