//! The join dialog: a server address, a username and a password in; a kept session out.
//! The password goes to the server once and is never stored; the token lands in the keychain.

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Entity, Focusable, Global, Subscription, TestSupportExt, WeakEntity, Window, div,
};

use crate::app::AppState;
use crate::style::{SHORT_FORM_LABEL_WIDTH, SPACE_2};
use crate::team_text::{server_warning, sign_in_error_text};
use crate::ui::LockButton as _;
use crate::workspace::Workspace;
use crate::{tokio_bridge, ui};
use apinata_core::team::{self, HttpTeamClient, ServerUrl, TeamError};

/// Which screen the dialog shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinPhase {
    /// Collecting the server, username and password.
    Form,
    /// The sign-in is in flight.
    Signing,
}

/// The dialog's body and state.
pub struct JoinView {
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    phase: JoinPhase,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

// The most recently opened dialog's view; it lapses once the dialog lets go of it.
struct OpenView(WeakEntity<JoinView>);
impl Global for OpenView {}

/// The open join dialog's view, for tests and the pilot; None when no dialog is open.
pub fn current(cx: &App) -> Option<Entity<JoinView>> {
    cx.try_global::<OpenView>().and_then(|v| v.0.upgrade())
}

/// Opens the join dialog over `window` with empty fields.
pub fn open(workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> Entity<JoinView> {
    open_for(workspace, "", "", window, cx)
}

/// `open` with the server and username filled in, for signing in again.
pub fn open_for(
    _workspace: &Entity<Workspace>,
    server: &str,
    username: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<JoinView> {
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
        let subscriptions = vec![
            cx.subscribe_in(&server_input, window, typed),
            cx.subscribe_in(&username_input, window, typed),
            cx.subscribe_in(&password, window, typed),
        ];
        JoinView {
            server: server_input,
            username: username_input,
            password,
            phase: JoinPhase::Form,
            error: None,
            _subscriptions: subscriptions,
        }
    });
    cx.set_global(OpenView(view.downgrade()));
    let body = view.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        // Enter never closes the dialog: the fields submit themselves.
        crate::dialog_frame::place(dialog, window)
            .title("Join team server")
            .close_button(false)
            .on_ok(|_, _, _| false)
            .child(body.clone())
    });
    // A prefilled form only lacks the password.
    let first = if server.is_empty() {
        view.read(cx).server.clone()
    } else {
        view.read(cx).password.clone()
    };
    first.read(cx).focus_handle(cx).focus(window, cx);
    view
}

impl JoinView {
    /// Sets the server address field.
    pub fn set_server(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.server
            .update(cx, |i, cx| i.set_value(text, window, cx));
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

    /// The warning for an address that would send the password in clear, if it parses.
    pub fn warning(&self, cx: &App) -> Option<&'static str> {
        ServerUrl::parse(&self.server(cx))
            .ok()
            .and_then(|s| server_warning(&s))
    }

    // Shows `text` on the form, which stays open.
    fn fail(&mut self, text: String, cx: &mut Context<Self>) {
        self.phase = JoinPhase::Form;
        self.error = Some(text);
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
        let task = tokio_bridge::spawn(cx, async move {
            let api = HttpTeamClient::new(server)?;
            team::sign_in(&api, &*secrets, &file, &username, &password).await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(Ok(account)) => {
                    window.close_dialog(cx);
                    let text = format!("Signed in to {shown} as {}.", account.username);
                    window.push_notification(ui::info_toast(text), cx);
                }
                Ok(Err(e)) => this.fail(sign_in_error_text(&e), cx),
                Err(_) => this.fail(sign_in_error_text(&TeamError::Unavailable), cx),
            });
        })
        .detach();
    }
}

impl Render for JoinView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let signing = self.phase == JoinPhase::Signing;
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
        v_flex()
            .id("team-join-dialog")
            .test_support()
            .gap(SPACE_2)
            .child(field("team-join-server", "Server", &self.server))
            .children(warning)
            .child(field("team-join-username", "Username", &self.username))
            .child(field("team-join-password", "Password", &self.password))
            .children(error)
            .child(ui::dialog_footer(
                Some(
                    Button::new("team-join-cancel")
                        .label("Cancel")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
                [],
                Button::new("team-join-submit")
                    .label(if signing { "Signing in…" } else { "Sign in" })
                    .primary()
                    .lock_filled(signing, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            ))
    }
}
