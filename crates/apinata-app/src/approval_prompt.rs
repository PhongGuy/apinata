//! The approval prompt: a modal asking the human to trust or deny one requested host, and
//! the helpers that make agent-supplied text safe to show in it.

use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement, Pixels, SharedString,
    StatefulInteractiveElement, Styled, TestSupportExt, WeakEntity, Window, canvas, div,
};
use std::path::Path;

use apinata_core::{display_host, is_hidden_char, is_variation_selector};

use crate::approvals::{Approvals, Decision, Outcome, Prompt};
use crate::shell::Shell;
use crate::ui;

/// The footer hint for Escape, worded as the script prompt words it: the request is dropped.
pub const ESCAPE_HINT: &str = "Esc: dismiss";

/// Longest text, in chars, the prompt shows before cutting it.
const MAX_CHARS: usize = 200;

/// Each [`is_hidden_char`] becomes U+FFFD and each variation selector is removed; past 200
/// chars the text is cut to 200 and "..." appended. Applied to the shown host, collection
/// path and request.
pub fn untrusted_text(text: &str) -> String {
    // Cut by char so no code point splits.
    let mut shown = defanged(&text.chars().take(MAX_CHARS).collect::<String>());
    if text.chars().nth(MAX_CHARS).is_some() {
        shown.push_str("...");
    }
    shown
}

// A hidden character gets a visible mark, not removal, so the human can see something was
// there; nothing is cut.
fn defanged(text: &str) -> String {
    text.chars()
        .filter(|&c| !is_variation_selector(c))
        .map(|c| if is_hidden_char(c) { '\u{fffd}' } else { c })
        .collect()
}

/// The collection's `path` on one line with id `id`: shortened in its middle to the room
/// layout gives it, so its last folder stays in view, with the whole path in its tooltip
/// (`{id}-tooltip`) and accessible label. All shown text passes [`untrusted_text`].
pub fn collection_path_line(
    id: &'static str,
    path: &Path,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<> {
    // The room is measured after layout and kept across frames; a change redraws the window
    // so the path refits. Hidden characters are marked before fitting, so the fit measures
    // what shows.
    let room = window.use_keyed_state(SharedString::from(format!("{id}-room")), cx, |_, _| {
        None::<Pixels>
    });
    let clean = Path::new(&defanged(&path.display().to_string())).to_path_buf();
    let home = dirs::home_dir();
    let shown = match *room.read(cx) {
        Some(room) => crate::path_display::fitted(&clean, home.as_deref(), room, window),
        None => crate::path_display::tilde(&clean, home.as_deref()),
    };
    let shown = untrusted_text(&shown);
    let full = untrusted_text(&path.display().to_string());
    let measure = canvas(
        move |bounds, window, cx| {
            let now = Some(bounds.size.width);
            if room.update(cx, |r, _| std::mem::replace(r, now)) != now {
                window.defer(cx, |window, _| window.refresh());
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full();
    div()
        .id(id)
        .test_support()
        .aria_label(full.clone())
        .aria_value(shown.clone())
        .relative()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .truncate()
        .text_size(crate::style::TEXT_SMALL)
        .text_color(cx.theme().muted_foreground)
        .tooltip(crate::ui::text_tooltip(format!("{id}-tooltip"), full))
        .child(measure)
        .child(shown)
}

/// A warning when any label of `host` is punycode (`xn--`), else None.
pub fn host_note(host: &str) -> Option<&'static str> {
    host.split('.')
        .any(|label| {
            label
                .get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case("xn--"))
        })
        .then_some("This host contains an international name, shown in its encoded form.")
}

/// The line saying which host name a host override sent to `host`. Both parts are shown
/// defanged, the address as a person reads it.
pub fn mapped_line(name: &str, host: &str) -> String {
    format!(
        "{} mapped to {}",
        untrusted_text(name),
        untrusted_text(&display_host(host))
    )
}

/// Opens the modal for `prompt` over `window`, naming its collection `name`. Only a click on
/// "Trust host" or "Deny host" records a decision; Enter, Escape and every other way out
/// dismiss the request.
pub fn open(
    shell: WeakEntity<Shell>,
    approvals: Entity<Approvals>,
    prompt: Prompt,
    name: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    let host: SharedString = untrusted_text(&display_host(&prompt.host)).into();
    let note = host_note(&prompt.host);
    let collection: SharedString = untrusted_text(&name).into();
    let path = prompt.collection.clone();
    let request: SharedString = untrusted_text(&prompt.request).into();
    let mapped: Option<SharedString> = prompt
        .mapped_from
        .as_deref()
        .map(|name| mapped_line(name, &prompt.host).into());
    let proxy: Option<SharedString> = prompt.proxy.as_deref().map(|p| untrusted_text(p).into());
    // The builder runs every frame, so each handler clones what it captures.
    crate::ui::open_dialog(window, cx, move |dialog, window, cx| {
        let dialog = crate::dialog_frame::place(dialog, window);
        // The host line is the dialog's only tab stop: Tab stays inside the dialog, and the
        // buttons are never focusable, so no key can press them.
        let focus = window.use_keyed_state("approval-focus", cx, |_, cx| {
            cx.focus_handle().tab_stop(true)
        });
        let focus: FocusHandle = focus.read(cx).clone();
        let on_key = |shell: &WeakEntity<Shell>, approvals: &Entity<Approvals>, prompt: &Prompt| {
            let (shell, approvals, prompt) = (shell.clone(), approvals.clone(), prompt.clone());
            move |_: &_, _: &mut Window, cx: &mut App| {
                answer(&shell, &approvals, &prompt, None, cx);
                true
            }
        };
        let button = |id: &'static str, label: &'static str, decision: Decision| {
            let (shell, approvals, prompt) = (shell.clone(), approvals.clone(), prompt.clone());
            Button::new(id)
                .label(label)
                .tab_stop(false)
                .on_click(move |event, window, cx| {
                    // The next prompt opens in the same place, so the second half of a
                    // double click must not answer it; a key never answers either.
                    if event.click_count() > 1 || event.is_keyboard() {
                        return;
                    }
                    let outcome = answer(&shell, &approvals, &prompt, Some(decision), cx);
                    window.close_dialog(cx);
                    if let Some(note) = outcome {
                        window.push_notification(note, cx);
                    }
                })
        };
        dialog
            .title("Allow requests to this host?")
            .close_button(false)
            .overlay_closable(false)
            .on_ok(on_key(&shell, &approvals, &prompt))
            .on_cancel(on_key(&shell, &approvals, &prompt))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .id("approval-host")
                            .test_support()
                            .track_focus(&focus)
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_lg()
                            .child(host.clone()),
                    )
                    .when_some(note, |el, note| {
                        el.child(div().id("approval-host-note").test_support().child(note))
                    })
                    .when_some(mapped.clone(), |el, line| {
                        el.child(div().id("approval-mapped").test_support().child(line))
                    })
                    .when_some(proxy.clone(), |el, proxy| {
                        el.child(
                            div()
                                .id("approval-proxy")
                                .test_support()
                                .child(format!("Via proxy {proxy}")),
                        )
                    })
                    .child(
                        div()
                            .id("approval-collection")
                            .test_support()
                            .child(format!("Collection: {collection}")),
                    )
                    .child(collection_path_line(
                        "approval-collection-path",
                        &path,
                        window,
                        cx,
                    ))
                    .child(
                        v_flex()
                            .p_2()
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                div()
                                    .id("approval-untrusted-note")
                                    .test_support()
                                    .child("Reported by the agent; not verified"),
                            )
                            .child(
                                div()
                                    .id("approval-request")
                                    .test_support()
                                    .child(format!("Request: {request}")),
                            ),
                    ),
            )
            // Enter and Escape only dismiss, so the footer says so on the left; the buttons
            // that record a decision stay on the right.
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_between()
                    .child(
                        div()
                            .id("approval-later-hint")
                            .test_support()
                            .text_size(crate::style::TEXT_SMALL)
                            .text_color(cx.theme().muted_foreground)
                            .child(ESCAPE_HINT),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(button("approval-deny", "Deny host", Decision::Deny))
                            .child(button("approval-trust", "Trust host", Decision::Trust)),
                    ),
            )
    });
}

/// Clears the shell's prompt, then records `decision` or, with `None`, dismisses. Returns the
/// notification to show, if any. The closing notify lets the shell open the next prompt once
/// this dialog has closed.
fn answer(
    shell: &WeakEntity<Shell>,
    approvals: &Entity<Approvals>,
    prompt: &Prompt,
    decision: Option<Decision>,
    cx: &mut App,
) -> Option<Notification> {
    let _ = shell.update(cx, |s, _| s.approval_answered());
    approvals.update(cx, |a, cx| {
        let note = match decision {
            None => {
                a.dismiss(prompt, cx);
                None
            }
            Some(decision) => match a.decide(prompt, decision, cx) {
                Ok(Outcome::Recorded) => None,
                Ok(Outcome::AlreadyDecided(Decision::Trust)) => {
                    Some(ui::info_toast("Already decided: trusted"))
                }
                Ok(Outcome::AlreadyDecided(Decision::Deny)) => {
                    Some(ui::info_toast("Already decided: denied"))
                }
                Err(e) => Some(Notification::error(format!(
                    "Could not save the trusted hosts: {e}"
                ))),
            },
        };
        cx.notify();
        note
    })
}
