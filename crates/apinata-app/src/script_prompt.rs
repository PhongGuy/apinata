//! The script approval prompt: a modal listing the script texts an agent asked to run, each
//! as it is on disk now with secrets masked, for the human to approve or leave unapproved.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::{
    App, Entity, FocusHandle, InteractiveElement, ParentElement, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, TestSupportExt, WeakEntity, Window, div,
};

use apinata_core::{ScriptTextKind, is_hidden_char, is_variation_selector};

use crate::approval_prompt::untrusted_text;
use crate::approvals::{Approvals, ScriptPrompt};
use crate::shell::Shell;
use crate::ui::dialog_footer;

/// `text` split into lines, each escaped as [`untrusted_text`] escapes but uncut: a tab
/// becomes four spaces and a line's trailing carriage return is dropped.
pub fn untrusted_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let mut shown = String::with_capacity(line.len());
            for c in line.chars().filter(|&c| !is_variation_selector(c)) {
                match c {
                    '\t' => shown.push_str("    "),
                    c if is_hidden_char(c) => shown.push('\u{fffd}'),
                    c => shown.push(c),
                }
            }
            shown
        })
        .collect()
}

/// Where a text comes from, in words.
fn kind_label(kind: &ScriptTextKind) -> String {
    match kind {
        ScriptTextKind::Script { phase, level } => {
            format!("{} script, {}", phase.as_str(), untrusted_text(level))
        }
        ScriptTextKind::Variable { name } => {
            format!("post-response variable {}", untrusted_text(name))
        }
        ScriptTextKind::AssertionLeft => "assertion".to_owned(),
        ScriptTextKind::AssertionRight => "assertion value".to_owned(),
        _ => "script".to_owned(),
    }
}

/// Opens the modal for `prompt` over `window`, naming its collection `name`. Only a click on
/// "Approve scripts" records anything; Enter, Escape, "Don't approve" and every other way
/// out dismiss the request.
pub fn open(
    shell: WeakEntity<Shell>,
    approvals: Entity<Approvals>,
    prompt: ScriptPrompt,
    name: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    let collection: SharedString = untrusted_text(&name).into();
    let path: SharedString = untrusted_text(&prompt.collection.display().to_string()).into();
    let request: SharedString = match prompt.request.as_str() {
        "" => "the whole collection".into(),
        request => untrusted_text(request).into(),
    };
    let texts: Vec<(SharedString, Vec<SharedString>)> = prompt
        .texts
        .iter()
        .map(|(item, text)| {
            let head = format!(
                "{} ({})",
                untrusted_text(&item.to_string()),
                kind_label(&text.kind)
            );
            let lines = untrusted_lines(&text.shown)
                .into_iter()
                .map(Into::into)
                .collect();
            (head.into(), lines)
        })
        .collect();
    let scroll = ScrollHandle::new();
    // The builder runs every frame, so each handler clones what it captures.
    window.open_dialog(cx, move |dialog, window, cx| {
        // The list scrolls in what the titled body leaves after four lines of header and
        // the footer's buttons, its top padding and its ring room.
        let room = crate::dialog_frame::titled_body_room(window)
            - window.rem_size() * 10.
            - crate::style::resolve(crate::style::DIALOG_FOOTER_GAP, window)
            - crate::style::FOCUS_RING_ROOM;
        let dialog = crate::dialog_frame::place(dialog, window);
        // The text list is the dialog's only tab stop, so Enter and Escape reach the dialog
        // and no key can press a button.
        let focus = window.use_keyed_state("script-approval-focus", cx, |_, cx| {
            cx.focus_handle().tab_stop(true)
        });
        let focus: FocusHandle = focus.read(cx).clone();
        let on_key = |shell: &WeakEntity<Shell>, approvals: &Entity<Approvals>, prompt: &ScriptPrompt| {
            let (shell, approvals, prompt) = (shell.clone(), approvals.clone(), prompt.clone());
            move |_: &_, _: &mut Window, cx: &mut App| {
                answer(&shell, &approvals, &prompt, false, cx);
                true
            }
        };
        let button = |id: &'static str, label: &'static str, approve: bool| {
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
                    let note = answer(&shell, &approvals, &prompt, approve, cx);
                    window.close_dialog(cx);
                    if let Some(note) = note {
                        window.push_notification(note, cx);
                    }
                })
        };
        let mono = cx.theme().mono_font_family.clone();
        let list = v_flex().gap_3().children(texts.iter().map(|(head, lines)| {
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_size(crate::style::TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .child(head.clone()),
                )
                .child(
                    v_flex()
                        .p_2()
                        .border_1()
                        .border_color(cx.theme().border)
                        .font_family(mono.clone())
                        .children(lines.iter().map(|line| div().child(line.clone()))),
                )
        }));
        dialog
            .title("Run these scripts?")
            .close_button(false)
            .overlay_closable(false)
            .on_ok(on_key(&shell, &approvals, &prompt))
            .on_cancel(on_key(&shell, &approvals, &prompt))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .id("script-approval-collection")
                            .test_support()
                            .child(format!("Collection: {collection}")),
                    )
                    .child(
                        div()
                            .id("script-approval-collection-path")
                            .test_support()
                            .whitespace_nowrap()
                            .truncate()
                            .text_size(crate::style::TEXT_SMALL)
                            .text_color(cx.theme().muted_foreground)
                            .child(path.clone()),
                    )
                    .child(
                        div()
                            .id("script-approval-request")
                            .test_support()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Requested by the agent for {request}")),
                    )
                    .child(
                        div()
                            .id("script-approval-untrusted-note")
                            .test_support()
                            .text_color(cx.theme().muted_foreground)
                            .child("Untrusted text from the collection files; read it before approving."),
                    )
                    .child(
                        div()
                            .relative()
                            .child(
                                div()
                                    .id("script-approval-text")
                                    .test_support()
                                    .track_focus(&focus)
                                    .max_h(room.max(window.rem_size() * 5.))
                                    .overflow_y_scroll()
                                    .track_scroll(&scroll)
                                    .child(list),
                            )
                            .vertical_scrollbar(&scroll),
                    ),
            )
            // Enter and Escape only dismiss, so the footer says so on the left.
            .footer(
                dialog_footer(
                    Some(button("script-approval-deny", "Don't approve", false)),
                    [],
                    button("script-approval-approve", "Approve scripts", true).primary(),
                )
                .hint(
                    div()
                        .id("script-approval-dismiss-hint")
                        .test_support()
                        .text_size(crate::style::TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .child("Esc: dismiss"),
                ),
            )
    });
}

/// Clears the shell's script prompt, then approves or dismisses. Returns the notification to
/// show, if any; the closing notify lets the shell open the next prompt.
fn answer(
    shell: &WeakEntity<Shell>,
    approvals: &Entity<Approvals>,
    prompt: &ScriptPrompt,
    approve: bool,
    cx: &mut App,
) -> Option<Notification> {
    let _ = shell.update(cx, |s, _| s.script_approval_answered());
    approvals.update(cx, |a, cx| {
        let note = if approve {
            a.approve_scripts(prompt, cx)
                .err()
                .map(|e| Notification::error(format!("Could not save the script approvals: {e}")))
        } else {
            a.dismiss_scripts(prompt, cx);
            None
        };
        cx.notify();
        note
    })
}
