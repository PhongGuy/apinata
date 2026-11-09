//! The Keyboard section of Preferences: one row per registry command with its keys, a
//! recorder that takes a new key with a conflict prompt, a reset per edited command and a
//! reset for all of them.

use gpui_kit::assets::IconName;
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::button::Button;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Entity, Hsla, Keystroke, Modifiers, SharedString, Subscription,
    TestSupportExt, Window, div,
};
use std::rc::Rc;

use crate::app::AppState;
use crate::commands::{Command, registry};
use crate::confirm::{self, Ask, Choice};
use crate::keymap::{kit_bindings, set_overrides};
use crate::persisted::Persisted;
use crate::prefs_section::PrefsSection;
use crate::shortcuts::{
    Chord, Overrides, Resolution, assign, conflicted, conflicts_with, diagnose, effective,
    input_chord, reset, reset_all, reset_conflict, reset_resolving, scope_label, shown_place,
};
use crate::style::{SPACE_1, TEXT_SMALL};
use crate::ui;

const NOTE: &str = "Shortcuts are saved in keymap.json. Edits made to that file while the app runs are replaced by the next change here.";
const INVALID: &str = "Invalid entry in keymap.json; using the default";
const PRESS: &str = "Press a key";
const NEEDS_MODIFIER: &str = "Hold Ctrl, Alt or another modifier with this key";
const INPUT_KEY: &str = "Text fields use this key; it will not work while one has focus";

// A recorded chord, or a default a reset would take back, that other commands already
// answer to, waiting for the user's answer.
struct Prompt {
    chord: Chord,
    others: Vec<&'static str>,
    reset: bool,
}

/// The section's state: it follows the live keymap and rebuilds its rows on every change.
pub struct KeyboardPrefs {
    keymap: Option<Entity<Persisted<Overrides>>>,
    _sub: Option<Subscription>,
    // The row being recorded or prompted, with its message; one row at a time.
    row: Option<&'static str>,
    message: Option<String>,
    prompt: Option<Prompt>,
    // Held only while recording, so every keystroke goes to the recorder and nowhere else.
    intercept: Option<Subscription>,
}

impl KeyboardPrefs {
    /// A section observing `AppState.keymap`; without AppState it shows the defaults.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let keymap = cx.try_global::<AppState>().map(|s| s.keymap.clone());
        let sub = keymap
            .as_ref()
            .map(|k| cx.observe(k, |_, _, cx| cx.notify()));
        KeyboardPrefs {
            keymap,
            _sub: sub,
            row: None,
            message: None,
            prompt: None,
            intercept: None,
        }
    }

    fn overrides(&self, cx: &App) -> Overrides {
        self.keymap
            .as_ref()
            .map(|k| k.read(cx).get().clone())
            .unwrap_or_default()
    }

    // Starts recording row `id`. The interceptor holds a weak handle, so closing
    // Preferences drops the entity and with it the subscription.
    fn record(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.row = Some(id);
        self.message = None;
        self.prompt = None;
        let this = cx.entity().downgrade();
        self.intercept = Some(cx.intercept_keystrokes(move |event, _, cx| {
            cx.stop_propagation();
            let keystroke = event.keystroke.clone();
            this.update(cx, |this, cx| this.pressed(&keystroke, cx))
                .ok();
        }));
        cx.notify();
    }

    // Ends recording and any prompt, leaving the keys as they are.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.row = None;
        self.message = None;
        self.prompt = None;
        self.intercept = None;
        cx.notify();
    }

    // Modifier taps are ignored, a bare Escape cancels, a bare key asks for a modifier,
    // a taken key asks how to settle it, and any other key is assigned at once.
    fn pressed(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let (Some(id), None) = (self.row, &self.prompt) else {
            return;
        };
        let Some(chord) = Chord::from_keystroke(keystroke) else {
            return;
        };
        let bare = chord.keystroke().modifiers == Modifiers::none();
        if bare && chord.keystroke().key == "escape" {
            return self.cancel(cx);
        }
        if chord.needs_modifier() {
            self.message = Some(NEEDS_MODIFIER.into());
            return cx.notify();
        }
        let cmds = registry();
        let others = conflicts_with(&cmds, &self.overrides(cx), id, &chord);
        self.intercept = None;
        if others.is_empty() {
            return self.settle(id, chord, Resolution::Replace, cx);
        }
        self.ask(chord, others, false, cx);
    }

    // Shows the conflict prompt on the current row, naming the chord and who holds it.
    fn ask(
        &mut self,
        chord: Chord,
        others: Vec<&'static str>,
        reset: bool,
        cx: &mut Context<Self>,
    ) {
        let cmds = registry();
        let scope_of = |id: &str| cmds.iter().find(|c| c.id == id).map(|c| c.scope);
        let this = self.row.and_then(scope_of);
        // A command that takes the key deeper in, where it wins, says where that is.
        let labels: Vec<String> = others
            .iter()
            .filter_map(|o| cmds.iter().find(|c| c.id == *o))
            .map(|c| match this.and_then(|t| shown_place(t, c.scope)) {
                Some(place) => format!("{} {place}", c.label),
                None => c.label.to_string(),
            })
            .collect();
        let shown = Kbd::format(chord.keystroke());
        self.message = Some(format!("{shown} is already used by {}", labels.join(", ")));
        self.prompt = Some(Prompt {
            chord,
            others,
            reset,
        });
        cx.notify();
    }

    // A reset whose default chord another command now holds asks first, as a recording does.
    fn reset(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let overrides = self.overrides(cx);
        let Some(others) = reset_conflict(&overrides, id) else {
            self.cancel(cx);
            return set_overrides(|o| reset(o, id), cx);
        };
        let cmds = registry();
        let taken = cmds.iter().find(|c| c.id == id).and_then(|cmd| {
            cmd.default_chords
                .iter()
                .filter_map(|d| Chord::parse(d))
                .find(|k| !conflicts_with(&cmds, &overrides, id, k).is_empty())
        });
        let Some(chord) = taken else {
            self.cancel(cx);
            return set_overrides(|o| reset(o, id), cx);
        };
        self.cancel(cx);
        self.row = Some(id);
        self.ask(chord, others, true, cx);
    }

    // Writes the chord, then notes when a text field's own binding would take it first.
    fn settle(&mut self, id: &'static str, chord: Chord, how: Resolution, cx: &mut Context<Self>) {
        let shadowed = input_chord(&kit_bindings(cx), &chord);
        set_overrides(|o| assign(&registry(), o, id, chord, how), cx);
        self.cancel(cx);
        if shadowed {
            self.row = Some(id);
            self.message = Some(INPUT_KEY.into());
        }
    }

    // The prompt's Replace or Swap answer.
    fn resolve(&mut self, how: Resolution, cx: &mut Context<Self>) {
        match (self.row, self.prompt.take()) {
            (Some(id), Some(prompt)) if prompt.reset => {
                set_overrides(|o| reset_resolving(&registry(), o, id, how), cx);
                self.cancel(cx);
            }
            (Some(id), Some(prompt)) => self.settle(id, prompt.chord, how, cx),
            _ => {}
        }
    }

    /// The section as of the keymap now, appended after History. Does not notify.
    pub fn section(&mut self, cx: &mut Context<Self>) -> PrefsSection {
        let overrides = self.overrides(cx);
        let cmds = registry();
        let diag = diagnose(&cmds, &overrides);
        let clash = conflicted(&cmds, &overrides);
        let muted = cx.theme().muted_foreground;
        let mut section = PrefsSection::new("prefs-keyboard", "Keyboard");
        for cmd in &cmds {
            let flags = RowFlags {
                edited: overrides.contains_key(cmd.id),
                invalid: diag.is_invalid(cmd.id),
                conflict: clash.contains(cmd.id),
            };
            let chords = effective(cmd, &overrides);
            let terms = row_terms(cmd, &chords);
            let keys = self.keys_cell(cmd.id, &chords, muted, cx);
            let id = cmd.id;
            let on_reset = cx.listener(move |this, _, _, cx| this.reset(id, cx));
            let control = row_control(cmd, keys, flags, muted, on_reset);
            let control = self.with_prompt(cmd.id, control, muted, cx);
            section = section.row_with_terms(format!("key-{}", cmd.id), cmd.label, terms, control);
        }
        let reset_all_button = Button::new("keys-reset-all")
            .key_context(ui::DIALOG_FOOTER_CONTEXT)
            .label("Reset all")
            .on_click(|_, window, cx| ask_reset_all(window, cx));
        section
            .row("pref-keys-reset-all", "All shortcuts", reset_all_button)
            .note(ui::form_note("prefs-keyboard-note", NOTE))
    }
}

impl KeyboardPrefs {
    // The keys cell: a click starts recording, and while it records it reads "Press a key"
    // and a mouse-down anywhere else cancels.
    fn keys_cell(
        &self,
        id: &'static str,
        chords: &[Chord],
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let recording = self.row == Some(id) && self.intercept.is_some();
        let label = if recording {
            PRESS.to_string()
        } else if chords.is_empty() {
            "Unbound".to_string()
        } else {
            chords
                .iter()
                .map(|c| crate::shortcuts::unparse(c.keystroke()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let keys = h_flex()
            .id(SharedString::from(format!("key-{id}-keys")))
            .cursor_pointer()
            .gap(SPACE_1)
            .on_click(cx.listener(move |this, _, _, cx| this.record(id, cx)));
        let keys = if recording {
            keys.on_mouse_down_out(cx.listener(|this, _, _, cx| this.cancel(cx)))
                .child(div().text_color(muted).child(PRESS))
        } else if chords.is_empty() {
            keys.child(div().text_color(muted).child("Unbound"))
        } else {
            keys.children(chords.iter().map(|c| Kbd::new(c.keystroke().clone())))
        };
        keys.aria_label(label).test_support()
    }

    // Puts the row's message and, after a taken key, its Replace / Swap / Cancel below.
    fn with_prompt(
        &self,
        id: &'static str,
        control: impl IntoElement,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mine = self.row == Some(id);
        let message = self.message.clone().filter(|_| mine);
        let prompt = self.prompt.as_ref().filter(|_| mine);
        // The column fills the row's control cell, keys first, so a long message wraps
        // inside the dialog instead of widening the row past it.
        v_flex()
            .w_full()
            .min_w_0()
            .items_start()
            .gap(SPACE_1)
            .child(control)
            .when_some(message, |el, text| {
                el.child(
                    div()
                        .id(SharedString::from(format!("key-{id}-message")))
                        .aria_label(text.clone())
                        .test_support()
                        .max_w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .text_size(TEXT_SMALL)
                        .text_color(muted)
                        .child(text),
                )
            })
            .when_some(prompt, |el, prompt| {
                let swap = prompt.others.len() == 1;
                el.child(
                    h_flex()
                        .gap(SPACE_1)
                        .child(answer(id, "replace", "Replace", {
                            let view = cx.entity();
                            move |_, cx: &mut App| {
                                view.update(cx, |this, cx| this.resolve(Resolution::Replace, cx))
                            }
                        }))
                        .when(swap, |el| {
                            el.child(answer(id, "swap", "Swap", {
                                let view = cx.entity();
                                move |_, cx: &mut App| {
                                    view.update(cx, |this, cx| this.resolve(Resolution::Swap, cx))
                                }
                            }))
                        })
                        .child(answer(id, "cancel", "Cancel", {
                            let view = cx.entity();
                            move |_, cx: &mut App| view.update(cx, |this, cx| this.cancel(cx))
                        })),
                )
            })
    }
}

// One prompt answer: the wrapper carries the id, the Button takes the click.
fn answer(
    id: &str,
    what: &str,
    label: &'static str,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let wrapper = SharedString::from(format!("key-{id}-{what}"));
    let on_press = Rc::new(on_press);
    let on_key = Rc::clone(&on_press);
    div()
        .id(wrapper.clone())
        .test_support()
        // Enter in a dialog arrives as the dialog's Confirm; the focused answer takes it.
        .on_action(move |_: &Confirm, window, cx| {
            cx.stop_propagation();
            on_key(window, cx);
        })
        .child(
            Button::new(SharedString::from(format!("{wrapper}-button")))
                .label(label)
                .small()
                .on_click(move |_, window, cx| on_press(window, cx)),
        )
}

// What the search also matches: the group, and each chord as file text and as shown.
fn row_terms(cmd: &Command, chords: &[crate::shortcuts::Chord]) -> Vec<String> {
    let mut terms = vec![cmd.group.label().to_string()];
    for chord in chords {
        terms.push(crate::shortcuts::unparse(chord.keystroke()));
        terms.push(Kbd::format(chord.keystroke()));
    }
    terms
}

/// What a shortcut row marks beside its keys.
#[derive(Debug, Clone, Copy)]
struct RowFlags {
    /// The keys differ from the defaults, so the row offers Reset.
    edited: bool,
    /// The keymap entry for the command cannot be used.
    invalid: bool,
    /// Another command shares one of the keys.
    conflict: bool,
}

fn row_control(
    cmd: &Command,
    keys: impl IntoElement,
    RowFlags {
        edited,
        invalid,
        conflict,
    }: RowFlags,
    muted: Hsla,
    on_reset: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = cmd.id;
    h_flex()
        .gap(SPACE_1)
        // A command that works in one pane only names that pane, so its key reads as local.
        .children(scope_label(cmd.scope).map(|area| {
            div()
                .id(SharedString::from(format!("key-{id}-scope")))
                .aria_label(area)
                .test_support()
                .text_xs()
                .text_color(muted)
                .child(area)
        }))
        .child(keys)
        .when(conflict, |el| {
            el.child(
                div()
                    .id(SharedString::from(format!("key-{id}-conflict")))
                    .test_support()
                    .child(Tag::warning().small().child("Conflict")),
            )
        })
        .when(invalid, |el| {
            el.child(
                div()
                    .id(SharedString::from(format!("key-{id}-invalid")))
                    .aria_label(INVALID)
                    .test_support()
                    .child(Tag::danger().small().child("Invalid")),
            )
        })
        .when(edited, |el| {
            let reset_id = SharedString::from(format!("key-{id}-reset"));
            // The press lives on the button, so Enter or Space on it resets too.
            el.child(
                div().id(reset_id.clone()).test_support().child(
                    ui::icon_button(
                        SharedString::from(format!("{reset_id}-button")),
                        IconName::Undo2,
                        "Reset",
                        None,
                    )
                    .key_context(ui::DIALOG_FOOTER_CONTEXT)
                    .on_click(on_reset),
                ),
            )
        })
}

// The confirm sits over Preferences; only a "Reset all" answer touches the keymap.
fn ask_reset_all(window: &mut Window, cx: &mut App) {
    let answer = confirm::ask(
        Ask {
            title: "Reset all shortcuts?".into(),
            detail: Some("Every command returns to its default keys.".into()),
            choices: vec![
                Choice {
                    id: "confirm-cancel",
                    label: "Cancel".into(),
                    danger: false,
                },
                Choice {
                    id: "keys-reset-all-confirm",
                    label: "Reset all".into(),
                    danger: true,
                },
            ],
            default: 1,
            cancel: 0,
        },
        window,
        cx,
    );
    window
        .spawn(cx, async move |cx| {
            if answer.recv().await == Ok(1) {
                cx.update(|_, cx| set_overrides(|o| reset_all(&registry(), o), cx))
                    .ok();
            }
        })
        .detach();
}
