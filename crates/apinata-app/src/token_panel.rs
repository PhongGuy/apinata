//! The token panel under the OAuth 2.0 editor: the cached token's status and scopes, with
//! Get token, Refresh and Clear for the auth a request, a folder or the collection uses.

use crate::ui::LockButton as _;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use apinata_core::{
    AuthorizeOptions, CancelHandle, ItemPath, OAuth2Error, OAuth2Target, Persistence, RequestFile,
    RunPlan, TokenStatus, VarContext, clear_oauth2_token, oauth2_status,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Result, Task, TestSupportExt, Window, div};

use crate::command::{Command, Target};
use crate::send_model::plan_message;
use crate::workspace::Workspace;
use crate::{executor, oauth2_tokens, settings, style, tokio_bridge, ui};

/// Where the panel's auth lives, read on every refresh (requests can be renamed).
pub type Locate = Box<dyn Fn(&App) -> Option<(PathBuf, Target)>>;

/// Run before Get token: the labels of the required fields left blank, which it marks.
pub type Check = Box<dyn Fn(&mut App) -> Vec<&'static str>>;

/// The neutral line a sign-in the user cancelled leaves.
pub const CANCELLED: &str = "Sign-in cancelled";

// How often the status is read again, for the expiry countdown and environment switches.
const REREAD: Duration = Duration::from_secs(15);

enum Action {
    Get(AuthorizeOptions),
    Refresh,
    Clear,
}

// What the panel's auth is, resolved from the plan of its item.
enum Planned {
    Request(Box<RequestFile>),
    Folder(ItemPath),
    Collection,
}

impl Planned {
    fn target(&self) -> OAuth2Target<'_> {
        match self {
            Planned::Request(req) => OAuth2Target::Request(req),
            Planned::Folder(item) => OAuth2Target::Folder(item),
            Planned::Collection => OAuth2Target::Collection,
        }
    }
}

// How an action ended short: an error, or a sign-in the user cancelled, which is none.
enum Stopped {
    Failed(String),
    Cancelled,
}

// An action's stop, if it had one, and the status read after it.
type Outcome = (Option<Stopped>, Result<Option<TokenStatus>, String>);

/// Shows the cached token of one auth and offers Get token, Refresh and Clear.
pub struct TokenPanel {
    workspace: Entity<Workspace>,
    locate: Locate,
    status: Option<TokenStatus>,
    action_error: Option<String>,
    action_note: Option<&'static str>,
    check: Option<Check>,
    read_error: Option<String>,
    memory_only: Option<String>,
    // A read can be replaced at any time; an action is never cut short by a read.
    read: Option<Task<()>>,
    action: Option<Task<()>>,
    reads_started: usize,
    _ticker: Task<()>,
}

impl TokenPanel {
    /// A panel for the auth `locate` finds; it reads the status at once and every 15 s.
    pub fn new(workspace: Entity<Workspace>, locate: Locate, cx: &mut Context<Self>) -> Self {
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REREAD).await;
                if this.update(cx, |p, cx| p.reread(cx)).is_err() {
                    break;
                }
            }
        });
        let mut panel = TokenPanel {
            workspace,
            locate,
            status: None,
            action_error: None,
            action_note: None,
            check: None,
            read_error: None,
            memory_only: None,
            read: None,
            action: None,
            reads_started: 0,
            _ticker: ticker,
        };
        panel.reread(cx);
        panel
    }

    /// Re-plan the target and re-read its status from the shared cache (loaded first).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.action.is_some() {
            return;
        }
        self.action_error = None;
        self.action_note = None;
        self.reread(cx);
    }

    /// Sets the check Get token runs first; blank required fields stop it with one message.
    pub fn set_check(&mut self, check: Check) {
        self.check = Some(check);
    }

    /// Fetches a token, signing in through the browser for interactive grants.
    pub fn get_token(&mut self, cx: &mut Context<Self>) {
        if self.action.is_some() {
            return;
        }
        // Only an auth the panel acts on is checked; otherwise the action says why it cannot run.
        let missing = match &self.check {
            Some(check) if (self.locate)(cx).is_some() => check(cx),
            _ => Vec::new(),
        };
        if !missing.is_empty() {
            self.action_error = Some(missing_message(&missing));
            self.action_note = None;
            return cx.notify();
        }
        let how = crate::oauth2_signin::authorize_options(cx);
        self.act(Action::Get(how), cx);
    }

    /// Exchanges the held refresh token for a new token.
    pub fn refresh_token(&mut self, cx: &mut Context<Self>) {
        self.act(Action::Refresh, cx);
    }

    /// Forgets the auth's cached tokens.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.act(Action::Clear, cx);
    }

    /// `status_line` of the last read status.
    pub fn status_text(&self) -> String {
        status_line(self.status.as_ref(), SystemTime::now())
    }

    /// "Scopes granted: <scope>" when the provider said which.
    pub fn scope_text(&self) -> Option<String> {
        let scope = self.status.as_ref()?.scope.as_deref()?;
        Some(format!("Scopes granted: {scope}"))
    }

    /// The last action's error, else its neutral note, else the cache's MemoryOnly reason.
    pub fn notice(&self) -> Option<String> {
        self.error()
            .or_else(|| self.action_note.map(str::to_owned))
            .or_else(|| self.memory_only.clone())
    }

    /// The last action's error, else the last status read's error.
    pub fn error(&self) -> Option<String> {
        self.action_error
            .clone()
            .or_else(|| self.read_error.clone())
    }

    /// Whether any read or action is in flight, including the read made at creation.
    pub fn is_busy(&self) -> bool {
        self.read.is_some() || self.action.is_some()
    }

    /// How many status reads (runs without an action) have started.
    pub fn reads_started(&self) -> usize {
        self.reads_started
    }

    // A read keeps the last action's error, so the timer does not wipe it unseen.
    fn reread(&mut self, cx: &mut Context<Self>) {
        if self.action.is_some() {
            return;
        }
        self.read = self.run(None, cx).ok();
        if self.read.is_some() {
            self.reads_started += 1;
        }
        cx.notify();
    }

    // Actions are refused while one runs; starting one drops any read, which has no effects.
    fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.action.is_some() {
            return;
        }
        self.read = None;
        self.action_error = None;
        self.action_note = None;
        let Some(exec) = executor::shared(cx) else {
            self.action_error = Some("the HTTP client is not ready".into());
            return cx.notify();
        };
        match self.run(Some((action, exec)), cx) {
            Ok(task) => self.action = Some(task),
            Err(why) => self.action_error = Some(why),
        }
        cx.notify();
    }

    // Plans the target on its collection actor, then on tokio loads the cache, runs the
    // action if any and reads the status; the result lands in the slot that started it.
    // `Err` says why nothing could start.
    fn run(
        &mut self,
        action: Option<(Action, std::sync::Arc<apinata_core::Executor>)>,
        cx: &mut Context<Self>,
    ) -> Result<Task<()>, String> {
        if !tokio_bridge::is_running(cx) {
            return Err("the HTTP client is not ready".into());
        }
        let is_action = action.is_some();
        let (root, target) = (self.locate)(cx).ok_or("no OAuth 2.0 auth to act on")?;
        let item = match &target {
            Target::Request(item) | Target::Folder(item) => item.clone(),
            Target::Collection => ItemPath::root(),
            Target::Environment(_) => return Err("environments have no token".into()),
        };
        let (reply, answer) = async_channel::bounded(1);
        let plan = Command::Plan { item, reply };
        self.workspace
            .update(cx, |w, cx| w.send(&root, plan, cx))
            .ok_or("the collection was closed")?;
        let cache = oauth2_tokens::shared(cx);
        Ok(cx.spawn(async move |this, cx| {
            let planned = match answer.recv().await {
                Ok(Ok(planned)) => Ok(planned),
                Ok(Err(e)) => Err(plan_message(&e)),
                Err(_) => Err("the collection was closed".to_owned()),
            };
            // Requests that inherit their timeout take the Preferences default.
            let planned = match planned {
                Ok(p) => {
                    let ctx = this
                        .update(cx, |_, cx| settings::with_request_timeout(&p.context, cx))
                        .unwrap_or_else(|_| p.context.clone());
                    planned_target(target, &p.plan).map(|which| (which, ctx))
                }
                Err(e) => Err(e),
            };
            let outcome = match planned {
                Err(e) => (None, Err(e)),
                Ok((which, ctx)) => {
                    let load = cache.clone();
                    let work = async move {
                        load.load().await;
                        perform(&which, &ctx, action).await
                    };
                    let task = cx.update(|cx| tokio_bridge::spawn(cx, work));
                    task.await.unwrap_or_else(|e| (None, Err(e.to_string())))
                }
            };
            let _ = this.update(cx, |p, cx| {
                p.apply(outcome, is_action, &cache.persistence());
                if is_action {
                    p.action = None;
                } else {
                    p.read = None;
                }
                cx.notify();
            });
        }))
    }

    fn apply(&mut self, (acted, status): Outcome, is_action: bool, persistence: &Persistence) {
        if is_action {
            (self.action_error, self.action_note) = match acted {
                Some(Stopped::Failed(e)) => (Some(e), None),
                Some(Stopped::Cancelled) => (None, Some(CANCELLED)),
                None => (None, None),
            };
        }
        match status {
            Ok(status) => {
                self.status = status;
                self.read_error = None;
            }
            Err(e) => {
                self.status = None;
                self.read_error = Some(e);
            }
        }
        self.memory_only = match persistence {
            Persistence::MemoryOnly { reason } => Some(reason.clone()),
            _ => None,
        };
    }
}

// A request's auth is read from its planned, draft-aware file; a folder's or the
// collection's from a probe core places there.
fn planned_target(target: Target, plan: &RunPlan) -> Result<Planned, String> {
    Ok(match target {
        Target::Folder(item) => Planned::Folder(item),
        Target::Collection => Planned::Collection,
        _ => {
            let req = plan.requests().next().ok_or("the request was not found")?;
            Planned::Request(Box::new(req.clone()))
        }
    })
}

async fn perform(
    which: &Planned,
    ctx: &VarContext,
    action: Option<(Action, std::sync::Arc<apinata_core::Executor>)>,
) -> Outcome {
    let target = which.target();
    let cancel = CancelHandle::new();
    let acted = match action {
        None => Ok(()),
        Some((Action::Get(how), exec)) => {
            exec.fetch_token(target, ctx, &how, &cancel).await.map(drop)
        }
        Some((Action::Refresh, exec)) => exec.refresh_token(target, ctx, &cancel).await.map(drop),
        Some((Action::Clear, _)) => clear_oauth2_token(target, ctx).await.map(drop),
    };
    let status = oauth2_status(target, ctx).map_err(|e| e.to_string());
    let stopped = acted.err().map(|e| match e {
        OAuth2Error::Cancelled => Stopped::Cancelled,
        e => Stopped::Failed(e.to_string()),
    });
    (stopped, status)
}

// "A is empty", "A and B are empty" or "A, B and C are empty".
fn missing_message(labels: &[&str]) -> String {
    match labels {
        [] => String::new(),
        [one] => format!("{one} is empty"),
        [rest @ .., last] => format!("{} and {last} are empty", rest.join(", ")),
    }
}

/// "No token", "Valid, expires in 54 min", "Valid, expires in 2 h 5 min",
/// "Valid, expires in less than a minute", "Valid, no expiry" or "Expired";
/// ", refresh token" appended when one is held.
pub fn status_line(status: Option<&TokenStatus>, now: SystemTime) -> String {
    let Some(status) = status else {
        return "No token".to_owned();
    };
    let mut line = if status.is_expired(now) {
        "Expired".to_owned()
    } else {
        match status.expires_at {
            None => "Valid, no expiry".to_owned(),
            Some(end) => {
                // Minutes round down, so the countdown never promises more than is left.
                let minutes = end.duration_since(now).unwrap_or_default().as_secs() / 60;
                match (minutes / 60, minutes % 60) {
                    (0, 0) => "Valid, expires in less than a minute".to_owned(),
                    (0, m) => format!("Valid, expires in {m} min"),
                    (h, 0) => format!("Valid, expires in {h} h"),
                    (h, m) => format!("Valid, expires in {h} h {m} min"),
                }
            }
        }
    };
    if status.has_refresh_token {
        line.push_str(", refresh token");
    }
    line
}

impl Render for TokenPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.is_busy();
        let can_refresh = self.status.as_ref().is_some_and(|s| s.has_refresh_token);
        let small = |id: &'static str, text: String| {
            div()
                .id(id)
                .test_support()
                .text_size(style::TEXT_SMALL)
                .child(text)
        };
        // A failure reads as an error; the memory-only reason is information and stays muted.
        let error = self.error().map(|text| {
            small("oauth2-token-error", text.clone())
                .aria_label(text)
                .text_color(cx.theme().danger)
        });
        let note = self.action_note.map(|text| {
            small("oauth2-token-note", text.to_owned())
                .aria_label(text)
                .text_color(cx.theme().muted_foreground)
        });
        let notice = self.memory_only.clone().map(|text| {
            small("oauth2-token-notice", text.clone())
                .aria_label(text)
                .text_color(cx.theme().muted_foreground)
        });
        // The title row sits on the form grid, so "Token" lines up with the field labels; the
        // status, scope and error lines grow below the buttons in the field column.
        v_flex().id("oauth2-token-panel").child(ui::form_row(
            "oauth2-token-row",
            "Token",
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap(style::SPACE_2)
                        .child(
                            Button::new("oauth2-get-token")
                                .label("Get token")
                                .lock(busy, cx)
                                .on_click(cx.listener(|p, _, _, cx| p.get_token(cx))),
                        )
                        .when(can_refresh, |row| {
                            row.child(
                                Button::new("oauth2-refresh-token")
                                    .label("Refresh")
                                    .lock(busy, cx)
                                    .on_click(cx.listener(|p, _, _, cx| p.refresh_token(cx))),
                            )
                        })
                        .child(
                            Button::new("oauth2-clear-token")
                                .label("Clear")
                                .lock(busy, cx)
                                .on_click(cx.listener(|p, _, _, cx| p.clear(cx))),
                        ),
                )
                .child(small("oauth2-token-status", self.status_text()))
                .children(
                    self.scope_text()
                        .map(|text| small("oauth2-token-scope", text)),
                )
                .children(error)
                .children(note)
                .children(notice),
        ))
    }
}
