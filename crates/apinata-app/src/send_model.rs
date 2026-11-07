//! Sending one request from a tab: plan it on the collection actor, run it on the tokio
//! bridge with the session's shared executor, and hold the outcome for the response pane.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use apinata_core::sse::SseEvent;
use apinata_core::{
    CancelHandle, DraftSnapshot, ExecutionMode, ItemPath, RequestFile, RequestResult, RunEvent,
    RunOptions, StreamLimits, VarContext, known_secrets,
};
use gpui_kit::{App, AsyncApp, Context, Entity, Global, Task, WeakEntity};

use crate::command::{Command, PlanError, Planned};
use crate::executor;
use crate::last_response::{self, KeptDigests, SendExtra, Stored};
use crate::live_stream::LiveStream;
use crate::prompt_vars::prompt_answers;
use crate::script_gate::{Prepared, prepare};
use crate::scrub;
use crate::settings;
use crate::tokio_bridge;
use crate::workspace::Workspace;

/// Where a tab's send stands.
#[derive(Clone)]
pub enum SendState {
    /// Nothing sent yet.
    Idle,
    /// Planning or in flight.
    Sending,
    /// The final hop's headers arrived and its body is open; the data is in `SendModel::live`.
    Streaming,
    /// Cancelled before the run started.
    Cancelled,
    /// No run happened: a plan error, a closed collection or no HTTP client.
    Failed(String),
    /// The run reported a result, whatever its outcome.
    Done(Arc<RequestResult>),
}

/// Where a shown result came from when it was kept from an earlier run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    /// When the kept response was saved.
    pub saved_at: SystemTime,
    /// Whether its body was too large to keep.
    pub body_dropped: bool,
    /// Whether a secret was scrubbed from its kept body.
    pub body_masked: bool,
    /// The places, among its headers, of the values a secret was scrubbed from.
    pub masked_headers: Vec<usize>,
    /// Keyed digests of those values' true text, when the send was kept with a key.
    pub header_digests: Option<KeptDigests>,
}

/// One tab's send: at most one at a time, cancellable while planning or in flight.
pub struct SendModel {
    workspace: Entity<Workspace>,
    state: SendState,
    cancel: Option<CancelHandle>,
    running: bool,
    // A prompt-variable or trust dialog is open for this send.
    waiting: bool,
    task: Option<Task<()>>,
    since: Option<Instant>,
    keep_as: Option<(PathBuf, ItemPath)>,
    // The secret values this send's request substitutes, scrubbed from what the store keeps,
    // and the request and context to look them up again once the run has fetched a token.
    secrets: Vec<String>,
    secrets_of: Option<(RequestFile, VarContext)>,
    // The scrub set of the send on screen, for comparing it with a kept one; empty when the
    // shown result was kept (already scrubbed).
    shown_secrets: Vec<String>,
    // A draft's request as planned, kept with its send once scrubbed.
    snapshot: Option<DraftSnapshot>,
    restored: Option<Restored>,
    // The latest (state, restored) while an older kept send is shown, and that send's seq.
    stash: Option<(SendState, Option<Restored>)>,
    viewing: Option<u64>,
    // When the latest result arrived, and the same for the latest while an older send is shown.
    finished_at: Option<SystemTime>,
    latest_finished_at: Option<SystemTime>,
    // Sticky: the tab has sent or restored a kept result, so its response pane stays open.
    ever_shown: bool,
    // The collection root of the latest send, for the error block's environment.
    root: Option<PathBuf>,
    // The open response body; `Some` only while Streaming.
    live: Option<LiveStream>,
    // The events the live view held when the stream ended, for a view until the final parse.
    ended_events: Vec<SseEvent>,
    // The run reported the end of the body, so Stop can no longer end just the read.
    body_ended: bool,
    // The seq the store kept a live result as, once it answers, and the wait for that answer.
    kept: Option<(std::sync::Weak<RequestResult>, last_response::KeptAs)>,
    kept_task: Option<Task<()>>,
}

impl SendModel {
    /// An idle model sending through the collections open in `workspace`.
    pub fn new(workspace: Entity<Workspace>) -> Self {
        SendModel {
            workspace,
            state: SendState::Idle,
            cancel: None,
            running: false,
            waiting: false,
            task: None,
            since: None,
            keep_as: None,
            secrets: Vec::new(),
            secrets_of: None,
            shown_secrets: Vec::new(),
            snapshot: None,
            restored: None,
            stash: None,
            viewing: None,
            finished_at: None,
            latest_finished_at: None,
            ever_shown: false,
            root: None,
            live: None,
            ended_events: Vec::new(),
            body_ended: false,
            kept: None,
            kept_task: None,
        }
    }

    /// The workspace this model sends through.
    pub fn workspace(&self) -> &Entity<Workspace> {
        &self.workspace
    }

    /// The collection root of the latest send, if any.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Back to idle when an error or cancellation is shown (`Failed`, `Cancelled`, or `Done`
    /// with no response); a received response, whatever its status, and a send in flight stay.
    pub fn clear_error(&mut self, cx: &mut Context<Self>) {
        let stale = |state: &SendState| match state {
            SendState::Failed(_) | SendState::Cancelled => true,
            SendState::Done(r) => r.response.is_none(),
            SendState::Idle | SendState::Sending | SendState::Streaming => false,
        };
        // While an older kept send is shown, the failure waits in the stash for Back to latest.
        if let Some((latest, restored)) = &mut self.stash
            && stale(latest)
        {
            *latest = SendState::Idle;
            *restored = None;
            cx.notify();
        }
        if stale(&self.state) {
            self.finish(SendState::Idle, cx);
        }
    }

    /// Drops the shown result, back to `Idle`, because the request it answered was replaced.
    /// A send in flight is left alone.
    pub fn clear_result(&mut self, cx: &mut Context<Self>) {
        if self.is_sending() {
            return;
        }
        self.leave_history();
        self.restored = None;
        self.shown_secrets.clear();
        self.finish(SendState::Idle, cx);
    }

    /// Whether this tab has sent or restored a kept result; once true it never goes back.
    pub fn ever_shown(&self) -> bool {
        self.ever_shown
    }

    /// The current state.
    pub fn state(&self) -> &SendState {
        &self.state
    }

    /// Whether a send is planning or in flight, its body still arriving or not.
    pub fn is_sending(&self) -> bool {
        matches!(self.state, SendState::Sending | SendState::Streaming)
    }

    /// Whether the response headers are in and the body is still open.
    pub fn is_streaming(&self) -> bool {
        matches!(self.state, SendState::Streaming)
    }

    /// Whether the run reported the end of the open response body (only while streaming).
    pub fn body_ended(&self) -> bool {
        self.body_ended
    }

    /// The events the live view held when the last stream ended; empty otherwise.
    pub fn ended_events(&self) -> &[SseEvent] {
        &self.ended_events
    }

    /// The live view of a streaming body, if one is open.
    pub fn live(&self) -> Option<&LiveStream> {
        self.live.as_ref()
    }

    /// Whether a prompt-variable or trust dialog is open for this send.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// How long the current send has been under way; `None` while not sending or before the
    /// plan is handed to the run.
    pub fn elapsed(&self) -> Option<Duration> {
        self.since.map(|t| t.elapsed())
    }

    /// Sends `item` of the collection at `root` as a one-request run of its current draft.
    /// Returns `false`, doing nothing, while a send is already under way; failures to start
    /// become `Failed` states.
    pub fn send(&mut self, root: &Path, item: ItemPath, cx: &mut Context<Self>) -> bool {
        let key = last_response::keeps(root, &item).then(|| (root.to_path_buf(), item.clone()));
        self.send_keyed(root, item, key, cx)
    }

    /// Sends `item` as `send` does, keeping a response under `key` (a collection root and the
    /// item path it is stored as); `None` keeps nothing.
    pub fn send_keyed(
        &mut self,
        root: &Path,
        item: ItemPath,
        key: Option<(PathBuf, ItemPath)>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.is_sending() {
            return false;
        }
        let shown = self.shown();
        self.ever_shown = true;
        self.root = Some(root.to_path_buf());
        let draft = key.as_ref().is_some_and(|(_, k)| k.is_draft());
        self.keep_as = key;
        self.secrets = Vec::new();
        self.secrets_of = None;
        self.snapshot = None;
        self.restored = None;
        self.leave_history();
        let Some(exec) = executor::shared(cx) else {
            self.finish(SendState::Failed("the HTTP client is not ready".into()), cx);
            return true;
        };
        let (reply, answer) = async_channel::bounded(1);
        let sent = self
            .workspace
            .update(cx, |w, cx| w.send(root, Command::Plan { item, reply }, cx));
        if sent.is_none() {
            self.finish(SendState::Failed("the collection is not open".into()), cx);
            return true;
        }
        let cancel = CancelHandle::new();
        self.cancel = Some(cancel.clone());
        self.state = SendState::Sending;
        self.since = None;
        cx.notify();
        let root = root.to_path_buf();
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut planned = match await_plan(answer).await {
                Ok(planned) => planned,
                Err(msg) => {
                    return drop(this.update(cx, |m, cx| m.finish(SendState::Failed(msg), cx)));
                }
            };
            // Prompt variables are asked for once, before the scripts gate; Cancel sends nothing.
            let labels = planned
                .plan
                .requests()
                .next()
                .map(|file| apinata_core::prompt_labels(file, &planned.context))
                .unwrap_or_default();
            if !labels.is_empty() {
                let memory = this
                    .update(cx, |m, cx| m.workspace.read(cx).prompt_memory(&root, cx))
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let _ = this.update(cx, |m, cx| m.wait(true, cx));
                let Some(answers) = prompt_answers(labels, memory, cx).await else {
                    return drop(this.update(cx, |m, cx| m.put_back(shown, cx)));
                };
                let _ = this.update(cx, |m, cx| m.wait(false, cx));
                planned.context = planned.context.with_prompt_values(answers);
            }
            // Requests that inherit their timeout take the Preferences default.
            if let Ok(context) = this.update(cx, |_, cx| {
                settings::with_request_timeout(&planned.context, cx)
            }) {
                planned.context = context;
            }
            // Captured before the plan moves into the run. A lookup that fails here, such as an
            // OAuth 2.0 send with no token yet, is repeated once the run is done.
            let source = planned
                .plan
                .requests()
                .next()
                .map(|file| (file.clone(), planned.context.clone()));
            let secrets = source
                .as_ref()
                .map(|(file, ctx)| secret_values(file, ctx))
                .unwrap_or_default();
            let snapshot = source
                .as_ref()
                .filter(|_| draft)
                .map(|(file, _)| file.snapshot());
            let _ = this.update(cx, |m, _| {
                m.secrets = secrets;
                m.secrets_of = source;
                m.snapshot = snapshot;
            });
            let scripts = match prepare(root, &planned, cx) {
                Prepared::Ready(scripts) => scripts,
                Prepared::Ask(question) => {
                    let _ = this.update(cx, |m, cx| m.wait(true, cx));
                    question.ask(cx).await
                }
            };
            let (tx, rx) = async_channel::unbounded::<RunEvent>();
            let on_event = Box::new(move |event: RunEvent| drop(tx.try_send(event)));
            // The timer starts here, as the plan is handed to the run.
            let _ = this.update(cx, |m, cx| {
                m.running = true;
                m.waiting = false;
                m.since = Some(Instant::now());
                cx.notify();
            });
            let run = cx.update(|cx| {
                let opts = RunOptions::new()
                    .authorize(crate::oauth2_signin::authorize_options(cx))
                    .execution_mode(ExecutionMode::Standalone)
                    .scripts(scripts)
                    .stream(StreamLimits::new());
                let run =
                    apinata_core::run(planned.plan, planned.context, exec, opts, cancel, on_event);
                tokio_bridge::spawn(cx, run)
            });
            drain(this, rx, cx).await;
            drop(run);
        }));
        true
    }

    /// Cancels the send. While planning it is dropped at once and shows `Cancelled`; in
    /// flight the run is cancelled and reports its own `Cancelled` outcome.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if !self.is_sending() {
            return;
        }
        if self.running {
            if let Some(cancel) = &self.cancel {
                cancel.cancel();
            }
        } else {
            self.task = None;
            self.finish(SendState::Cancelled, cx);
        }
    }

    /// Ends the open response body, keeping what arrived. Does nothing unless streaming. Once
    /// the run has reported the end of the body only post-response work is left, and Stop
    /// cancels it like any cancel.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.is_streaming() {
            return;
        }
        if self.body_ended {
            self.cancel(cx);
        } else if let Some(cancel) = &self.cancel {
            cancel.stop();
        }
    }

    /// Shows a run started elsewhere: `Sending` until its first request result, then each
    /// `RequestFinished` replaces the shown result, ending on the last; `Failed` if the
    /// channel closes without one. `cancel` has no effect on a followed run.
    pub fn follow(&mut self, events: async_channel::Receiver<RunEvent>, cx: &mut Context<Self>) {
        self.follow_with(events, None, cx);
    }

    /// Follows a run as `follow` does, with its `cancel` handle so `cancel` and `stop` reach it.
    pub fn follow_with(
        &mut self,
        events: async_channel::Receiver<RunEvent>,
        cancel: Option<CancelHandle>,
        cx: &mut Context<Self>,
    ) {
        self.cancel = cancel;
        self.ever_shown = true;
        self.state = SendState::Sending;
        self.since = Some(Instant::now());
        self.running = true;
        self.keep_as = None;
        self.restored = None;
        self.leave_history();
        cx.notify();
        self.task = Some(cx.spawn(async move |this, cx| drain(this, events, cx).await));
    }

    /// Shows `result` as `show` does, as a result of the collection at `root`, so the error
    /// block reads that collection's environment.
    pub fn show_at(&mut self, root: &Path, result: Arc<RequestResult>, cx: &mut Context<Self>) {
        self.root = Some(root.to_path_buf());
        self.show(result, cx);
    }

    /// Shows `result` as `Done`, dropping any send or followed run under way.
    pub fn show(&mut self, result: Arc<RequestResult>, cx: &mut Context<Self>) {
        self.ever_shown = true;
        self.task = None;
        self.keep_as = None;
        self.restored = None;
        self.shown_secrets.clear();
        self.leave_history();
        self.finish(SendState::Done(result), cx);
    }

    /// Shows a kept result as `Done` with its `Restored` facts; ignored unless the state is `Idle`.
    /// While an older send is shown over an `Idle` latest, it becomes the latest instead.
    pub fn restore(
        &mut self,
        result: Arc<RequestResult>,
        restored: Restored,
        cx: &mut Context<Self>,
    ) {
        if !self.ever_shown {
            self.ever_shown = true;
            cx.notify();
        }
        if let Some(stash) = &mut self.stash {
            if matches!(stash.0, SendState::Idle) {
                *stash = (SendState::Done(result), Some(restored));
            }
            return;
        }
        if !matches!(self.state, SendState::Idle) {
            return;
        }
        self.restored = Some(restored);
        self.shown_secrets.clear();
        self.track_restored(cx);
        self.finish(SendState::Done(result), cx);
    }

    /// Shows kept send `seq` read-only, keeping the latest to go back to; ignored while sending.
    pub fn view_kept(&mut self, seq: u64, stored: Stored, cx: &mut Context<Self>) {
        if self.is_sending() {
            return;
        }
        if self.stash.is_none() {
            self.stash = Some((self.state.clone(), self.restored.clone()));
            self.latest_finished_at = self.finished_at;
        }
        self.restored = Some(Restored {
            saved_at: stored.saved_at,
            body_dropped: stored.body_dropped,
            body_masked: stored.body_masked,
            masked_headers: stored.masked_headers,
            header_digests: stored.header_digests,
        });
        self.viewing = Some(seq);
        self.shown_secrets.clear();
        self.track_restored(cx);
        self.finish(SendState::Done(Arc::new(stored.result)), cx);
    }

    /// Shows the latest again; does nothing unless an older send is shown.
    pub fn back_to_latest(&mut self, cx: &mut Context<Self>) {
        if let Some((state, restored)) = self.stash.take() {
            self.viewing = None;
            self.restored = restored;
            let latest = self.latest_finished_at.take();
            self.finish(state, cx);
            self.finished_at = latest;
        }
    }

    /// The kept send the store made of the live result on screen; `None` before the store
    /// answers, for a result it did not keep, and for a kept result shown from history.
    pub fn kept_as(&self) -> Option<last_response::KeptAs> {
        let (sent, kept) = self.kept.as_ref()?;
        let SendState::Done(shown) = &self.state else {
            return None;
        };
        let live = self.restored.is_none() && self.viewing.is_none();
        (live && std::ptr::eq(sent.as_ptr(), Arc::as_ptr(shown))).then_some(*kept)
    }

    /// The seq of the older send shown, if one is.
    pub fn viewing(&self) -> Option<u64> {
        self.viewing
    }

    /// The secret values scrubbed from the send on screen; empty for a kept result.
    pub fn shown_secrets(&self) -> &[String] {
        &self.shown_secrets
    }

    fn leave_history(&mut self) {
        self.stash = None;
        self.viewing = None;
    }

    // Registers this model so clearing the store can take its kept result off screen.
    fn track_restored(&self, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let models = &mut cx.default_global::<RestoredModels>().0;
        models.retain(|m| m.upgrade().is_some());
        if !models.iter().any(|m| m.entity_id() == me.entity_id()) {
            models.push(me);
        }
    }

    /// Drops a shown kept result, back to `Idle`; does nothing unless `restored()` is `Some`.
    pub fn forget_restored(&mut self, cx: &mut Context<Self>) {
        self.leave_history();
        if self.restored.take().is_some() {
            self.state = SendState::Idle;
            cx.notify();
        }
    }

    /// When the shown result ended: the time a kept one was saved, else when the send finished
    /// in this session; `None` before any result.
    pub fn ended_at(&self) -> Option<SystemTime> {
        match &self.restored {
            Some(r) => Some(r.saved_at),
            None => self.finished_at,
        }
    }

    /// The kept-result facts of the shown result; `None` once anything else was shown.
    pub fn restored(&self) -> Option<Restored> {
        self.restored.clone()
    }

    /// [`SendModel::restored`] borrowed, for a caller that reads it on every render.
    pub fn restored_ref(&self) -> Option<&Restored> {
        self.restored.as_ref()
    }

    // A send's result is kept once, for the request it was sent as, when it got a response.
    fn done(&mut self, result: Arc<RequestResult>, cx: &mut Context<Self>) {
        // A live result ends any history view picked between a followed run's results.
        self.restored = None;
        self.shown_secrets.clear();
        self.leave_history();
        if let Some((root, item)) = self.keep_as.take()
            && result.response.is_some()
        {
            // A draft saved while this send was in flight keeps it under the saved file.
            let item = if item.is_draft() {
                self.workspace
                    .read(cx)
                    .saved_as(&root, &item)
                    .unwrap_or(item)
            } else {
                item
            };
            // The context's clones share the token cache, so this lookup sees a token the run
            // fetched or refreshed; the union keeps whatever the earlier lookup found.
            let mut secrets = std::mem::take(&mut self.secrets);
            if let Some((file, ctx)) = self.secrets_of.take() {
                extend_unique(&mut secrets, secret_values(&file, &ctx));
            }
            // A draft's snapshot also hides what its scripts marked sensitive and what core
            // derived as sent, such as a password typed into the url.
            let request = self.snapshot.take().map(|mut snap| {
                let mut hidden = secrets.clone();
                hidden.extend(result.script_sensitive_values().iter().cloned());
                hidden.extend(result.derived_credentials().iter().cloned());
                scrub::scrub_snapshot(&mut snap, &hidden);
                snap.to_json()
            });
            let extra = SendExtra {
                request,
                origin: None,
            };
            self.shown_secrets = secrets.clone();
            if let Some(answer) =
                last_response::record(root, item, Arc::clone(&result), secrets, extra, cx)
            {
                let sent = Arc::downgrade(&result);
                // A send that lands before this answer drops it; that send is then compared
                // by its masked values, never another send's.
                self.kept_task = Some(cx.spawn(async move |this, cx| {
                    if let Ok(kept) = answer.recv().await {
                        let _ = this.update(cx, |m, cx| {
                            m.kept = Some((sent, kept));
                            cx.notify();
                        });
                    }
                }));
            }
        }
        self.finish(SendState::Done(result), cx);
    }

    // What the pane showed before a send, so a cancelled prompt can put it back exactly.
    fn shown(&self) -> Shown {
        Shown {
            state: self.state.clone(),
            restored: self.restored.clone(),
            stash: self.stash.clone(),
            viewing: self.viewing,
            since: self.since,
            keep_as: self.keep_as.clone(),
            ever_shown: self.ever_shown,
        }
    }

    // Never drops `self.task`: the task itself calls this.
    fn put_back(&mut self, shown: Shown, cx: &mut Context<Self>) {
        self.state = shown.state;
        self.restored = shown.restored;
        self.stash = shown.stash;
        self.viewing = shown.viewing;
        self.since = shown.since;
        self.keep_as = shown.keep_as;
        self.ever_shown = shown.ever_shown;
        self.cancel = None;
        self.running = false;
        self.waiting = false;
        // Clearing the store while the prompt was open unregistered this model, and the
        // kept result it would put back is gone with it.
        let me = cx.entity_id();
        let tracked = cx
            .try_global::<RestoredModels>()
            .is_some_and(|g| g.0.iter().any(|m| m.entity_id() == me));
        if !tracked {
            self.forget_restored(cx);
        }
        cx.notify();
    }

    // Never drops `self.task`: the task itself calls this.
    fn finish(&mut self, state: SendState, cx: &mut Context<Self>) {
        if matches!(state, SendState::Done(_)) {
            self.finished_at = Some(SystemTime::now());
        }
        self.state = state;
        self.ended_events = self
            .live
            .take()
            .map(|l| l.events().to_vec())
            .unwrap_or_default();
        self.body_ended = false;
        self.cancel = None;
        self.running = false;
        self.waiting = false;
        self.since = None;
        cx.notify();
    }

    // Applies one run event that is not a request result to the streaming state.
    fn stream_event(&mut self, event: RunEvent) {
        // Response events after a result are the next run item's or stale; `live` is Some only
        // while streaming, so they are ignored outside Sending and Streaming.
        if !self.is_sending() {
            return;
        }
        match event {
            RunEvent::ResponseStarted {
                status,
                headers,
                event_stream,
                ..
            } => {
                self.live = Some(LiveStream::new(status, headers, event_stream));
                self.body_ended = false;
                if matches!(self.state, SendState::Sending) {
                    self.state = SendState::Streaming;
                }
            }
            RunEvent::ResponseChunk { bytes, .. } => {
                if let Some(live) = &mut self.live {
                    live.push(&bytes);
                }
            }
            RunEvent::ResponseEnded { .. } => self.body_ended = true,
            _ => {}
        }
    }

    fn wait(&mut self, waiting: bool, cx: &mut Context<Self>) {
        self.waiting = waiting;
        cx.notify();
    }
}

struct Shown {
    state: SendState,
    restored: Option<Restored>,
    stash: Option<(SendState, Option<Restored>)>,
    viewing: Option<u64>,
    since: Option<Instant>,
    keep_as: Option<(PathBuf, ItemPath)>,
    ever_shown: bool,
}

// Every model that showed a kept result, so clearing the store can take it back off screen.
#[derive(Default)]
struct RestoredModels(Vec<WeakEntity<SendModel>>);

impl Global for RestoredModels {}

/// Forgets the shown kept result in every live model that restored one, pruning dead handles.
pub fn forget_all_restored(cx: &mut App) {
    let models = std::mem::take(&mut cx.default_global::<RestoredModels>().0);
    for model in models.into_iter().filter_map(|m| m.upgrade()) {
        model.update(cx, |m, cx| m.forget_restored(cx));
    }
}

// The sender drops when the run ends, closing the channel; a run that dies before its
// RequestFinished must not leave the model Sending. Queued events are applied in batches of
// up to BATCH, one model update each, so a fast stream redraws once per batch. A body that
// fails mid-read may end with no ResponseEnded, so only RequestFinished leaves Streaming.
const BATCH: usize = 256;

async fn drain(
    this: WeakEntity<SendModel>,
    rx: async_channel::Receiver<RunEvent>,
    cx: &mut AsyncApp,
) {
    while let Ok(first) = rx.recv().await {
        let mut batch = vec![first];
        while batch.len() < BATCH {
            match rx.try_recv() {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }
        let _ = this.update(cx, |m, cx| {
            for event in batch {
                match event {
                    RunEvent::RequestFinished { result, .. } => m.done(result, cx),
                    other => m.stream_event(other),
                }
            }
            cx.notify();
        });
    }
    let _ = this.update(cx, |m, cx| {
        if m.is_sending() {
            m.finish(
                SendState::Failed("the request ended unexpectedly".into()),
                cx,
            );
        }
    });
}

/// The plan the collection actor answers with, or the user-facing reason there is none.
pub(crate) async fn await_plan(
    answer: async_channel::Receiver<Result<Planned, PlanError>>,
) -> Result<Planned, String> {
    match answer.recv().await {
        Ok(Ok(planned)) => Ok(planned),
        Ok(Err(e)) => Err(plan_message(&e)),
        Err(_) => Err("the collection was closed".into()),
    }
}

/// The user-facing text of a plan error: "<path> was not found", else the inner message.
pub fn plan_message(e: &PlanError) -> String {
    match e {
        PlanError::NotFound(path) => format!("{path} was not found"),
        PlanError::Select(msg) | PlanError::Context(msg) => msg.clone(),
    }
}

// Adds each of `values` to `into` unless it is already there, keeping the first order.
pub(crate) fn extend_unique(into: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !into.contains(&value) {
            into.push(value);
        }
    }
}

// Every secret value `file`'s context knows, the set saved examples replace too; none when
// its resolution fails.
pub(crate) fn secret_values(file: &RequestFile, ctx: &VarContext) -> Vec<String> {
    known_secrets(file, ctx)
        .map(|found| found.iter().map(|s| s.value().to_owned()).collect())
        .unwrap_or_default()
}
