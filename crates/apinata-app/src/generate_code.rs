//! The Generate Code dialog: resolves a request as Copy as curl does and shows it as a snippet
//! in a chosen target, highlighted and read-only, with a Copy button.

use std::path::Path;

use apinata_core::{ItemPath, ResolvedRequest, Target};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{IndexPath, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClipboardItem, Entity, Global, Rems, SharedString, Subscription, TestSupportExt,
    WeakEntity, Window, div, rems,
};

use crate::copy_curl::{confirm_secrets, plan_and_resolve};
use crate::ui::{RingRoom as _, RingSides};
use crate::workspace::Workspace;
use crate::{style, ui};

// Below this many rems of room the target sits beside the preview instead of above it.
const SIDE_BY_SIDE_BELOW: f32 = 12.;

// The target column's width beside the preview: eight 24 px steps, 192 px at 100%.
const TARGET_COLUMN_WIDTH: Rems = Rems(style::SPACE_5.0 * 8.);

// The target last picked, so the next dialog opens on it.
struct LastTarget(Target);
impl Global for LastTarget {}

// The most recently opened dialog's view; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<GenerateCodeView>);
impl Global for OpenView {}

/// Registers the grammars the kit links without a highlights query, so their snippets are
/// painted rather than shown plain. Safe to call more than once.
pub fn init() {
    use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};
    let csharp = LanguageConfig::new(
        "csharp",
        tree_sitter_c_sharp::LANGUAGE.into(),
        Vec::new(),
        tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
        "",
        "",
    );
    LanguageRegistry::singleton().register("csharp", &csharp);
}

/// The open dialog's view, for tests and the pilot; None when no dialog is open.
pub fn current(cx: &App) -> Option<Entity<GenerateCodeView>> {
    cx.try_global::<OpenView>().and_then(|v| v.0.upgrade())
}

/// Resolves `item` as Send would, asks before showing secret values, then opens the dialog on
/// the last target; on failure it opens nothing and toasts why.
pub fn open_generate_code(
    workspace: &Entity<Workspace>,
    root: &Path,
    item: ItemPath,
    window: &mut Window,
    cx: &mut App,
) {
    let planned = plan_and_resolve(workspace, root, item, window, cx);
    window
        .spawn(cx, async move |cx| {
            let request = match planned.recv().await {
                Ok(Ok(r)) => r,
                Ok(Err(message)) => {
                    let note = Notification::error(format!("Could not generate code: {message}"));
                    cx.update(|window, cx| window.push_notification(note, cx))
                        .ok();
                    return;
                }
                Err(_) => return,
            };
            if !request.sensitive_values().is_empty()
                && !confirm_secrets("Show secrets?", "confirm-show", "Show with secrets", cx).await
            {
                return;
            }
            cx.update(|window, cx| show(request, window, cx)).ok();
        })
        .detach();
}

fn show(request: ResolvedRequest, window: &mut Window, cx: &mut App) {
    let target = cx.try_global::<LastTarget>().map_or(Target::Curl, |t| t.0);
    let view = cx.new(|cx| GenerateCodeView::new(request, target, window, cx));
    cx.set_global(OpenView(view.downgrade()));
    window.open_dialog(cx, move |d, window, _| {
        ui::with_close(crate::dialog_frame::place(d, window), "Generate code")
            // Enter on the target picker must not close the dialog.
            .on_ok(|_, _, _| false)
            .child(view.clone())
    });
}

/// The dialog body: the resolved request, the chosen target and its snippet.
pub struct GenerateCodeView {
    request: ResolvedRequest,
    target: Target,
    text: String,
    select: Entity<SelectState<Vec<SharedString>>>,
    editor: Entity<EditorState>,
    _picked: Subscription,
}

impl GenerateCodeView {
    fn new(
        request: ResolvedRequest,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let labels: Vec<SharedString> = Target::ALL.iter().map(|t| t.label().into()).collect();
        let at = Target::ALL.iter().position(|t| *t == target).unwrap_or(0);
        let select = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(at)), window, cx));
        let editor = cx.new(|cx| EditorState::new(window, cx).searchable(false));
        let picked = cx.subscribe_in(
            &select,
            window,
            |this, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(label)) = event
                    && let Some(t) = Target::ALL.iter().find(|t| t.label() == label.as_ref())
                {
                    this.set_target(*t, window, cx);
                }
            },
        );
        let mut view = GenerateCodeView {
            request,
            target,
            text: String::new(),
            select,
            editor,
            _picked: picked,
        };
        view.set_target(target, window, cx);
        view
    }

    /// The shown target.
    pub fn target(&self) -> Target {
        self.target
    }

    /// The shown snippet: `generate(&request, target)`.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Shows `target`'s snippet with its highlighter and remembers it for the next dialog.
    pub fn set_target(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        self.target = target;
        self.text = apinata_core::generate(&self.request, target);
        cx.set_global(LastTarget(target));
        let text = self.text.clone();
        self.editor.update(cx, |e, cx| {
            e.set_highlighter(highlighter(target), cx);
            e.set_value(text, window, cx);
        });
        if let Some(at) = Target::ALL.iter().position(|t| *t == target) {
            self.select.update(cx, |s, cx| {
                s.set_selected_index(Some(IndexPath::new(at)), window, cx)
            });
        }
        cx.notify();
    }

    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.text.clone()));
        window.push_notification(ui::info_toast("Copied code"), cx);
    }
}

/// The highlighter name for a target's language; the shell targets share bash. PowerShell,
/// which has no kit grammar, and any later target show as plain `"text"`.
pub fn highlighter(target: Target) -> &'static str {
    match target {
        Target::Curl | Target::Httpie => "bash",
        Target::PythonRequests => "python",
        Target::JsFetch | Target::NodeAxios => "javascript",
        Target::GoNative => "go",
        Target::PhpCurl => "php",
        Target::RustReqwest => "rust",
        Target::JavaHttpClient => "java",
        Target::CSharpHttpClient => "csharp",
        _ => "text",
    }
}

impl Render for GenerateCodeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let room = crate::dialog_frame::titled_body_room(window);
        // Too short to stack the target above three code lines, so the target moves beside the
        // preview and the preview takes the whole height.
        let rem = window.rem_size();
        let side_by_side = room < rem * SIDE_BY_SIDE_BELOW;
        let preview_h = if side_by_side {
            room - style::FOCUS_RING_ROOM * 2.
        } else {
            style::CODE_PREVIEW_HEIGHT.to_pixels(rem).min(room * 0.6)
        };
        // Enter on Copy copies; the target select keeps its own Enter.
        let copy = ui::icon_button("generate-code-copy", IconName::Copy, "Copy", None)
            .key_context(ui::DIALOG_FOOTER_CONTEXT)
            .on_click(cx.listener(|this, _, window, cx| this.copy(window, cx)));
        // Tall enough that every target shows without scrolling; the kit still caps the popup
        // to the window. A medium row is about 1.92 rem, plus list padding.
        let target = div().min_w_0().child(ui::dialog_select(
            Select::new(&self.select)
                .id("generate-code-target")
                .menu_max_h(rems(Target::ALL.len() as f32 * 2. + 1.)),
        ));
        let preview = div()
            .id("generate-code-preview")
            .test_support()
            .min_w_0()
            .child(ui::ringed_editor(
                Editor::new(&self.editor).readonly(true).h_full(),
                &self.editor,
            ));
        // Copy sits outside the code, which the kit editor's paddings cannot make room for:
        // right of the target when stacked, under it when side by side, so the code keeps the
        // short layout's whole height. The kit body clips flush with the view, so the padding
        // keeps the rings whole.
        if side_by_side {
            h_flex()
                .id("generate-code")
                .test_support()
                .ring_room(RingSides::ALL)
                .items_start()
                .gap_2()
                .child(
                    v_flex()
                        .w(TARGET_COLUMN_WIDTH)
                        .flex_none()
                        .gap_2()
                        .child(target)
                        .child(h_flex().justify_end().child(copy)),
                )
                .child(preview.h(preview_h).flex_1())
        } else {
            v_flex()
                .id("generate-code")
                .test_support()
                .ring_room(RingSides::ALL)
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(target.flex_1())
                        .child(div().flex_none().child(copy)),
                )
                .child(preview.h(preview_h))
        }
    }
}
