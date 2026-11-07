//! The OAuth 2.0 browser sign-in: an `Authorizer` that core drives on tokio, bridged over a
//! channel to one dialog at a time on the shell window, plus the injectable URL opener.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use apinata_core::{
    AuthorizationPrompt, AuthorizeCancelled, AuthorizeOptions, Authorizer, check_redirect,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, ClipboardItem, Entity, Global, Result, SharedString,
    TestSupportExt, Window, div,
};

use crate::style::SPACE_3;

/// Opens a URL in the system browser; only `main.rs` installs `cx.open_url`.
pub type Opener = Arc<dyn Fn(&str, &mut App)>;

/// What the open sign-in dialog shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Showing {
    /// The authorization server's host.
    pub host: String,
    /// The full authorization URL.
    pub url: String,
    /// Whether the user pastes the redirect back (no loopback listener).
    pub paste: bool,
    /// Why the last pasted address was refused.
    pub error: Option<String>,
}

type Answer = Result<Option<String>, AuthorizeCancelled>;

enum Message {
    Open {
        id: u64,
        prompt: AuthorizationPrompt,
        reply: async_channel::Sender<Answer>,
    },
    Done(u64),
}

struct Entry {
    id: u64,
    showing: Showing,
    reply: async_channel::Sender<Answer>,
    paste: Option<Entity<InputState>>,
    /// The dialog is on screen; false while queued and once the user answered it.
    open: bool,
}

/// The sign-in state; observe it to learn when the sign-in dialog opens or closes.
#[derive(Default)]
pub struct SignIn {
    messages: Option<async_channel::Sender<Message>>,
    opener: Option<Opener>,
    next_id: Arc<AtomicU64>,
    /// Prompts in arrival order; the front one owns the dialog.
    queue: VecDeque<Entry>,
}

impl Global for SignIn {}

/// Route sign-in prompts to dialogs on `window`; `start` calls it for the shell window.
pub fn install(window: AnyWindowHandle, cx: &mut App) {
    let (tx, rx) = async_channel::unbounded();
    let state = cx.default_global::<SignIn>();
    state.messages = Some(tx);
    state.queue.clear();
    // The loop ends with the window or when a later install replaces the sender.
    cx.spawn(async move |cx| {
        while let Ok(message) = rx.recv().await {
            if window
                .update(cx, |_, window, cx| handle(message, window, cx))
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

/// Sets the browser opener used when a sign-in dialog appears.
pub fn set_opener(opener: Opener, cx: &mut App) {
    cx.default_global::<SignIn>().opener = Some(opener);
}

/// `AuthorizeOptions` carrying the app authorizer once `install` ran, plain otherwise.
pub fn authorize_options(cx: &App) -> AuthorizeOptions {
    let options = AuthorizeOptions::new();
    match cx.try_global::<SignIn>() {
        Some(SignIn {
            messages: Some(messages),
            next_id,
            ..
        }) => options.authorizer(Arc::new(AppAuthorizer {
            messages: messages.clone(),
            next_id: Arc::clone(next_id),
        })),
        _ => options,
    }
}

/// The open sign-in dialog, if any; none once the user answered it.
pub fn showing(cx: &App) -> Option<Showing> {
    let front = cx.try_global::<SignIn>()?.queue.front()?;
    front.open.then(|| front.showing.clone())
}

struct AppAuthorizer {
    messages: async_channel::Sender<Message>,
    next_id: Arc<AtomicU64>,
}

/// Tells the dialog loop that core stopped waiting on prompt `id`, however the sign-in ended.
struct DoneGuard {
    id: u64,
    messages: async_channel::Sender<Message>,
}

impl Drop for DoneGuard {
    fn drop(&mut self) {
        let _ = self.messages.try_send(Message::Done(self.id));
    }
}

impl Authorizer for AppAuthorizer {
    fn authorize(
        &self,
        prompt: AuthorizationPrompt,
    ) -> Pin<Box<dyn Future<Output = Answer> + Send>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let messages = self.messages.clone();
        Box::pin(async move {
            let _done = DoneGuard {
                id,
                messages: messages.clone(),
            };
            let (reply, answer) = async_channel::bounded(1);
            let open = Message::Open { id, prompt, reply };
            if messages.send(open).await.is_err() {
                return Err(AuthorizeCancelled);
            }
            // A dialog that went away unanswered (window closed) counts as a cancel.
            answer.recv().await.unwrap_or(Err(AuthorizeCancelled))
        })
    }
}

fn handle(message: Message, window: &mut Window, cx: &mut App) {
    match message {
        // A caller that gave up before its prompt arrived needs no dialog and no browser.
        Message::Open { reply, .. } if reply.is_closed() => {}
        Message::Open { id, prompt, reply } => {
            let host = url::Url::parse(&prompt.url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default();
            let paste = prompt
                .wants_redirect
                .then(|| cx.new(|cx| InputState::new(window, cx)));
            let entry = Entry {
                id,
                showing: Showing {
                    host,
                    url: prompt.url,
                    paste: prompt.wants_redirect,
                    error: None,
                },
                reply,
                paste,
                open: false,
            };
            let queue = &mut cx.global_mut::<SignIn>().queue;
            queue.push_back(entry);
            if queue.len() == 1 {
                show_front(window, cx);
            }
        }
        Message::Done(id) => {
            let queue = &mut cx.global_mut::<SignIn>().queue;
            if queue.front().is_some_and(|e| e.id == id) {
                let front = queue.pop_front().expect("the front entry");
                if front.open {
                    window.close_dialog(cx);
                }
                if !cx.global::<SignIn>().queue.is_empty() {
                    show_front(window, cx);
                }
            } else {
                // A queued prompt whose caller gave up never gets a dialog.
                queue.retain(|e| e.id != id);
            }
        }
    }
}

fn show_front(window: &mut Window, cx: &mut App) {
    let state = cx.global_mut::<SignIn>();
    let Some(front) = state.queue.front_mut() else {
        return;
    };
    front.open = true;
    let (id, url, paste) = (front.id, front.showing.url.clone(), front.paste.clone());
    let opener = state.opener.clone();
    open_dialog(id, paste.clone(), window, cx);
    if let Some(input) = paste {
        input.update(cx, |i, cx| i.focus(window, cx));
    }
    if let Some(opener) = opener {
        opener(&url, cx);
    }
}

/// Sends `answer` for prompt `id` and marks its dialog closed; the caller closes the dialog.
fn answer(id: u64, answer: Answer, cx: &mut App) {
    if let Some(entry) = (cx.global_mut::<SignIn>().queue.iter_mut()).find(|e| e.id == id) {
        let _ = entry.reply.try_send(answer);
        entry.open = false;
    }
}

/// Checks the pasted address with core's parser: a refusal stays in the dialog, an accepted
/// one answers core. Returns whether the dialog may close.
fn submit(id: u64, cx: &mut App) -> bool {
    let state = cx.global::<SignIn>();
    let Some(input) = (state.queue.iter().find(|e| e.id == id)).and_then(|e| e.paste.clone())
    else {
        return false;
    };
    let pasted = input.read(cx).value().trim().to_owned();
    match check_redirect(&pasted) {
        Ok(()) => {
            answer(id, Ok(Some(pasted)), cx);
            true
        }
        Err(e) => {
            if let Some(entry) = (cx.global_mut::<SignIn>().queue.iter_mut()).find(|e| e.id == id) {
                entry.showing.error = Some(e.to_string());
            }
            false
        }
    }
}

fn open_dialog(id: u64, paste: Option<Entity<InputState>>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let dialog = crate::dialog_frame::place(dialog, window);
        let Some(shown) = (cx.global::<SignIn>().queue.iter())
            .find(|e| e.id == id)
            .map(|e| e.showing.clone())
        else {
            return dialog;
        };
        let text = if shown.paste {
            format!(
                "Sign in at {}, then paste the address your browser was sent to.",
                shown.host
            )
        } else {
            format!("Continue in your browser at {}.", shown.host)
        };
        let muted = cx.theme().muted_foreground;
        let url = SharedString::from(shown.url.clone());
        let copy = Button::new("oauth2-signin-copy")
            .label("Copy URL")
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(shown.url.clone()))
            });
        let cancel = Button::new("oauth2-signin-cancel")
            .label("Cancel")
            .on_click(move |_, window, cx| {
                answer(id, Err(AuthorizeCancelled), cx);
                window.close_dialog(cx);
            });
        let cont = paste.is_some().then(|| {
            Button::new("oauth2-signin-continue")
                .label("Continue")
                .primary()
                .on_click(move |_, window, cx| {
                    if submit(id, cx) {
                        window.close_dialog(cx);
                    }
                })
        });
        let error = shown.error.map(|e| {
            div()
                .id("oauth2-signin-error")
                .test_support()
                .text_color(cx.theme().danger)
                .child(e)
        });
        dialog
            .title("Sign in")
            .close_button(false)
            .overlay_closable(false)
            .on_ok(move |_, _, cx| submit(id, cx))
            .on_cancel(move |_, _, cx| {
                answer(id, Err(AuthorizeCancelled), cx);
                true
            })
            .child(
                v_flex()
                    .id("oauth2-signin")
                    .test_support()
                    .gap(SPACE_3)
                    .child(div().child(text))
                    // A long query would wrap into a wall of text; Copy URL has it whole.
                    .child(
                        div()
                            .id("oauth2-signin-url")
                            .test_support()
                            .aria_label(url.clone())
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .text_color(muted)
                            .child(url),
                    )
                    .when_some(paste.clone(), |el, input| {
                        el.child(crate::ui::padded_input(
                            Input::new(&input)
                                .id("oauth2-signin-paste")
                                .w_full()
                                .bg(crate::ui::field_fill(false, cx)),
                            &input,
                        ))
                    })
                    .children(error)
                    .child(
                        h_flex()
                            .key_context(crate::ui::DIALOG_FOOTER_CONTEXT)
                            .gap_2()
                            .justify_end()
                            .child(copy)
                            .child(cancel)
                            .children(cont),
                    ),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::{
        Answer, AuthorizationPrompt, AuthorizeCancelled, Message, SignIn, install, set_opener,
        showing,
    };
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, AppContext as _, Context, IntoElement, Render, TestAppContext, Window,
        WindowOptions, div,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    // A window with the dialog loop installed and an opener that counts its calls.
    fn window(cx: &mut TestAppContext) -> (AnyWindowHandle, Arc<AtomicU64>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            cx.set_reduce_motion(true);
        });
        let (win, _) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank))
            })
            .unwrap();
        let opened = Arc::new(AtomicU64::new(0));
        let seen = Arc::clone(&opened);
        cx.update(|cx| {
            install(win, cx);
            set_opener(
                Arc::new(move |_, _| {
                    seen.fetch_add(1, Ordering::Relaxed);
                }),
                cx,
            );
        });
        (win, opened)
    }

    fn open(cx: &mut TestAppContext, id: u64, reply: async_channel::Sender<Answer>) {
        let messages = cx.update(|cx| cx.global::<SignIn>().messages.clone().unwrap());
        let prompt = AuthorizationPrompt::new(
            "http://127.0.0.1:9/authorize".into(),
            false,
            "http://127.0.0.1:9/cb".into(),
        );
        messages
            .try_send(Message::Open { id, prompt, reply })
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn a_prompt_abandoned_before_it_shows_opens_nothing(cx: &mut TestAppContext) {
        let (win, opened) = window(cx);
        let (reply, answer) = async_channel::bounded(1);
        drop(answer);
        open(cx, 7, reply);
        assert_eq!(opened.load(Ordering::Relaxed), 0);
        assert_eq!(cx.update(|cx| showing(cx)), None);
        assert!(
            !cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
                .unwrap()
        );
    }

    #[gpui_kit::test]
    fn cancel_ends_showing_before_core_lets_go(cx: &mut TestAppContext) {
        let (win, opened) = window(cx);
        let (reply, answer) = async_channel::bounded(1);
        open(cx, 7, reply);
        assert_eq!(opened.load(Ordering::Relaxed), 1);
        assert!(cx.update(|cx| showing(cx)).is_some());
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.click("oauth2-signin-cancel", cx)
        })
        .unwrap();
        cx.run_until_parked();
        // No Done has arrived: the prompt's caller still holds `answer`.
        assert_eq!(answer.try_recv(), Ok(Err(AuthorizeCancelled)));
        assert_eq!(cx.update(|cx| showing(cx)), None);
    }
}
