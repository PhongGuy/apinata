//! A themed in-app confirmation dialog that replaces the platform prompt: Enter answers the
//! default, Escape the cancel choice, and each button carries an id the pilot can see.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{WindowExt, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, InteractiveElement, ParentElement, SharedString, StatefulInteractiveElement as _,
    TestSupportExt, Window, div,
};

use crate::ui::dialog_footer;

/// One answer button; `danger` paints it destructive.
#[derive(Clone)]
pub struct Choice {
    /// Element id of the button.
    pub id: &'static str,
    /// Text shown on the button.
    pub label: SharedString,
    /// Paints the button destructive.
    pub danger: bool,
}

/// `default` is the index Enter answers (painted primary unless `danger`);
/// `cancel` the index Escape answers.
#[derive(Clone)]
pub struct Ask {
    /// The question, shown as the dialog title.
    pub title: SharedString,
    /// Optional explanation shown under the title.
    pub detail: Option<SharedString>,
    /// The buttons, left to right.
    pub choices: Vec<Choice>,
    /// Index into `choices` that Enter answers.
    pub default: usize,
    /// Index into `choices` that Escape answers.
    pub cancel: usize,
}

/// Opens the modal over `window` on the next effect cycle, so a popup menu that restores focus
/// after its click handler cannot pull focus out of it. The receiver yields the chosen index
/// once; it errs when the dialog goes away unanswered (window closed).
pub fn ask(spec: Ask, window: &mut Window, cx: &mut App) -> async_channel::Receiver<usize> {
    // A one-slot channel and try_send: whichever answer lands first wins, later ones drop.
    let (tx, rx) = async_channel::bounded(1);
    window.defer(cx, move |window, cx| {
        // The builder runs every frame, so each handler clones what it captures.
        window.open_dialog(cx, move |dialog, window, _| {
            let dialog = crate::dialog_frame::place(dialog, window);
            let key = |ix: usize| {
                let tx = tx.clone();
                move |_: &_, _: &mut Window, _: &mut App| {
                    let _ = tx.try_send(ix);
                    true
                }
            };
            // Enter closes at once like the buttons: the kit's animated close would give focus
            // back only later, on top of wherever the answer sent it.
            let enter = {
                let tx = tx.clone();
                let default = spec.default;
                move |_: &_, window: &mut Window, cx: &mut App| {
                    let _ = tx.try_send(default);
                    window.close_dialog(cx);
                    false
                }
            };
            let mut buttons: Vec<Button> = spec
                .choices
                .iter()
                .enumerate()
                .map(|(ix, c)| {
                    let tx = tx.clone();
                    Button::new(c.id)
                        .label(c.label.clone())
                        .when(c.danger, |b| b.danger())
                        .when(!c.danger && ix == spec.default, |b| b.primary())
                        .on_click(move |_, window, cx| {
                            let _ = tx.try_send(ix);
                            window.close_dialog(cx);
                        })
                })
                .collect();
            // The last choice sits at the right edge; the footer's padding is the body gap.
            let last = buttons.pop();
            dialog
                .title(spec.title.clone())
                .close_button(false)
                .on_ok(enter)
                .on_cancel(key(spec.cancel))
                .child(
                    v_flex()
                        .id("confirm-dialog")
                        // The question names the dialog for screen readers and the pilot.
                        .aria_label(spec.title.clone())
                        .test_support()
                        .when_some(spec.detail.clone(), |el, detail| {
                            el.child(
                                div()
                                    .id("confirm-detail")
                                    .test_support()
                                    .aria_label(detail.clone())
                                    .child(detail),
                            )
                        })
                        .when_some(last, |el, last| {
                            el.child(dialog_footer(None, buttons, last))
                        }),
                )
        });
    });
    rx
}

/// The delete prompt: `[Cancel, Delete]`, Delete the danger-styled default (index 1).
pub fn delete_ask(title: impl Into<SharedString>, detail: Option<&'static str>) -> Ask {
    Ask {
        title: title.into(),
        detail: detail.map(SharedString::from),
        choices: vec![
            Choice {
                id: "confirm-cancel",
                label: "Cancel".into(),
                danger: false,
            },
            Choice {
                id: "confirm-delete",
                label: "Delete".into(),
                danger: true,
            },
        ],
        default: 1,
        cancel: 0,
    }
}

/// The refusal shown instead of the delete prompt while other environments extend `name`:
/// `kind` ("Environment" or "Global environment") titles it, and Cancel is the only choice.
pub fn extended_ask(kind: &str, name: &str, by: &[String]) -> Ask {
    let by: Vec<String> = by.iter().map(|b| format!("\"{b}\"")).collect();
    Ask {
        title: format!("{kind} \"{name}\" is extended by {}", by.join(", ")).into(),
        detail: Some("Change their extends first.".into()),
        choices: vec![Choice {
            id: "confirm-cancel",
            label: "Cancel".into(),
            danger: false,
        }],
        default: 0,
        cancel: 0,
    }
}

/// The discard prompt: `[Cancel, Discard]`, Discard the danger-styled default (index 1).
pub fn discard_ask(title: impl Into<SharedString>) -> Ask {
    Ask {
        title: title.into(),
        detail: None,
        choices: vec![
            Choice {
                id: "confirm-cancel",
                label: "Cancel".into(),
                danger: false,
            },
            Choice {
                id: "confirm-discard",
                label: "Discard".into(),
                danger: true,
            },
        ],
        default: 1,
        cancel: 0,
    }
}

/// The save prompt: `[Cancel, discard, save]`, save the primary default (index 2).
pub fn save_ask(
    title: SharedString,
    detail: Option<SharedString>,
    discard: &'static str,
    save: &'static str,
) -> Ask {
    Ask {
        title,
        detail,
        choices: vec![
            Choice {
                id: "confirm-cancel",
                label: "Cancel".into(),
                danger: false,
            },
            Choice {
                id: "confirm-discard",
                label: discard.into(),
                danger: false,
            },
            Choice {
                id: "confirm-save",
                label: save.into(),
                danger: false,
            },
        ],
        default: 2,
        cancel: 0,
    }
}

/// The prompt before `action` drops `count` staged copies whose files left their paths:
/// `[Cancel, action]`. Cancel is the default, since Enter or Space on a row can bring this up.
pub fn drop_staged_ask(count: usize, action: &'static str) -> Ask {
    let (title, detail) = match count {
        1 => (
            "Drop 1 staged copy?".to_owned(),
            "It is gone from its path, so the index may hold the only copy of the edit.",
        ),
        n => (
            format!("Drop {n} staged copies?"),
            "They are gone from their paths, so the index may hold the only copies of the edits.",
        ),
    };
    careful_ask(title, detail, "confirm-drop", action)
}

/// [`drop_staged_ask`] for a list cut short: `listed` gone copies were seen, and more past the
/// end may be gone too, so the count is a floor and none listed still asks.
pub fn drop_staged_past_cap_ask(listed: usize, action: &'static str) -> Ask {
    let (title, detail) = match listed {
        0 => (
            "Drop any staged copies past the list?".to_owned(),
            "The list stops short, and staged files past it may be gone from their paths, so the \
             index may hold the only copies of the edits.",
        ),
        n => (
            format!("Drop {n} or more staged copies?"),
            "They are gone from their paths, and staged files past the end of the list may be \
             too, so the index may hold the only copies of the edits.",
        ),
    };
    careful_ask(title, detail, "confirm-drop", action)
}

// A question whose safe answer is Cancel: the default, and Escape's answer too, so an accidental
// Enter changes nothing. The confirming button is index 1.
fn careful_ask(title: String, detail: &'static str, id: &'static str, label: &'static str) -> Ask {
    Ask {
        title: title.into(),
        detail: Some(detail.into()),
        choices: vec![
            Choice {
                id: "confirm-cancel",
                label: "Cancel".into(),
                danger: false,
            },
            Choice {
                id,
                label: label.into(),
                danger: true,
            },
        ],
        default: 0,
        cancel: 0,
    }
}

/// Overwriting the stored values of `names`: `[Cancel, Replace]`, Cancel the default.
pub fn replace_secret_ask(names: &[String]) -> Ask {
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    replace_entries_ask(&quoted)
}

/// [`replace_secret_ask`] over `entries` already quoted, each perhaps naming its environment.
pub fn replace_entries_ask(entries: &[String]) -> Ask {
    let (title, detail) = match entries {
        [one] => (
            format!("Replace the stored value of {one}?"),
            "A value is already stored for this name. It is overwritten and cannot be recovered.",
        ),
        many => (
            format!("Replace the stored values of {}?", many.join(", ")),
            "Values are already stored for these names. They are overwritten and cannot be \
             recovered.",
        ),
    };
    careful_ask(title, detail, "confirm-replace", "Replace")
}

/// Dropping typing in secret rows named like another secret row, which no Save can store:
/// `[Cancel, Discard]`, Cancel the default.
pub fn discard_twin_ask(names: &[String]) -> Ask {
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    careful_ask(
        format!("Discard the value typed for {}?", quoted.join(", ")),
        "Secret names must be unique, so it cannot be stored.",
        "confirm-discard",
        "Discard",
    )
}

/// Moving the value of `name` into the file: `[Cancel, Make plain]`, Cancel the default.
pub fn move_secret_ask(name: &str) -> Ask {
    careful_ask(
        format!("Move the value of \"{name}\" into the file?"),
        "It is written to this environment's file as plain text, where git can see it, and \
         leaves the secret store.",
        "confirm-make-plain",
        "Make plain",
    )
}

/// The window-close question while a collection run is in flight: Cancel (default) or stop.
pub fn stop_run_ask() -> Ask {
    Ask {
        title: "Stop the run and close?".into(),
        detail: Some("A collection run is still going.".into()),
        choices: vec![
            Choice {
                id: "confirm-cancel",
                label: "Cancel".into(),
                danger: false,
            },
            Choice {
                id: "confirm-stop",
                label: "Stop and close".into(),
                danger: true,
            },
        ],
        default: 0,
        cancel: 0,
    }
}
