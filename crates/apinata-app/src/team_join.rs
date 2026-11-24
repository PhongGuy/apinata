//! The join dialog: a server address, then a username and password or the server's single
//! sign-on in the browser; a kept session out. The password goes to the server once and is
//! never stored; the token lands in the keychain.

use crate::ui::SolidRing as _;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClipboardItem, Entity, Focusable, Global, Subscription, TestSupportExt, WeakEntity,
    Window, div,
};

use crate::app::AppState;
use crate::style::{SHORT_FORM_LABEL_WIDTH, SPACE_2};
use crate::team_text::{server_warning, sign_in_error_text};
use crate::ui::LockButton as _;
use crate::workspace::Workspace;
use crate::{tokio_bridge, ui};
use apinata_core::team::{
    self, Account, HttpTeamClient, SSO_TIMEOUT, ServerInfo, ServerUrl, TeamApi as _, TeamError,
    TeamStore,
};
use apinata_core::{CancelFlag, CancelHandle};

/// Which screen the dialog shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinPhase {
    /// Collecting the server, username and password.
    Form,
    /// The sign-in is in flight.
    Signing,
    /// Waiting for the browser to come back from single sign-on.
    Browser,
}

// The server's answer about itself, for the address it was asked at.
struct Checked {
    server: String,
    info: ServerInfo,
}

/// The dialog's body and state.
pub struct JoinView {
    workspace: Entity<Workspace>,
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    phase: JoinPhase,
    error: Option<String>,
    notice: Option<String>,
    // The last server answer; a newer check or an edited address drops it.
    checked: Option<Checked>,
    check_generation: u64,
    // The single sign-on start URL while the browser is out, and whether it opened there.
    sso_url: Option<String>,
    browser_opened: bool,
    // The running sign-in's flags; closing the dialog sets them.
    cancel: Option<CancelFlag>,
    sso_cancel: Option<CancelHandle>,
    _subscriptions: Vec<Subscription>,
}

// A dialog torn down mid-sign-in (its window closing) cancels like the Cancel button.
impl Drop for JoinView {
    fn drop(&mut self) {
        self.stop();
    }
}

// The most recently opened dialog's view; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<JoinView>);
impl Global for OpenView {}

/// The open join dialog's view, for tests and the pilot; None when no dialog is open.
pub fn current(cx: &App) -> Option<Entity<JoinView>> {
    cx.try_global::<OpenView>().and_then(|v| v.0.upgrade())
}

/// Opens the join dialog over `window` with empty fields; a finished join opens the
/// workspace list.
pub fn open(workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> Entity<JoinView> {
    open_for(workspace, "", "", window, cx)
}

/// `open` with the server and username filled in, for signing in again. A filled-in server is
/// asked at once what sign-in it offers, and a kept sign-in that expired is named.
pub fn open_for(
    workspace: &Entity<Workspace>,
    server: &str,
    username: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<JoinView> {
    let notice = expired_account(server, username, cx).then(|| "Your sign-in expired.".to_owned());
    let view = cx.new(|cx| {
        let server_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://team.example.com")
                .default_value(server.to_owned())
        });
        let username_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Username")
                .default_value(username.to_owned())
        });
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let typed = |this: &mut JoinView,
                     _: &Entity<InputState>,
                     e: &InputEvent,
                     window: &mut Window,
                     cx: &mut Context<JoinView>| match e {
            InputEvent::Change => {
                this.error = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.submit(window, cx),
            _ => {}
        };
        // The server is asked what it offers when the address is left or entered, never per key.
        let server_typed = |this: &mut JoinView,
                            _: &Entity<InputState>,
                            e: &InputEvent,
                            window: &mut Window,
                            cx: &mut Context<JoinView>| match e {
            InputEvent::Change => {
                this.error = None;
                this.forget_check();
                cx.notify();
            }
            InputEvent::Blur => this.check_server(window, cx),
            InputEvent::PressEnter { .. } => {
                this.check_server(window, cx);
                if this.password_offered() {
                    this.submit(window, cx);
                }
            }
            _ => {}
        };
        let subscriptions = vec![
            cx.subscribe_in(&server_input, window, server_typed),
            cx.subscribe_in(&username_input, window, typed),
            cx.subscribe_in(&password, window, typed),
        ];
        JoinView {
            workspace: workspace.clone(),
            server: server_input,
            username: username_input,
            password,
            phase: JoinPhase::Form,
            error: None,
            notice,
            checked: None,
            check_generation: 0,
            sso_url: None,
            browser_opened: false,
            cancel: None,
            sso_cancel: None,
            _subscriptions: subscriptions,
        }
    });
    cx.set_global(OpenView(view.downgrade()));
    let body = view.clone();
    crate::ui::open_dialog(window, cx, move |dialog, window, _| {
        // Enter never closes the dialog: the fields submit themselves. Escape cancels a sign-in.
        let view = body.clone();
        crate::dialog_frame::place(dialog, window)
            .title("Join team server")
            .close_button(false)
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, _, cx| {
                view.update(cx, |v, _| v.stop());
                true
            })
            .child(body.clone())
    });
    // A prefilled form only lacks the password.
    let first = if server.is_empty() {
        view.read(cx).server.clone()
    } else {
        view.update(cx, |v, cx| v.check_server(window, cx));
        view.read(cx).password.clone()
    };
    first.read(cx).focus_handle(cx).focus(window, cx);
    view
}

// Whether the kept account for `username` on `server` holds a sign-in whose expiry has passed.
fn expired_account(server: &str, username: &str, cx: &App) -> bool {
    let (Ok(url), Some(state)) = (ServerUrl::parse(server), cx.try_global::<AppState>()) else {
        return false;
    };
    TeamStore::load(&state.team)
        .ok()
        .and_then(|store| store.account(&url).cloned())
        .is_some_and(|a: Account| a.username == username && a.expired())
}

impl JoinView {
    /// Sets the server address field.
    pub fn set_server(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.server
            .update(cx, |i, cx| i.set_value(text, window, cx));
        self.forget_check();
        cx.notify();
    }

    /// Sets the username field.
    pub fn set_username(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.username
            .update(cx, |i, cx| i.set_value(text, window, cx));
        cx.notify();
    }

    /// Sets the password field.
    pub fn set_password(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.password
            .update(cx, |i, cx| i.set_value(text, window, cx));
        cx.notify();
    }

    /// The server address field's text.
    pub fn server(&self, cx: &App) -> String {
        self.server.read(cx).value().to_string()
    }

    /// The username field's text.
    pub fn username(&self, cx: &App) -> String {
        self.username.read(cx).value().to_string()
    }

    /// The current screen.
    pub fn phase(&self) -> JoinPhase {
        self.phase
    }

    /// The message under the form, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The note above the form, such as an expired sign-in; None when there is nothing to say.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// The single sign-on provider the server named for the address in the field, if any.
    pub fn sso_name(&self) -> Option<&str> {
        let provider = self.checked.as_ref()?.info.sso.as_ref()?;
        Some(provider.name.as_str())
    }

    /// The start URL shown for copying while the browser is out.
    pub fn sso_url(&self) -> Option<&str> {
        self.sso_url.as_deref()
    }

    /// Whether the form asks for a username and password: true until the server says it
    /// takes only single sign-on.
    pub fn password_offered(&self) -> bool {
        self.checked
            .as_ref()
            .is_none_or(|c| c.info.password_sign_in || c.info.sso.is_none())
    }

    /// The warning for an address that would send the password in clear, if it parses.
    pub fn warning(&self, cx: &App) -> Option<&'static str> {
        ServerUrl::parse(&self.server(cx))
            .ok()
            .and_then(|s| server_warning(&s))
    }

    // Drops the server's last answer, and any answer still on its way.
    fn forget_check(&mut self) {
        self.checked = None;
        self.check_generation += 1;
    }

    /// Asks the server in the field what sign-in it offers. Only the newest check's answer is
    /// kept, so a slow answer for an earlier address never shows its provider.
    pub fn check_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.server(cx);
        if self.checked.as_ref().is_some_and(|c| c.server == text) {
            return;
        }
        self.forget_check();
        cx.notify();
        let Ok(server) = ServerUrl::parse(&text) else {
            return;
        };
        let generation = self.check_generation;
        let task = tokio_bridge::spawn(cx, async move {
            HttpTeamClient::new(server)?.server_info().await
        });
        cx.spawn_in(window, async move |this, cx| {
            let answer = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.check_generation != generation {
                    return;
                }
                if let Ok(Ok(info)) = answer {
                    this.checked = Some(Checked { server: text, info });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // Cancels the running sign-in, if any; core then keeps nothing and revokes a new token.
    fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        if let Some(cancel) = self.sso_cancel.take() {
            cancel.cancel();
        }
    }

    // Shows `text` on the form, which stays open.
    fn fail(&mut self, text: String, cx: &mut Context<Self>) {
        self.phase = JoinPhase::Form;
        self.error = Some(text);
        self.sso_url = None;
        cx.notify();
    }

    /// Checks the fields in order and signs in; a problem shows as the error on the form.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase != JoinPhase::Form {
            return;
        }
        let server = match ServerUrl::parse(&self.server(cx)) {
            Ok(server) => server,
            Err(e) => return self.fail(sign_in_error_text(&e), cx),
        };
        let username = self.username(cx).trim().to_owned();
        if username.is_empty() {
            return self.fail("Enter your username.".into(), cx);
        }
        let password = self.password.read(cx).value().to_string();
        if password.is_empty() {
            return self.fail("Enter your password.".into(), cx);
        }
        let Some(state) = cx.try_global::<AppState>() else {
            return;
        };
        let (secrets, file) = (Arc::clone(&state.secrets), state.team.clone());
        self.phase = JoinPhase::Signing;
        self.error = None;
        cx.notify();
        // The password moves into the sign-in future and is dropped with it.
        let shown = server.to_string();
        let resumed = server.clone();
        let cancel = CancelFlag::new();
        self.cancel = Some(cancel.clone());
        let watched = cancel.clone();
        let task = tokio_bridge::spawn(cx, async move {
            let api = HttpTeamClient::new(server)?;
            team::sign_in(&api, &*secrets, &file, &username, &password, &watched).await
        });
        let workspace = self.workspace.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let cancelled = cancel.is_cancelled();
            let _ = cx.update(|window, cx| {
                signed_in(
                    result, cancelled, &shown, &resumed, &workspace, &this, window, cx,
                )
            });
        })
        .detach();
    }

    /// Starts single sign-on: core holds a loopback port, the start URL opens in the browser and
    /// shows for copying, and the dialog waits for the browser to come back.
    pub fn sign_in_sso(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase != JoinPhase::Form || self.sso_name().is_none() {
            return;
        }
        let server = match ServerUrl::parse(&self.server(cx)) {
            Ok(server) => server,
            Err(e) => return self.fail(sign_in_error_text(&e), cx),
        };
        let Some(state) = cx.try_global::<AppState>() else {
            return;
        };
        let (secrets, file) = (Arc::clone(&state.secrets), state.team.clone());
        self.phase = JoinPhase::Browser;
        self.error = None;
        self.sso_url = None;
        cx.notify();
        let shown = server.to_string();
        let resumed = server.clone();
        let cancel = CancelHandle::new();
        self.sso_cancel = Some(cancel.clone());
        let watched = cancel.clone();
        // The pending sign-in stays on tokio; only its start URL crosses to the window.
        let (url_tx, url_rx) = async_channel::bounded::<String>(1);
        let task = tokio_bridge::spawn(cx, async move {
            let api = HttpTeamClient::new(server)?;
            let pending = team::begin_sso(&api).await?;
            let _ = url_tx.send(pending.url().to_owned()).await;
            drop(url_tx);
            pending
                .finish(&api, &*secrets, &file, SSO_TIMEOUT, &watched)
                .await
        });
        let workspace = self.workspace.clone();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(url) = url_rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.browser_opened = crate::oauth2_signin::open_in_browser(&url, cx);
                    this.sso_url = Some(url);
                    cx.notify();
                });
            }
            let result = task.await;
            let cancelled = cancel.is_cancelled();
            let _ = cx.update(|window, cx| {
                signed_in(
                    result, cancelled, &shown, &resumed, &workspace, &this, window, cx,
                )
            });
        })
        .detach();
    }
}

// Finishes either sign-in. The result is handled even after the dialog is gone: a cancel that
// came too late to stop the sign-in still leaves the user signed in, and the toast says so.
#[allow(clippy::too_many_arguments)] // Each is a distinct piece of the finished sign-in.
fn signed_in(
    result: Result<Result<Account, TeamError>, tokio_bridge::JoinError>,
    cancelled: bool,
    shown: &str,
    resumed: &ServerUrl,
    workspace: &Entity<Workspace>,
    this: &WeakEntity<JoinView>,
    window: &mut Window,
    cx: &mut App,
) {
    match result {
        Ok(Ok(account)) => {
            if !cancelled {
                window.close_dialog(cx);
            }
            let text = format!("Signed in to {shown} as {}.", account.username);
            window.push_notification(ui::info_toast(text), cx);
            crate::team_mode::resume_server(workspace, resumed, cx);
            if !cancelled {
                crate::team_workspaces::open(workspace, window, cx);
            }
        }
        _ if cancelled => {}
        Ok(Err(e)) => {
            let _ = this.update(cx, |this, cx| this.fail(sign_in_error_text(&e), cx));
        }
        Err(_) => {
            let text = sign_in_error_text(&TeamError::Unavailable(String::new()));
            let _ = this.update(cx, |this, cx| this.fail(text, cx));
        }
    }
}

impl Render for JoinView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let signing = self.phase != JoinPhase::Form;
        let field = |id: &'static str, label: &'static str, input: &Entity<InputState>| {
            ui::form_row(
                format!("{id}-row"),
                label,
                ui::padded_input(
                    Input::new(input)
                        .id(id)
                        .w_full()
                        .disabled(signing)
                        .bg(ui::field_fill(signing, cx)),
                    input,
                ),
            )
            .label_width(SHORT_FORM_LABEL_WIDTH)
        };
        let muted = cx.theme().muted_foreground;
        let notice = self.notice.clone().map(|text| {
            div()
                .id("team-join-notice")
                .test_support()
                .text_color(cx.theme().warning)
                .child(text)
        });
        let warning = self.warning(cx).map(|text| {
            div()
                .id("team-join-warning")
                .test_support()
                .text_color(cx.theme().warning)
                .child(text)
        });
        let error = self.error.clone().map(|message| {
            div()
                .id("team-join-error")
                .test_support()
                .text_color(cx.theme().danger)
                .child(message)
        });
        let password = self.password_offered();
        let sso_label = self.sso_name().map(|name| format!("Sign in with {name}"));
        let sso_button = |label: String, primary: bool| {
            let button = Button::new("team-join-sso").solid_ring(cx).label(label);
            let button = if primary {
                button.primary().lock_filled(signing, cx)
            } else {
                button.lock(signing, cx)
            };
            button.on_click(cx.listener(|this, _, window, cx| this.sign_in_sso(window, cx)))
        };
        // With a password form the provider is a second way in under it; alone it is the action.
        let sso_row = (password && self.phase != JoinPhase::Browser)
            .then(|| sso_label.clone().map(|label| sso_button(label, false)))
            .flatten();
        let browser = self.sso_url.clone().map(|url| {
            let copied = url.clone();
            let lead = if self.browser_opened {
                "Continue in your browser. If it did not open, copy this address into it:"
            } else {
                "Open this address in your browser to sign in:"
            };
            v_flex()
                .gap(SPACE_2)
                .child(div().text_color(muted).child(lead))
                .child(
                    h_flex()
                        .gap(SPACE_2)
                        .child(
                            div()
                                .id("team-join-sso-url")
                                .test_support()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(url),
                        )
                        .child(
                            Button::new("team-join-copy-url")
                                .solid_ring(cx)
                                .label("Copy URL")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                                }),
                        ),
                )
        });
        let primary = match (self.phase, password, sso_label) {
            (JoinPhase::Browser, _, _) => Button::new("team-join-waiting")
                .solid_ring(cx)
                .label("Waiting for browser…")
                .primary()
                .lock_filled(true, cx),
            (_, false, Some(label)) => sso_button(label, true),
            _ => Button::new("team-join-submit")
                .solid_ring(cx)
                .label(if signing { "Signing in…" } else { "Sign in" })
                .primary()
                .lock_filled(signing, cx)
                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
        };
        v_flex()
            .id("team-join-dialog")
            .test_support()
            .gap(SPACE_2)
            .children(notice)
            .child(field("team-join-server", "Server", &self.server))
            .children(warning)
            .when(password, |this| {
                this.child(field("team-join-username", "Username", &self.username))
                    .child(field("team-join-password", "Password", &self.password))
            })
            .children(sso_row)
            .children(browser)
            .children(error)
            .child(ui::dialog_footer(
                Some(
                    Button::new("team-join-cancel")
                        .solid_ring(cx)
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.stop();
                            window.close_dialog(cx);
                        })),
                ),
                [],
                primary,
            ))
    }
}
