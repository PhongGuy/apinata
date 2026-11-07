//! The "Input required" dialog that asks for a request's `{{?label}}` prompt variables, and
//! the per-collection memory of answers given this app session.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyWindowHandle, App, AsyncApp, Entity, FocusHandle, TestSupportExt, Window};

use crate::style::{SPACE_2, SPACE_3};
use crate::ui;

/// Answers given this app session in one open collection, by label; never written to disk.
#[derive(Clone, Default)]
pub struct PromptMemory {
    answers: Arc<Mutex<BTreeMap<String, String>>>,
}

impl PromptMemory {
    /// The last answer given for `label`, if any.
    pub fn get(&self, label: &str) -> Option<String> {
        self.lock().get(label).cloned()
    }

    /// Keeps each answer, replacing an earlier one for the same label.
    pub fn remember(&self, answers: &[(String, String)]) {
        let mut map = self.lock();
        for (label, value) in answers {
            map.insert(label.clone(), value.clone());
        }
    }

    // A panic while holding the lock leaves the map intact, so a poisoned lock is still usable.
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
        self.answers.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The dialog's form: one input per prompt label, in label order.
pub struct PromptForm {
    labels: Vec<String>,
    inputs: Vec<Entity<InputState>>,
}

impl PromptForm {
    /// One field per label, pre-filled from `memory`; the first field is focused.
    pub fn new(
        labels: &[String],
        memory: &PromptMemory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs: Vec<_> = labels
            .iter()
            .map(|label| {
                let value = memory.get(label).unwrap_or_default();
                cx.new(|cx| InputState::new(window, cx).default_value(value))
            })
            .collect();
        if let Some(first) = inputs.first() {
            first.update(cx, |i, cx| i.focus(window, cx));
        }
        Self {
            labels: labels.to_vec(),
            inputs,
        }
    }

    /// The fields, one per label.
    pub fn inputs(&self) -> &[Entity<InputState>] {
        &self.inputs
    }

    /// (label, value) per field, in label order; values exactly as typed.
    pub fn values(&self, cx: &App) -> Vec<(String, String)> {
        self.labels
            .iter()
            .zip(&self.inputs)
            .map(|(label, input)| (label.clone(), input.read(cx).value().to_string()))
            .collect()
    }
}

impl Render for PromptForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().id("prompt-vars-form").gap(SPACE_2).children(
            self.labels
                .iter()
                .zip(&self.inputs)
                .enumerate()
                .map(|(i, (label, input))| {
                    ui::form_row(
                        format!("prompt-var-{i}-row"),
                        label.clone(),
                        ui::padded_input(
                            Input::new(input)
                                .id(format!("prompt-var-{i}"))
                                .w_full()
                                .bg(ui::field_fill(false, cx)),
                            input,
                        ),
                    )
                }),
        )
    }
}

/// Opens "Input required" over `window`; Some(values) on Continue or Enter, None on Cancel or Escape.
pub fn ask(
    form: Entity<PromptForm>,
    window: &mut Window,
    cx: &mut App,
) -> async_channel::Receiver<Option<Vec<(String, String)>>> {
    open(form, window, cx).0
}

type Answer = Option<Vec<(String, String)>>;

// `ask`, also returning a slot that holds the dialog's focus handle once the deferred open ran.
fn open(
    form: Entity<PromptForm>,
    window: &mut Window,
    cx: &mut App,
) -> (
    async_channel::Receiver<Answer>,
    Rc<RefCell<Option<FocusHandle>>>,
) {
    // A one-slot channel and try_send: whichever answer lands first wins, later ones drop.
    let (tx, rx) = async_channel::bounded(1);
    let slot = Rc::new(RefCell::new(None));
    let opened = slot.clone();
    window.defer(cx, move |window, cx| {
        // Nobody waits for an answer any more, so the dialog never opens.
        if tx.is_closed() {
            return;
        }
        let first = form.read(cx).inputs.first().cloned();
        let builder_form = form.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let dialog = crate::dialog_frame::place(dialog, window);
            // Every answer is accepted as typed, empty included, so Continue is always enabled.
            let answer = {
                let (tx, form) = (tx.clone(), builder_form.clone());
                move |cx: &mut App| {
                    let _ = tx.try_send(Some(form.read(cx).values(cx)));
                }
            };
            let on_ok = answer.clone();
            let (cancel_tx, click_tx) = (tx.clone(), tx.clone());
            let buttons = h_flex()
                .key_context(ui::DIALOG_FOOTER_CONTEXT)
                .gap_2()
                .justify_end()
                .child(Button::new("prompt-vars-cancel").label("Cancel").on_click(
                    move |_, window, cx| {
                        let _ = click_tx.try_send(None);
                        window.close_dialog(cx);
                    },
                ))
                .child(
                    Button::new("prompt-vars-continue")
                        .label("Continue")
                        .primary()
                        .on_click(move |_, window, cx| {
                            answer(cx);
                            window.close_dialog(cx);
                        }),
                );
            dialog
                .title("Input required")
                .close_button(false)
                .on_ok(move |_, _, cx| {
                    on_ok(cx);
                    true
                })
                .on_cancel(move |_, _, _| {
                    let _ = cancel_tx.try_send(None);
                    true
                })
                .child(
                    v_flex()
                        .id("prompt-vars-dialog")
                        .test_support()
                        .gap(SPACE_3)
                        .child(builder_form.clone())
                        .child(buttons),
                )
        });
        // The kit focuses a new dialog's own handle as it opens, so this is ours.
        *opened.borrow_mut() = window.focused(cx);
        // The dialog takes focus as it opens; the first field takes it back, ready to type.
        if let Some(first) = first {
            first.update(cx, |i, cx| i.focus(window, cx));
        }
    });
    (rx, slot)
}

/// Asks for `labels` in the dialog window. Continue stores the answers in `memory` and returns
/// them; Cancel or a dialog closed unanswered returns None; no window returns Some(empty).
pub async fn prompt_answers(
    labels: Vec<String>,
    memory: PromptMemory,
    cx: &mut AsyncApp,
) -> Option<Vec<(String, String)>> {
    let Some(window) = cx.update(|cx| crate::script_gate::dialog_window(cx)) else {
        return Some(Vec::new());
    };
    let (rx, slot) = window
        .update(cx, |_, window, cx| {
            let form = cx.new(|cx| PromptForm::new(&labels, &memory, window, cx));
            open(form, window, cx)
        })
        .ok()?;
    let mut guard = CloseOnDrop {
        window,
        slot,
        cx: cx.clone(),
        answered: false,
    };
    let answer = rx.recv().await;
    guard.answered = true;
    let answers = answer.ok()??;
    memory.remember(&answers);
    Some(answers)
}

// Closes the dialog when the asking task is dropped unanswered, as a cancelled send does.
// The kit only pops the top dialog, so it closes only while focus is inside ours: a modal
// stack keeps focus in the top dialog, and any doubt leaves the dialog open instead.
struct CloseOnDrop {
    window: AnyWindowHandle,
    slot: Rc<RefCell<Option<FocusHandle>>>,
    cx: AsyncApp,
    answered: bool,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        let Some(ours) = self.slot.borrow_mut().take() else {
            return;
        };
        // A drop can run inside an App update, so the close waits for its own turn.
        let (window, mut cx) = (self.window, self.cx.clone());
        self.cx
            .foreground_executor()
            .spawn(async move {
                let _ = window.update(&mut cx, |_, window, cx| {
                    if ours.is_focused(window) || ours.contains_focused(window, cx) {
                        window.close_dialog(cx);
                    }
                });
            })
            .detach();
    }
}
