//! The team sync dialog: what stands in the way of a team collection's sync, line by line, with
//! the action that clears it. It keeps no state of its own; it renders the workspace's live
//! status and redraws whenever the workspace notifies.

use std::path::{Path, PathBuf};

use apinata_core::team::{Block, TeamState, TeamSync};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Global, Subscription, TestSupportExt, WeakEntity, Window, div};

use crate::style::{SPACE_2, TEXT_SMALL};
use crate::team_text::problem_lines;
use crate::ui::dialog_footer;
use crate::workspace::Workspace;

/// The open dialog for one team collection.
pub struct SyncView {
    workspace: Entity<Workspace>,
    root: PathBuf,
    _watch: Subscription,
}

// The most recently opened dialog; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<SyncView>);
impl Global for OpenView {}

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
        (self.workspace.read(cx).team_status(&self.root))
            .map(|status| problem_lines(&status))
            .unwrap_or_default()
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
        // read-only folder held back by outside edits.
        let retry = matches!(
            state,
            Some(TeamState::Blocked(
                Block::Secret { .. } | Block::Hook { .. } | Block::Failed { .. }
            ))
        )
        .then(|| {
            Button::new("team-retry")
                .label("Retry")
                .on_click(cx.listener(|this, _, _, cx| this.with_sync(cx, TeamSync::retry)))
        });
        let discard = matches!(state, Some(TeamState::ReadOnly { paused: true })).then(|| {
            Button::new("team-discard")
                .label("Discard")
                .on_click(cx.listener(|this, _, _, cx| this.with_sync(cx, TeamSync::discard)))
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
        // Long output scrolls inside the lines, so the footer with its fix stays in the window.
        let cap = crate::dialog_frame::titled_body_room(window) * 0.6;
        let close = Button::new("team-sync-close")
            .label("Close")
            .on_click(|_, window, cx| window.close_dialog(cx));
        // The fix is the primary button when there is one; otherwise Close is.
        let footer = match retry.or(discard) {
            Some(fix) => dialog_footer(Some(close), [], fix.primary()),
            None => dialog_footer(None, [], close.primary()),
        };
        v_flex()
            .id("team-sync-dialog")
            .test_support()
            .gap(SPACE_2)
            .child(
                v_flex()
                    .id("team-lines")
                    .gap(SPACE_2)
                    .max_h(cap)
                    .overflow_y_scroll()
                    .children(lines),
            )
            .child(footer)
    }
}
