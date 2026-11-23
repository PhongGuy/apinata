//! Renaming the request from the breadcrumb: a double-click, or F2 or Enter on the focused
//! name, swaps the name for a field; the rename itself goes through the sidebar's.

use std::path::Path;

use apinata_core::ItemPath;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, KeyDownEvent, SharedString, Subscription, TestSupportExt,
    Window, div, rems,
};

use super::RequestEditor;
use crate::workspace::Workspace;
use crate::{style, ui};

const RENAME_TIP: &str = "Double-click to rename";

/// A retyping of the request's name in progress; dropping it drops the input's subscription.
pub(super) struct TitleRename {
    old: String,
    // The name Enter refused and why, until the name is edited.
    error: Option<(SharedString, String)>,
    input: Entity<InputState>,
    _subscription: Subscription,
}

impl TitleRename {
    /// Why the typed name was refused, while the field shows it.
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_ref().map(|(_, e)| e.as_str())
    }
}

/// Whether the breadcrumb may rename `item` of `root`: only a saved request in a collection
/// that takes saves, so never a draft, the scratch pad or a read-only team collection.
pub(super) fn renames(workspace: &Workspace, root: &Path, item: &ItemPath) -> bool {
    !item.is_draft() && !workspace.is_scratch(root) && !workspace.is_team_read_only(root)
}

impl RequestEditor {
    /// Opens the name field with the whole name selected, when the request renames here.
    pub fn start_title_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((root, item)) = self.target(cx) else {
            return;
        };
        if self.title_rename.is_some() || !renames(self.workspace.read(cx), &root, &item) {
            return;
        }
        let Some(view) = self.workspace.read(cx).request(&root, &item, cx) else {
            return;
        };
        let old = view.name.clone();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(old.clone()));
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        });
        self.title_rename = Some(TitleRename {
            old,
            error: None,
            _subscription: cx.subscribe_in(&input, window, Self::title_rename_event),
            input,
        });
        cx.notify();
    }

    // Enter and clicking away commit; a sibling's name keeps the field open on Enter and is
    // dropped with a toast on blur. Switching windows keeps the field focused, so it stays.
    fn title_rename_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => {
                let clash = (self.title_rename.as_ref()).and_then(|r| self.title_clash(r, cx));
                if let Some(r) = &mut self.title_rename
                    && let Some(error) = clash
                {
                    r.error = Some((r.input.read(cx).value(), error));
                    cx.notify();
                    return;
                }
                if let Some(r) = self.title_rename.take() {
                    self.commit_title(r, cx);
                }
                self.end_title_rename(window, cx);
            }
            InputEvent::Blur if window.is_window_active() => {
                if let Some(r) = self.title_rename.take() {
                    match self.title_clash(&r, cx) {
                        Some(error) => {
                            let message = format!("{error}, so the name was not changed.");
                            window.push_notification(Notification::error(message), cx);
                        }
                        None => self.commit_title(r, cx),
                    }
                }
                cx.notify();
            }
            // Enter echoes the unchanged text; only a real edit answers the refusal.
            InputEvent::Change => {
                if let Some(r) = &mut self.title_rename
                    && r.error
                        .take_if(|(refused, _)| *refused != r.input.read(cx).value())
                        .is_some()
                {
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    fn title_clash(&self, r: &TitleRename, cx: &App) -> Option<String> {
        let (root, item) = self.target(cx)?;
        let name = r.input.read(cx).value().trim().to_string();
        crate::sidebar::name_clash(self.workspace.read(cx), &root, &item, &r.old, &name, cx)
    }

    // Hands a real change to the sidebar's rename; a blank or unchanged name sends nothing.
    fn commit_title(&mut self, r: TitleRename, cx: &mut Context<Self>) {
        let name = r.input.read(cx).value().trim().to_string();
        let Some((root, item)) = self.target(cx) else {
            return;
        };
        if name.is_empty() || name == r.old {
            return;
        }
        self.workspace
            .update(cx, |w, cx| w.rename_item(&root, item, name, cx));
    }

    // Escape drops the typed name.
    fn cancel_title_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.title_rename = None;
        self.end_title_rename(window, cx);
    }

    // Closes the field and hands focus back to the name crumb.
    fn end_title_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.title_rename = None;
        self.title_focus.focus(window, cx);
        cx.notify();
    }

    // The request's name at the end of the breadcrumb: the field while retyped, a focusable
    // crumb that a double-click, F2 or Enter renames, or plain text where it cannot rename.
    pub(super) fn title_crumb(
        &self,
        title: String,
        renames: bool,
        editing: Option<Option<String>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let crumb = div().id("breadcrumb-file").test_support().flex_none();
        if let (Some(error), Some(r)) = (editing, &self.title_rename) {
            let field = ui::invalid_input(
                Input::new(&r.input).xsmall().bg(ui::field_fill(false, cx)),
                div(),
                &r.input,
                error.is_some(),
                (
                    "breadcrumb-rename-danger-ring".into(),
                    "breadcrumb-rename-invalid-icon".into(),
                ),
                window,
                cx,
            );
            return crumb
                .w(rems(16.))
                .flex_shrink(1.)
                .min_w(rems(6.))
                .child(
                    v_flex()
                        .id("breadcrumb-rename-field")
                        .test_support()
                        .w_full()
                        .on_action(cx.listener(|this, _: &Escape, window, cx| {
                            this.cancel_title_rename(window, cx)
                        }))
                        .child(field)
                        .when_some(error, |d, message| {
                            d.child(div().child(ui::field_refusal(
                                "breadcrumb-rename-error",
                                message.into(),
                                cx,
                            )))
                        }),
                )
                .into_any_element();
        }
        if !renames {
            return crumb
                .text_color(cx.theme().foreground)
                .child(title)
                .into_any_element();
        }
        let editor = cx.entity().downgrade();
        let label = format!("Rename request {title}");
        // F2 has no binding outside the tree, so the crumb answers the key itself.
        let on_key = cx.listener(|this, e: &KeyDownEvent, window, cx| {
            let k = &e.keystroke;
            if k.key == "f2" && !k.modifiers.modified() {
                this.start_title_rename(window, cx);
                cx.stop_propagation();
            }
        });
        crumb
            .on_key_down(on_key)
            .tooltip(ui::text_tooltip("breadcrumb-file-tooltip", RENAME_TIP))
            .child(
                Button::new("breadcrumb-rename")
                    .ghost()
                    .xsmall()
                    .px(style::SPACE_1)
                    .py(style::SPACE_0)
                    .min_h(style::CRUMB_MIN_HEIGHT)
                    .label(title)
                    .accessibility_label(label)
                    .track_focus(&self.title_focus)
                    // A single click stays inert, so the name is not a trap for a stray click.
                    .on_click(move |e, window, cx| {
                        if e.click_count() > 1 || e.is_keyboard() {
                            _ = editor.update(cx, |this, cx| this.start_title_rename(window, cx));
                        }
                    }),
            )
            .into_any_element()
    }
}
