//! The branch dialog: lists a collection's branches, fetches and switches to one, and pulls or
//! pushes the current one.
//! The model is `branch_picker`; this is the view over it.

use crate::ui::LockButton as _;
use crate::ui::SolidRing as _;
use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Entity, Global, Subscription, TestSupportExt, WeakEntity, Window, div,
};

use crate::branch_picker::{Activity, BranchPicker, Notice, PickerEvent};
use crate::changes_dialog;
use crate::style::{SPACE_1, SPACE_2};
use crate::ui::{self, RingRoom as _, RingSides};
use crate::workspace::Workspace;

const EMPTY: &str = "No branches yet. Fetch to look for them.";

/// Why Fetch, Pull and Push are locked in a repository without a remote.
pub const NO_REMOTE: &str = "No remote configured";

// The dialog's body: it owns the model, so closing the dialog drops the model and cancels a fetch.
struct BranchView {
    picker: Entity<BranchPicker>,
    workspace: Entity<Workspace>,
    _subscription: Subscription,
}

// The most recently opened dialog's model; it lapses once the dialog lets go of it.
struct OpenPicker(WeakEntity<BranchPicker>);
impl Global for OpenPicker {}

/// The open dialog's model, for tests and the pilot; None when no dialog is open.
pub fn current(cx: &App) -> Option<Entity<BranchPicker>> {
    cx.try_global::<OpenPicker>().and_then(|p| p.0.upgrade())
}

/// Opens the branch dialog of the repository at `root` over `window`.
pub fn open(
    workspace: &Entity<Workspace>,
    root: PathBuf,
    window: &mut Window,
    cx: &mut App,
) -> Entity<BranchPicker> {
    let picker = cx.new(|cx| BranchPicker::new(workspace.clone(), root, cx));
    cx.set_global(OpenPicker(picker.downgrade()));
    let view = cx.new(|cx| BranchView {
        picker: picker.clone(),
        workspace: workspace.clone(),
        // A switch that landed, or closed the collection, ends the dialog's job.
        _subscription: cx.subscribe_in(&picker, window, |_, _, _: &PickerEvent, window, cx| {
            window.close_dialog(cx)
        }),
    });
    let guard = picker.clone();
    crate::ui::open_dialog(window, cx, move |dialog, window, _| {
        let guard = guard.clone();
        crate::dialog_frame::place(dialog, window)
            .title("Switch branch")
            .close_button(false)
            // Escape waits out a running switch or pull: closing then would lose its result.
            .on_cancel(move |_, _, cx| {
                !matches!(
                    guard.read(cx).activity(),
                    Activity::Switching | Activity::Pulling
                )
            })
            // Enter acts on the focused row or button only, never confirms the list away.
            .on_ok(|_, _, _| false)
            .child(view.clone())
    });
    picker
}

impl BranchView {
    fn open_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.picker.read(cx).root().to_owned();
        window.close_dialog(cx);
        changes_dialog::open_changes(&self.workspace, &root, None, window, cx);
    }
}

impl Render for BranchView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let picker = self.picker.read(cx);
        let activity = picker.activity();
        // While anything runs the dialog is read-only: rows and Fetch are off, and so is Close
        // during a switch (its result must land first).
        let idle = activity == Activity::Idle;
        let muted = cx.theme().muted_foreground;
        let rows: Vec<AnyElement> = picker
            .branches()
            .iter()
            .enumerate()
            .map(|(ix, b)| {
                // A local row shows its name, and its upstream only when that is named
                // differently; a remote-only row shows `remote/name`.
                let remote = b.upstream.as_ref();
                let shown = |u: &apinata_core::RemoteBranch| format!("{}/{}", u.remote, u.name);
                let name = b.local.clone().or_else(|| remote.map(shown));
                let upstream = remote
                    .filter(|u| b.local.as_deref().is_some_and(|l| l != u.name))
                    .map(shown);
                let label = name.unwrap_or_default();
                let branch = b.clone();
                // A left-aligned row like the environment selector's: the name, its upstream
                // when it differs, and a check at the far end on the checked-out branch.
                let check = b.head.then(|| {
                    div()
                        .id("branch-head-check")
                        .test_support()
                        .child(Icon::new(IconName::Check).small())
                });
                let content = h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(SPACE_2)
                    .justify_between()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap(SPACE_1)
                            .child(
                                div()
                                    .id(("branch-name", ix))
                                    .test_support()
                                    .min_w_0()
                                    .truncate()
                                    .child(label.clone()),
                            )
                            .children(upstream.map(|u| {
                                div()
                                    .id(("branch-upstream", ix))
                                    .test_support()
                                    .aria_label(u.clone())
                                    .text_color(muted)
                                    .child(u)
                            })),
                    )
                    .children(check);
                Button::new(("branch-row", ix))
                    .solid_ring(cx)
                    .ghost()
                    .w_full()
                    .accessibility_label(label)
                    .child(content)
                    .lock(!idle || b.head, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let branch = branch.clone();
                        this.picker.update(cx, |p, cx| p.switch(branch, cx));
                    }))
                    .into_any_element()
            })
            .collect();
        let list = if rows.is_empty() && activity != Activity::Listing {
            div()
                .text_color(cx.theme().muted_foreground)
                .child(EMPTY)
                .into_any_element()
        } else {
            // The rows are buttons only, so a focused row takes Enter as its press. The kit
            // body clips flush with the list, so the padding keeps a focused row's ring whole.
            v_flex()
                .key_context(ui::DIALOG_FOOTER_CONTEXT)
                .ring_room(RingSides::ALL)
                .gap(SPACE_2)
                .children(rows)
                .into_any_element()
        };
        let notice = picker.notice().cloned().map(|n| {
            let changes = (n == Notice::Dirty).then(|| {
                Button::new("branch-open-changes")
                    .solid_ring(cx)
                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                    .label("Open changes")
                    .on_click(cx.listener(|this, _, window, cx| this.open_changes(window, cx)))
            });
            let publish = match &n {
                Notice::Publish { remote, .. } => Some(
                    Button::new("branch-publish")
                        .solid_ring(cx)
                        .key_context(ui::DIALOG_FOOTER_CONTEXT)
                        .label(format!("Publish to {remote}"))
                        .lock(!idle, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.picker.update(cx, |p, cx| p.publish(cx));
                        })),
                ),
                _ => None,
            };
            v_flex()
                .gap(SPACE_2)
                .child(
                    div()
                        .id("branch-notice")
                        .test_support()
                        .text_color(cx.theme().danger)
                        .child(n.text().to_owned()),
                )
                .children(changes)
                .children(publish)
        });
        // Without a remote there is nothing to sync with, and the buttons say so up front.
        let no_remote = picker.lacks_remote();
        let fetching = activity == Activity::Fetching;
        let fetch = Button::new("branch-fetch")
            .solid_ring(cx)
            .label(if fetching { "Fetching…" } else { "Fetch" })
            .lock(!idle || no_remote, cx)
            .when(no_remote, |b| b.tooltip(NO_REMOTE));
        let fetch = fetch.on_click(cx.listener(|this, _, _, cx| {
            this.picker.update(cx, |p, cx| p.fetch(cx));
        }));
        // A team collection syncs on its own, so it offers no pull or push.
        let root = picker.root().to_owned();
        let syncs = self.workspace.read(cx).team_status(&root).is_none();
        let sync_button = |id: &'static str, label: &'static str, busy: &'static str, on: bool| {
            Button::new(id)
                .solid_ring(cx)
                .label(if on { busy } else { label })
                .lock(!(idle || activity == Activity::Listing) || no_remote, cx)
                .when(no_remote, |b| b.tooltip(NO_REMOTE))
        };
        let pull = syncs.then(|| {
            let on = activity == Activity::Pulling;
            sync_button("branch-pull", "Pull", "Pulling…", on).on_click(cx.listener(
                |this, _, _, cx| {
                    this.picker.update(cx, |p, cx| p.pull(cx));
                },
            ))
        });
        let push = syncs.then(|| {
            let on = activity == Activity::Pushing;
            sync_button("branch-push", "Push", "Pushing…", on).on_click(cx.listener(
                |this, _, _, cx| {
                    this.picker.update(cx, |p, cx| p.push(cx));
                },
            ))
        });
        // Stop shows while git talks to the remote; a pull's fast-forward cannot stop, so it
        // stays visible but off once the fetch is over.
        let running = matches!(
            activity,
            Activity::Fetching | Activity::Pulling | Activity::Pushing
        );
        let stop = running.then(|| {
            Button::new("branch-fetch-cancel")
                .solid_ring(cx)
                .label("Stop")
                .lock(!picker.can_cancel(), cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.picker.update(cx, |p, _| p.cancel_fetch());
                }))
        });
        let close = Button::new("branch-close")
            .solid_ring(cx)
            .label("Close")
            .primary()
            .lock_filled(
                matches!(activity, Activity::Switching | Activity::Pulling),
                cx,
            )
            .on_click(|_, window, cx| window.close_dialog(cx));
        v_flex()
            .id("branch-dialog")
            .test_support()
            .gap(SPACE_2)
            .child(list)
            .children(notice)
            .child(ui::dialog_footer(
                Some(fetch),
                pull.into_iter().chain(push).chain(stop),
                close,
            ))
    }
}
