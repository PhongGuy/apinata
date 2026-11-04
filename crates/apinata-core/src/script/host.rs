//! What a send shares with its script phases, and the installers that expose it to scripts.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use super::engine::Installer;
use super::{ExecutionMode, RunnerControl, ScriptPhase};
use crate::CancelHandle;
use crate::assertion::AssertionOutcome;
use crate::context::VarContext;
use crate::cookies::SetCookie;
use crate::execute::{ExecError, Executor};
use crate::request::RequestFile;
use crate::resolve::PreparedRequest;
use crate::response::Response;
use crate::runner::{Guard, Nesting};

const DEFERRED: &str = include_str!("deferred.js");

/// One send's state, shared by every phase of it. The installers of later tasks read and edit
/// it; `send_one` takes the edited request and the sensitive values back out.
pub(crate) struct HostState {
    pub(crate) ctx: VarContext,
    pub(crate) request: RequestFile,
    pub(crate) prepared: Arc<Mutex<PreparedRequest>>,
    /// The response, set once it arrives, for the post-response and tests phases.
    pub(crate) response: OnceLock<Response>,
    /// The body a script set with `res.setBody`, which later phases and the assertions read
    /// in place of the response's own.
    pub(crate) response_body: Mutex<Option<serde_json::Value>>,
    pub(crate) mode: ExecutionMode,
    /// Sensitive values a script read, so callers can redact what it logged.
    pub(crate) sensitive: Arc<Mutex<Vec<String>>>,
    /// The host policy's refusals of a script's own sends, once each, so a caller can ask for them.
    pub(crate) refused: Mutex<Vec<ExecError>>,
    /// The run's executor, guard and cancel handle, which a script's own sends go through.
    pub(crate) exec: Arc<Executor>,
    pub(crate) guard: Option<Guard>,
    pub(crate) cancel: CancelHandle,
    /// What scripts asked of the runner; the runner decides which of it to honour.
    pub(crate) control: Mutex<RunnerControl>,
    /// The request's assertion verdicts, set before the tests phase for `getAssertionResults`.
    pub(crate) assertions: OnceLock<Vec<AssertionOutcome>>,
    /// The send's nesting level, through which `bru.runRequest` runs another request.
    pub(crate) nesting: Nesting,
    /// Each cookie the scripts wrote or removed, with the phase whose script did it, in
    /// write order.
    pub(crate) cookies: Mutex<Vec<(ScriptPhase, SetCookie)>>,
}

impl HostState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        ctx: &VarContext,
        request: &RequestFile,
        prepared: PreparedRequest,
        mode: ExecutionMode,
        exec: Arc<Executor>,
        guard: Option<Guard>,
        cancel: CancelHandle,
        nesting: Nesting,
    ) -> HostState {
        HostState {
            ctx: ctx.clone(),
            request: request.clone(),
            prepared: Arc::new(Mutex::new(prepared)),
            response: OnceLock::new(),
            response_body: Mutex::new(None),
            mode,
            sensitive: Arc::new(Mutex::new(Vec::new())),
            refused: Mutex::new(Vec::new()),
            exec,
            guard,
            cancel,
            control: Mutex::new(RunnerControl::default()),
            assertions: OnceLock::new(),
            nesting,
            cookies: Mutex::new(Vec::new()),
        }
    }

    /// Records a cookie row a script of `phase` made.
    pub(crate) fn record_cookie(&self, phase: ScriptPhase, row: SetCookie) {
        let mut cookies = self.cookies.lock().unwrap_or_else(PoisonError::into_inner);
        cookies.push((phase, row));
    }

    /// The response with the body a script set through `res.setBody`; `None` when none did.
    pub(crate) fn edited_response(&self) -> Option<Response> {
        let body = self
            .response_body
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let text = super::req_res::body_text(body.as_ref()?);
        Some(self.response.get()?.clone().with_text_body(text))
    }
}

/// The installers for one phase, in the order they run; later tasks append theirs here.
/// `deadline` bounds every run the phase nests through `bru.runRequest`.
pub(crate) fn installers(
    state: &Arc<HostState>,
    phase: ScriptPhase,
    deadline: Option<Instant>,
) -> Vec<Installer> {
    let with_res = phase != ScriptPhase::PreRequest;
    vec![
        Box::new(|ctx| ctx.eval::<(), _>(DEFERRED)),
        super::bru::installer(Arc::clone(state)),
        super::req_res::installer(Arc::clone(state), with_res),
        super::chai::installer(),
        super::send::installer(Arc::clone(state)),
        super::run_request::installer(Arc::clone(state), deadline),
        super::oauth2::installer(Arc::clone(state)),
        super::timers::installer(),
        super::cookies::installer(Arc::clone(state), phase),
        super::libs::installer(),
        super::host_libs::installer(),
        super::crypto::installer(),
        super::jwt::installer(),
        super::web::installer(),
        super::modules::installer(Arc::clone(state)),
    ]
}

/// The installers for one expression: `bru`, `req` and `res` over the deferred stubs, with no
/// sends, timers or libraries, so those members throw as unsupported.
pub(crate) fn expression_installers(state: &Arc<HostState>) -> Vec<Installer> {
    vec![
        Box::new(|ctx| ctx.eval::<(), _>(DEFERRED)),
        super::bru::installer(Arc::clone(state)),
        super::req_res::installer(Arc::clone(state), true),
        super::bru::process_env_installer(Arc::clone(state)),
    ]
}
