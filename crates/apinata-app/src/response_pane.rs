//! The response pane: a status line over Body, Headers, Cookies, Timeline, Tests and Console
//! tabs for one tab's send, with run errors shown in place of the body.

use crate::ui::LockButton as _;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime};

use apinata_core::sse::{self, SseEvent};
use apinata_core::{
    AssertionOutcome, BodyState, Comparison, ConnectCause, ConnectHint, ConsoleEntry, ConsoleLevel,
    CookieInfo, CookieOutcome, CookieRejection, ExecError, HOP_BODY_LIMIT, ItemPath, OAuth2Error,
    Outcome, RequestResult, ResolveError, Response, RunError, ScriptPhase, TestOutcome,
    UndefinedHint, Verdict, is_credential_header, is_credential_name,
};
use gpui_kit::base::{ObservedElement, SelectableText};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, Anchor, AnyElement, App, ClipboardItem, Div, ElementId, Entity, FocusHandle, Focusable,
    FontFeatures, FontWeight, Global, Hsla, Pixels, Rems, ScrollHandle, ScrollStrategy,
    SharedString, Stateful, Subscription, Svg, Task, TestSupportExt, UniformListScrollHandle,
    WeakEntity, Window, canvas, div, relative, rems, svg, uniform_list,
};

use crate::app::AppState;
use crate::body_text::{format_size, format_time, size_labels};
use crate::body_view::{BodyView, MASKED, SCRUBBED_TIP, SetVariable, Shown};
use crate::compare_rows::{ComparePalette, RevealHeader, RowActions, Toggle, compare_row};
use crate::compare_source::{self, CompareSide, CompareSource, HeaderValue, SourceChoice};
use crate::compare_view::{self, CompareRow};
use crate::diff_doc::DiffRow;
use crate::download_model::DownloadModel;
use crate::executor;
use crate::kv_grid::{ActionShow, KvColumn, action_slot};
use crate::last_response::{self, KeptAs, KeptSend, ResponseStore, StoreOp, Stored};
use crate::live_stream::{self, EventRow};
use crate::request_editor::SendRequest;
use crate::save_example;
use crate::scroll_follow::{FocusFollow, Followed};
use crate::send_model::{SendModel, SendState};
use crate::tab_area::{ExportHar, SaveAsExample};
use crate::timing_bar::{self, PhaseKind, Phases};
use crate::ui::{KvCell, KvView, RingRoom as _, RingSides, Tone};
use crate::{style, ui};
use gpui_kit::assets::IconName as AssetIcon;

/// How the error block reads: a calm note for a cancel, an alert for a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorTone {
    /// Muted, with a stop sign: nothing went wrong, the user stopped it.
    Neutral,
    /// The danger colour with an alert icon.
    Danger,
}

/// What kind of failure an error block reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The user stopped the send.
    Cancelled,
    /// Nothing listens at the address.
    Refused,
    /// The host name could not be resolved.
    Dns,
    /// The TLS handshake failed.
    Tls,
    /// Any other connection failure.
    Connect,
    /// No response within the timeout.
    Timeout,
    /// The proxy failed.
    Proxy,
    /// The client certificate could not be loaded.
    Certificate,
    /// The host guard refused the host.
    HostNotAllowed,
    /// A variable has no value.
    UndefinedVariable,
    /// The request was stopped before it was sent.
    NotSent,
    /// A subscription was refused at its handshake or its socket ended with an error.
    Subscription,
    /// Anything else.
    Failed,
}

/// What the error block shows in place of the response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorView {
    /// What kind of failure this is.
    pub kind: ErrorKind,
    /// The headline.
    pub title: &'static str,
    /// One plain sentence under the headline.
    pub sentence: Option<String>,
    /// The redacted error text, shown under "Show details".
    pub message: Option<String>,
    /// How the block reads.
    pub tone: ErrorTone,
    /// Whether a Retry is offered.
    pub retry: bool,
    /// The variable an "Add to environment" button offers to define.
    pub add_variable: Option<String>,
    /// The other environments defining the variable, each offered as a "Switch to" button.
    pub use_environments: Vec<String>,
}

/// The block for a run's `error`; `details` is its redacted text and `env` the selected
/// collection environment.
pub fn describe_error(error: &RunError, details: &str, env: Option<&str>) -> ErrorView {
    let view = |kind, title, sentence: String, retry| ErrorView {
        kind,
        title,
        sentence: Some(sentence),
        message: Some(ui::plain_quotes(details)),
        tone: ErrorTone::Danger,
        retry,
        add_variable: None,
        use_environments: Vec::new(),
    };
    let undefined = match error {
        RunError::Resolve(ResolveError::UndefinedVariable { name, .. })
        | RunError::OAuth2(OAuth2Error::Resolve(ResolveError::UndefinedVariable {
            name, ..
        })) => Some(name.as_str()),
        _ => None,
    };
    if let Some(name) = undefined {
        let (mut sentence, add) = undefined_sentence(name, env);
        // The hint says why it is undefined; the app words the remedy with its own controls.
        let mut use_environments = Vec::new();
        if let Some(hint) = error.hint() {
            sentence = format!("{sentence} {}.", app_hint(hint, name));
            if let UndefinedHint::DefinedElsewhere { environments } = hint {
                use_environments = environments.clone();
            }
        }
        return ErrorView {
            add_variable: add,
            use_environments,
            ..view(
                ErrorKind::UndefinedVariable,
                "Undefined variable",
                sentence,
                false,
            )
        };
    }
    match error {
        // The person said no to the network prompt: a calm note, never a fault.
        RunError::Exec(ExecError::NetworkDeclined) => ErrorView {
            kind: ErrorKind::Cancelled,
            title: "Cancelled",
            sentence: Some(as_sentence(&ui::plain_quotes(details))),
            message: None,
            tone: ErrorTone::Neutral,
            retry: true,
            add_variable: None,
            use_environments: Vec::new(),
        },
        RunError::Exec(ExecError::Connect { url, cause, .. }) => {
            let hint = ConnectHint::new(url, *cause);
            let kind = match cause {
                ConnectCause::Refused => ErrorKind::Refused,
                ConnectCause::Dns => ErrorKind::Dns,
                ConnectCause::Tls
                | ConnectCause::TlsUnknownIssuer
                | ConnectCause::TlsWrongProtocol
                | ConnectCause::TlsExpired => ErrorKind::Tls,
                _ => ErrorKind::Connect,
            };
            view(kind, hint.title(), hint.sentence().to_owned(), true)
        }
        RunError::Exec(ExecError::Timeout { .. }) => view(
            ErrorKind::Timeout,
            "Timed out",
            "No response within the request timeout.".into(),
            true,
        ),
        RunError::Exec(ExecError::Proxy { proxy, .. }) => view(
            ErrorKind::Proxy,
            "Proxy error",
            format!("The proxy {proxy} could not be reached or refused the request."),
            true,
        ),
        RunError::Exec(ExecError::Certificate { .. }) => view(
            ErrorKind::Certificate,
            "Client certificate error",
            "The client certificate could not be loaded.".into(),
            true,
        ),
        RunError::Exec(
            ExecError::HostNotAllowed { host, .. } | ExecError::ProxyNotAllowed { host, .. },
        ) => view(
            ErrorKind::HostNotAllowed,
            "Host not allowed",
            format!("Sending to {host} is not allowed here."),
            true,
        ),
        RunError::Exec(ExecError::File { .. }) => view(
            ErrorKind::NotSent,
            "Request not sent",
            "A body file could not be read.".into(),
            false,
        ),
        RunError::Subscription { .. } => {
            let plain = ui::plain_quotes(details);
            let first = plain.lines().next().unwrap_or_default().to_owned();
            view(ErrorKind::Subscription, "Subscription failed", first, true)
        }
        RunError::Resolve(_) | RunError::Exec(ExecError::SecretInProxy) => view(
            ErrorKind::NotSent,
            "Request not sent",
            as_sentence(&ui::plain_quotes(details)),
            false,
        ),
        _ => {
            let plain = ui::plain_quotes(details);
            let first = plain.lines().next().unwrap_or_default().to_owned();
            let empty = first.is_empty();
            let v = view(ErrorKind::Failed, "Request failed", first, true);
            // An empty first line says nothing, so the title stands alone.
            ErrorView {
                sentence: v.sentence.filter(|_| !empty),
                ..v
            }
        }
    }
}

/// A connection, timeout, proxy or certificate failure as "Title: sentence", in the words a
/// failed send's block uses; `None` for any other error.
pub fn transport_text(error: &ExecError) -> Option<String> {
    let view = describe_error(&RunError::Exec(error.clone()), "", None);
    let transport = matches!(
        view.kind,
        ErrorKind::Refused
            | ErrorKind::Dns
            | ErrorKind::Tls
            | ErrorKind::Connect
            | ErrorKind::Timeout
            | ErrorKind::Proxy
            | ErrorKind::Certificate
    );
    transport.then_some(format!("{}: {}", view.title, view.sentence?))
}

/// The block for a send stopped before any run (a plan error, a closed collection).
pub fn describe_failure(message: &str) -> ErrorView {
    ErrorView {
        kind: ErrorKind::NotSent,
        title: "Request not sent",
        sentence: Some(ui::plain_quotes(message)),
        message: None,
        tone: ErrorTone::Danger,
        retry: false,
        add_variable: None,
        use_environments: Vec::new(),
    }
}

// The undefined-variable sentence, and the name to offer when an environment can define it.
// The core hint worded for the app: its environment editor and selector, never CLI flags.
fn app_hint(hint: &UndefinedHint, name: &str) -> String {
    match hint {
        UndefinedHint::SecretWithoutValue { environment } => {
            format!(
                "{name} is a secret with no value in environment {environment}; set it in the environment editor"
            )
        }
        UndefinedHint::SecretUnreadable {
            environment,
            reason,
        } => format!(
            "{name} is a secret in environment {environment}; its stored value could not be read ({reason}); set it in the environment editor"
        ),
        // The block offers a "Switch to" button per environment, so the text only names them.
        UndefinedHint::DefinedElsewhere { environments } => match environments.as_slice() {
            [one] => format!("{name} is defined in environment {one}"),
            many => format!("{name} is defined in environments {}", many.join(", ")),
        },
        // A future hint kind reads in the core's words until the app words it.
        other => other.cli_text(name),
    }
}

fn undefined_sentence(name: &str, env: Option<&str>) -> (String, Option<String>) {
    if let Some(var) = name.strip_prefix("process.env.") {
        let s =
            format!("{var} is not set in the process environment or the collection's .env file.");
        return (s, None);
    }
    let definable = !name.is_empty()
        && !name.starts_with(['$', '?'])
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c == '{' || c == '}');
    if !definable {
        return (format!("{name} is not defined."), None);
    }
    let s = match env {
        Some(env) => format!("{name} is not defined in {env}."),
        None => format!("{name} is not defined, and no environment is selected."),
    };
    (s, Some(name.to_owned()))
}

// Core's lower-case message as a sentence: a capital first letter and a final stop.
fn as_sentence(text: &str) -> String {
    let mut out = crate::import_report::failure_text(text);
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

// A result, held weakly so it can be told apart, and its console lines redacted.
type RedactedConsole = (Weak<RequestResult>, Rc<[ConsoleEntry]>);

// A Timeline value as shown masked and as shown revealed; the eye exists when they differ.
#[derive(Clone)]
struct Masked {
    hidden: String,
    revealed: String,
}

impl Masked {
    fn eye(&self) -> bool {
        self.hidden != self.revealed
    }
}

// One hop's values, masked once per result. A header masked for its credential name carries
// that name, lower-cased, since its reveal is kept by name for the session.
struct HopMasks {
    url: Masked,
    request_headers: Vec<Masked>,
    request_named: Vec<Option<String>>,
    request_body: Option<Masked>,
    response_headers: Vec<Masked>,
    response_named: Vec<Option<String>>,
}

/// What a header whose name marks it a credential shows in the Timeline until revealed.
pub const NAME_MASK: &str = crate::env_table::SECRET_HIDDEN;

// The credential header names revealed in the Timeline, by collection root; they stay revealed
// for the session but never carry into another collection.
#[derive(Default)]
struct NamedReveals(BTreeSet<(Option<PathBuf>, String)>);

impl Global for NamedReveals {}

// Which part of a hop a reveal belongs to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Field {
    Url,
    RequestHeader,
    RequestBody,
    ResponseHeader,
    // A row of the Headers tab, by its place in the response's headers.
    Header,
}

// A Headers tab row: name, shown value, eye (None without one, else revealed) and whether the
// eye is locked.
type HeaderRow = (String, String, Option<bool>, bool);

// The reveal key: hop, field and row within the field.
type RevealKey = (usize, Field, usize);

// The masks for one result (Timeline values, response headers and cookie values), with the
// reveals made on them. A different result replaces the whole value, which forgets the reveals.
struct ResultMasks {
    result: Weak<RequestResult>,
    hops: Rc<[HopMasks]>,
    headers: Rc<[Masked]>,
    // Cookie values already masked, by value, built as the Cookies tab lists them.
    cookies: HashMap<String, Masked>,
    revealed: BTreeSet<RevealKey>,
    // A cookie is told apart by its name, domain and path, which survive a delete above it.
    revealed_cookies: BTreeSet<(String, String, String)>,
    // A Set-Cookie row of this response is told apart by its hop and its place in that hop.
    revealed_set: BTreeSet<(usize, usize)>,
}

/// The pane's tabs, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseTab {
    /// The response body.
    Body,
    /// The final response's headers.
    Headers,
    /// The jar's cookies for the final URL's host.
    Cookies,
    /// Each hop of the exchange, redirects included.
    Timeline,
    /// Each enabled assertion and its verdict, then each script test.
    Tests,
    /// What the request's scripts logged.
    Console,
    /// The response compared with another send or a saved example; listed only while a
    /// source is set.
    Compare,
    /// The parsed events of an event-stream; listed right after Body for one.
    Events,
}

impl ResponseTab {
    const ALL: [ResponseTab; 8] = [
        ResponseTab::Body,
        ResponseTab::Headers,
        ResponseTab::Cookies,
        ResponseTab::Timeline,
        ResponseTab::Tests,
        ResponseTab::Console,
        ResponseTab::Compare,
        ResponseTab::Events,
    ];
    // The tabs always on offer; Compare joins them while a comparison is open.
    const BASE: [ResponseTab; 6] = [
        ResponseTab::Body,
        ResponseTab::Headers,
        ResponseTab::Cookies,
        ResponseTab::Timeline,
        ResponseTab::Tests,
        ResponseTab::Console,
    ];

    fn label(self) -> &'static str {
        match self {
            ResponseTab::Body => "Body",
            ResponseTab::Headers => "Headers",
            ResponseTab::Cookies => "Cookies",
            ResponseTab::Timeline => "Timeline",
            ResponseTab::Tests => "Tests",
            ResponseTab::Console => "Console",
            ResponseTab::Compare => "Compare",
            ResponseTab::Events => "Events",
        }
    }
}

/// The status line's facts about a received response.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The status code.
    pub status: u16,
    /// The reason phrase.
    pub reason: String,
    /// The response time, formatted.
    pub time: String,
    /// The body size as received, formatted.
    pub size: String,
    /// Whether the body was cut at the size limit.
    pub truncated: bool,
    /// The "From ..." label of a kept response, if one is shown.
    pub saved: Option<String>,
}

/// Where the pane's request lives, read at call time so renames follow.
pub type HistoryTarget = Rc<dyn Fn(&App) -> Option<(PathBuf, ItemPath)>>;

/// What the response shows before its tab has sent anything.
pub const RESPONSE_HINT: &str = "Send a request to see the response";

/// The collapsed response before a tab first sends: one row ("Response", the hint, Send's key)
/// stacked; a `RESPONSE_STRIP_WIDTH` column with the hint wrapped side by side.
pub fn response_strip(horizontal: bool, window: &Window, cx: &App) -> impl IntoElement {
    let label = div()
        .id("response-strip-label")
        .test_support()
        .flex_none()
        .child("Response");
    let hint = div()
        .id("response-hint")
        .test_support()
        .min_w_0()
        .text_color(cx.theme().muted_foreground)
        .when(!horizontal, |d| d.truncate())
        .child(RESPONSE_HINT);
    let strip = match horizontal {
        // Items keep their own width, so the key chip fits its text and not the column.
        true => v_flex()
            .w(style::RESPONSE_STRIP_WIDTH)
            .h_full()
            .items_start()
            .gap(style::SPACE_1),
        false => h_flex()
            .w_full()
            .h(style::ROW_HEIGHT)
            .items_center()
            .gap(style::SPACE_2),
    };
    strip
        .flex_none()
        .px(style::SPACE_2)
        .overflow_hidden()
        .text_size(style::TEXT_SMALL)
        .child(label)
        .child(hint)
        .children(
            ui::key_hint(
                &SendRequest,
                Some(crate::request_editor::KEY_CONTEXT),
                window,
                cx,
            )
            .map(|kbd| {
                div()
                    .id("response-strip-kbd")
                    .test_support()
                    .flex_none()
                    .child(kbd)
            }),
        )
}

// The menu's schema test entry and whether it is locked; with no tooltip in a menu, a lock
// gives its reason in the label.
fn schema_menu_label(lock: Option<&str>) -> (String, bool) {
    match lock {
        None => ("Add schema test".to_owned(), false),
        Some(why) => {
            let mut chars = why.chars();
            let first = chars.next().map(|c| c.to_lowercase().collect::<String>());
            let why = first.unwrap_or_default() + chars.as_str();
            (format!("Add schema test ({why})"), true)
        }
    }
}

// The open comparison: its source, the result once computed, and what the list shows of it.
struct CompareState {
    source: CompareSource,
    comparison: Option<Arc<Comparison>>,
    rows: Arc<[CompareRow]>,
    // The text-diff rows alone, which size the line-number gutter.
    diff_rows: Arc<[DiffRow]>,
    volatile_open: bool,
    scroll: UniformListScrollHandle,
    // Each side's header values, which the rows read their text from, and the header names
    // whose rows are revealed.
    headers: (Vec<HeaderValue>, Vec<HeaderValue>),
    revealed: Vec<String>,
}

impl CompareState {
    fn new(source: CompareSource) -> Self {
        CompareState {
            source,
            comparison: None,
            rows: Arc::from(Vec::new()),
            diff_rows: Arc::from(Vec::new()),
            volatile_open: false,
            scroll: UniformListScrollHandle::new(),
            headers: Default::default(),
            revealed: Vec::new(),
        }
    }

    // Flattens the comparison into rows, keeping the scroll position.
    fn refresh_rows(&mut self) {
        let Some(c) = &self.comparison else { return };
        let reveal = compare_view::Reveal {
            old: &self.headers.0,
            new: &self.headers.1,
            revealed: &self.revealed,
        };
        let rows = compare_view::rows_revealing(c, self.volatile_open, &reveal);
        self.diff_rows = rows
            .iter()
            .filter_map(|r| match r {
                CompareRow::Diff(d) => Some(d.clone()),
                _ => None,
            })
            .collect();
        self.rows = rows.into();
    }
}

/// Shows one `SendModel`'s state: a placeholder while idle or sending, else the result.
pub struct ResponsePane {
    renders: usize,
    send: Entity<SendModel>,
    body: Entity<BodyView>,
    // Re-runs the shown request into a file for the body's "Download full response".
    download: Entity<DownloadModel>,
    tab: ResponseTab,
    // The tab strip's scroll, so the chosen tab can be scrolled into view.
    tab_scroll: ScrollHandle,
    // The header's intrinsic widths as last laid out, which floor the pane side by side.
    min: MinWidths,
    // The live tail's list scroll; it follows the newest row while it sits at the end.
    live_scroll: UniformListScrollHandle,
    // The Events list's scroll; live, it follows the newest event while it sits at the end.
    events_scroll: UniformListScrollHandle,
    // Scrolls the Timeline to whichever of its eyes Tab lands on.
    timeline_follow: FocusFollow,
    // Whether the send is streaming, which narrows the strip to the tabs a live body has.
    streaming: bool,
    // Whether the Events tab is on offer: the live or shown response is an event-stream.
    events_tab: bool,
    // The shown result's events, parsed off-thread once per result.
    done_events: Option<(Weak<RequestResult>, Rc<Vec<SseEvent>>)>,
    events_task: Option<Task<()>>,
    // One Tab stop per tab, indexed by `ResponseTab::ALL` position.
    tab_focus: Vec<FocusHandle>,
    tab_reveal: ui::Reveal,
    // Keeps the tab that holds keyboard focus in view, apart from the chosen one.
    tab_focus_reveal: ui::Reveal,
    tab_fades: ui::StripFades,
    cookies: Vec<CookieInfo>,
    // What the Cookies tab lists; it belongs to the pane, so it outlives each send.
    cookies_view: CookiesView,
    ticker: Option<Task<()>>,
    expiry_ticker: Option<Task<()>>,
    // The soonest moment an "N ago" label drawn this frame reads differently, and the
    // timer armed for it.
    age_due: Cell<Option<SystemTime>>,
    age_ticker: Option<(SystemTime, Task<()>)>,
    focus: FocusHandle,
    retry: bool,
    save_example: bool,
    elapsed: bool,
    // The tabs on offer, a subset of `ResponseTab::BASE` in its order.
    tabs: &'static [ResponseTab],
    // Whether the Compare button is offered.
    compare_offered: bool,
    compare: Option<CompareState>,
    // Only the newest comparison may land, so a slow older one never overwrites it.
    compare_generation: u64,
    compare_task: Option<Task<()>>,
    // The real header values of the sends made live in this session, by request and kept send,
    // so Compare can find what the at-rest mask hides. Memory only, pruned to the kept history.
    live_headers: HashMap<(PathBuf, ItemPath, u64), (KeptAs, Vec<HeaderValue>)>,
    // The Console tab's level filter; it belongs to the pane, so it outlives each send.
    console_filter: Option<ConsoleLevel>,
    // The shown result's console lines redacted once per result, as renders repeat.
    console_redacted: RefCell<Option<RedactedConsole>>,
    // The Timeline's masked values and reveals for the shown result.
    timeline_masks: RefCell<Option<ResultMasks>>,
    target: Option<HistoryTarget>,
    // Where the tab's sends are kept, which for a draft differs from its item.
    key: Option<HistoryTarget>,
    history: Vec<KeptSend>,
    // Only the newest history read may land, so a slow older reply never overwrites it.
    history_task: Option<Task<()>>,
    // Likewise only the newest history pick may land, so quick picks show the last one.
    load_task: Option<Task<()>>,
    // Whether the pane was narrower than its wide header's floor when last laid out.
    compact: Rc<Cell<bool>>,
    // The pane's laid-out height, and the room of the split it sits in (set by the editor).
    height: Rc<Cell<Pixels>>,
    room: Rc<Cell<Pixels>>,
    // The error block's details are shown, and the error they were opened for.
    details_open: bool,
    details_for: Option<ErrorView>,
    _subscription: Subscription,
    // The Headers and Cookies tabs' filter fields; they belong to the pane like the console filter.
    headers_filter: Entity<InputState>,
    cookies_filter: Entity<InputState>,
    _filter_events: Vec<Subscription>,
    _body_events: Subscription,
    _inputs: Vec<Subscription>,
}

// How often the elapsed time redraws while a send is under way.
const TICK: Duration = Duration::from_millis(100);
// How often the Cookies tab redraws so expiry countdowns stay current.
const EXPIRY_TICK: Duration = Duration::from_secs(1);
// How late past an "N ago" label's change its redraw wakes, so a clock read a moment
// early cannot draw the old text again.
const AGE_SLACK: Duration = Duration::from_millis(500);

// Every pane that lists history, so clearing the store can empty each list.
#[derive(Default)]
struct HistoryPanes(Vec<WeakEntity<ResponsePane>>);

impl Global for HistoryPanes {}

/// Re-reads the history list of every live pane that has one, pruning dead handles.
pub fn refresh_all_history(cx: &mut App) {
    let panes = &mut cx.default_global::<HistoryPanes>().0;
    panes.retain(|p| p.upgrade().is_some());
    for pane in panes.clone().into_iter().filter_map(|p| p.upgrade()) {
        pane.update(cx, |p, cx| p.refresh_history(cx));
    }
}

impl ResponsePane {
    /// A pane on the Body tab showing `send`'s current state, then following it.
    pub fn new(send: Entity<SendModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let body = cx.new(|cx| BodyView::new(window, cx));
        let current = done(send.read(cx));
        let download = {
            let workspace = send.read(cx).workspace().clone();
            cx.new(|_| DownloadModel::new(workspace))
        };
        wire_download(&body, &download, &send, window, cx);
        let scrubbed = body_scrubbed(&send, cx);
        body.update(cx, |b, cx| {
            b.set_scrubbed(scrubbed);
            b.show(current.clone(), window, cx)
        });
        // Re-show the body only when the result itself changes, so a re-render of the
        // model keeps the body's prepared text and scroll position.
        let mut last = current;
        let headers_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter headers"));
        let cookies_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter cookies"));
        let filter_events = [&headers_filter, &cookies_filter]
            .map(|input| {
                cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .into_iter()
            .collect::<Vec<_>>();
        let subscription = cx.observe_in(&send, window, move |this, send, window, cx| {
            let next = done(send.read(cx));
            let same = match (&last, &next) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if !same {
                last = next.clone();
                this.refresh_history(cx);
                this.list_cookies(cx);
                this.tick_expiry(cx);
                wire_download(&this.body, &this.download, &send, window, cx);
                let scrubbed = body_scrubbed(&send, cx);
                this.body.update(cx, |b, cx| {
                    b.set_scrubbed(scrubbed);
                    b.show(next, window, cx)
                });
            }
            let was_streaming = this.streaming;
            this.streaming = send.read(cx).is_streaming();
            this.refresh_events_tab(cx);
            if !same {
                this.parse_events(was_streaming, cx);
            }
            // Each new send's error opens with its details collapsed.
            if send.read(cx).is_sending() {
                this.details_open = false;
            }
            this.keep_live_headers(cx);
            this.tick(cx);
            cx.notify();
        });
        // The body asks for the Set as variable dialog; the pane knows the collection.
        let body_events = cx.subscribe_in(&body, window, |this, _, e: &SetVariable, window, cx| {
            this.open_set_variable(e, window, cx)
        });
        // The pane is drawn cached, so the collection, the script approvals and the settings
        // it reads (error offers, warnings, the live body's font size) redraw it on change.
        let workspace = send.read(cx).workspace().clone();
        let mut inputs = vec![cx.observe(&workspace, |_, _, cx| cx.notify())];
        if let Some(approvals) = cx.try_global::<AppState>().map(|s| s.approvals.clone()) {
            inputs.push(cx.observe(&approvals, |_, _, cx| cx.notify()));
        }
        inputs.extend(ui::redraw_on_settings(cx));
        let mut pane = ResponsePane {
            renders: 0,
            send,
            body,
            download,
            tab: ResponseTab::Body,
            tab_scroll: ScrollHandle::new(),
            min: MinWidths::default(),
            live_scroll: UniformListScrollHandle::new(),
            events_scroll: UniformListScrollHandle::new(),
            timeline_follow: FocusFollow::default(),
            streaming: false,
            events_tab: false,
            done_events: None,
            events_task: None,
            tab_focus: ResponseTab::ALL
                .iter()
                .map(|_| cx.focus_handle().tab_stop(true))
                .collect(),
            tab_reveal: ui::Reveal::whole_at_left(),
            tab_focus_reveal: ui::Reveal::default(),
            tab_fades: ui::StripFades::default(),
            cookies: Vec::new(),
            cookies_view: CookiesView::Response,
            ticker: None,
            expiry_ticker: None,
            age_due: Cell::new(None),
            age_ticker: None,
            focus: cx.focus_handle(),
            retry: true,
            save_example: true,
            elapsed: true,
            tabs: &ResponseTab::BASE,
            compare_offered: true,
            compare: None,
            compare_generation: 0,
            compare_task: None,
            live_headers: HashMap::new(),
            console_filter: None,
            console_redacted: RefCell::default(),
            timeline_masks: RefCell::default(),
            target: None,
            key: None,
            history: Vec::new(),
            history_task: None,
            load_task: None,
            compact: Rc::default(),
            height: Rc::default(),
            room: Rc::default(),
            details_open: false,
            details_for: None,
            _subscription: subscription,
            headers_filter,
            cookies_filter,
            _filter_events: filter_events,
            _body_events: body_events,
            _inputs: inputs,
        };
        pane.refresh_events_tab(cx);
        pane.parse_events(false, cx);
        pane.list_cookies(cx);
        pane.tick(cx);
        pane.tick_expiry(cx);
        pane
    }

    // Opens the Set as variable dialog for the send's collection, or says why it cannot.
    fn open_set_variable(&self, e: &SetVariable, window: &mut Window, cx: &mut Context<Self>) {
        let send = self.send.read(cx);
        let (workspace, root) = (send.workspace().clone(), send.root().map(Path::to_path_buf));
        match root {
            Some(root) => {
                crate::set_variable::open_value(
                    &workspace,
                    &root,
                    &e.name,
                    &e.value,
                    e.sensitive,
                    window,
                    cx,
                );
            }
            None => window.push_notification(
                ui::info_toast("This response has no collection to set a variable in"),
                cx,
            ),
        }
    }

    /// The same pane without the error block's Retry, for a host that has no request to
    /// send again.
    pub fn without_retry(mut self) -> Self {
        self.retry = false;
        self
    }

    /// Hides the "Save response as example" button, for panes that do not show a send of their own.
    pub fn without_save_example(mut self) -> Self {
        self.save_example = false;
        self
    }

    /// Hides the elapsed time, for a response that was never timed.
    pub fn without_elapsed(mut self) -> Self {
        self.elapsed = false;
        self
    }

    /// Offers only the Body and Headers tabs, for a response with no exchange behind it.
    pub fn without_exchange_tabs(mut self) -> Self {
        self.tabs = &[ResponseTab::Body, ResponseTab::Headers];
        self
    }

    /// Hides the Compare button, for a pane that has no other send to compare with.
    pub fn without_compare(mut self) -> Self {
        self.compare_offered = false;
        self
    }

    /// Keeps the body read-only by hiding its View and Edit toggle.
    pub fn without_body_edit(self, cx: &mut Context<Self>) -> Self {
        self.body.update(cx, |b, _| b.set_editable(false));
        self
    }

    /// The same pane for the request `target` names, listing the sends kept under `key`.
    pub fn with_history(
        mut self,
        target: HistoryTarget,
        key: HistoryTarget,
        cx: &mut Context<Self>,
    ) -> Self {
        self.target = Some(target);
        self.key = Some(key);
        self.refresh_history(cx);
        let me = cx.entity().downgrade();
        let panes = &mut cx.default_global::<HistoryPanes>().0;
        panes.retain(|p| p.upgrade().is_some());
        panes.push(me);
        self
    }

    /// The kept sends of this pane's request, newest first; empty without a target or store.
    pub fn history(&self) -> &[KeptSend] {
        &self.history
    }

    /// Loads kept send `seq` through the store queue and shows it in the send model.
    pub fn show_history(&mut self, seq: u64, cx: &mut Context<Self>) {
        let Some((root, item)) = self.key.as_ref().and_then(|k| k(cx)) else {
            return;
        };
        let (reply, answer) = async_channel::bounded(1);
        // With remembering off the store is only read, so a version 1 store stays as it is.
        let remember = cx
            .try_global::<AppState>()
            .is_none_or(|app| app.settings.read(cx).get().remember_responses);
        let op = if remember {
            StoreOp::LoadSend {
                root,
                item,
                seq,
                reply,
            }
        } else {
            StoreOp::PeekSend {
                root,
                item,
                seq,
                reply,
            }
        };
        last_response::enqueue(op, cx);
        // A send cleared or evicted since the list was read comes back as None; re-reading
        // the list then drops its stale row.
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let stored = answer.recv().await.ok().flatten();
            let _ = this.update(cx, |p, cx| match stored {
                Some(stored) => p.send.update(cx, |m, cx| m.view_kept(seq, stored, cx)),
                None => p.refresh_history(cx),
            });
        }));
    }

    // Re-read the request's kept sends through the store queue, so the list sees any save
    // queued before it. A tab without a history key lists nothing.
    fn refresh_history(&mut self, cx: &mut Context<Self>) {
        let Some((root, item)) = self.key.as_ref().and_then(|k| k(cx)) else {
            self.history.clear();
            self.live_headers.clear();
            self.history_task = None;
            self.recompute_compare(cx);
            cx.notify();
            return;
        };
        let (reply, answer) = async_channel::bounded(1);
        last_response::enqueue(StoreOp::History { root, item, reply }, cx);
        self.history_task = Some(cx.spawn(async move |this, cx| {
            let list = answer.recv().await.unwrap_or_default();
            let _ = this.update(cx, |p, cx| {
                p.live_headers
                    .retain(|_, (kept, _)| list.iter().any(|k| kept.is(k)));
                p.history = list;
                // "Previous" moves with the history, so an open comparison is redone.
                p.recompute_compare(cx);
                cx.notify();
            });
        }));
    }

    // Keep one redraw timer running exactly while the model is sending, so the elapsed
    // time advances; the loop also ends on its own once the send is over or the pane drops.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.send.read(cx).is_sending() {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let sending = this.update(cx, |p, cx| {
                    cx.notify();
                    p.send.read(cx).is_sending()
                });
                if !matches!(sending, Ok(true)) {
                    break;
                }
            }
        }));
    }

    // Notes an "N ago" label for `saved` drawn at `now`, so the frame's timer wakes when the
    // soonest one changes.
    fn note_age(&self, saved: SystemTime, now: SystemTime) {
        let elapsed = now.duration_since(saved).unwrap_or_default();
        let due = now + last_response::next_age_change(elapsed);
        let soonest = self.age_due.get().map_or(due, |d| d.min(due));
        self.age_due.set(Some(soonest));
    }

    // Keep one timer armed for the soonest change among the labels this frame drew. It
    // redraws the pane, whose render arms the next; a frame without one disarms it.
    fn tick_age(&mut self, cx: &mut Context<Self>) {
        let due = self.age_due.take();
        if self.age_ticker.as_ref().map(|(at, _)| *at) == due {
            return;
        }
        self.age_ticker = due.map(|at| {
            let wait = at
                .duration_since(last_response::wall_now(cx))
                .unwrap_or_default()
                + AGE_SLACK;
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                this.update(cx, |p, cx| {
                    p.age_ticker = None;
                    cx.notify();
                })
                .ok();
            });
            (at, task)
        });
    }

    // Whether the shown tab has a countdown to keep current: the Cookies tab's Jar view
    // listing at least one cookie with an expiry.
    fn counting_down(&self) -> bool {
        self.tab == ResponseTab::Cookies
            && self.cookies_view == CookiesView::Jar
            && self.cookies.iter().any(|c| c.expires.is_some())
    }

    /// What the Cookies tab lists.
    pub fn cookies_view(&self) -> CookiesView {
        self.cookies_view
    }

    /// Shows `view` in the Cookies tab.
    pub fn set_cookies_view(&mut self, view: CookiesView, cx: &mut Context<Self>) {
        self.cookies_view = view;
        self.tick_expiry(cx);
        cx.notify();
    }

    /// Each Set-Cookie of the shown response as the This response table shows it, values
    /// unmasked.
    pub fn set_cookie_table(&self, cx: &App) -> Vec<SetCookieCells> {
        done(self.send.read(cx))
            .map(|r| set_cookie_cells(&r))
            .unwrap_or_default()
    }

    // Keep one redraw timer running exactly while a countdown is shown, so its labels
    // (computed at render) advance. A loop that finds nothing left to count clears itself.
    fn tick_expiry(&mut self, cx: &mut Context<Self>) {
        if !self.counting_down() {
            self.expiry_ticker = None;
            return;
        }
        if self.expiry_ticker.is_some() {
            return;
        }
        self.expiry_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(EXPIRY_TICK).await;
                let going = this.update(cx, |p, cx| {
                    cx.notify();
                    let going = p.counting_down();
                    if !going {
                        p.expiry_ticker = None;
                    }
                    going
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// The listed cookies as (name, value, domain, path).
    pub fn cookie_rows(&self) -> Vec<(String, String, String, String)> {
        self.cookies
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.value.clone(),
                    c.domain.clone(),
                    c.path.clone(),
                )
            })
            .collect()
    }

    /// The listed cookies as the table shows them, with expiry measured from `now`.
    pub fn cookie_table(&self, now: SystemTime) -> Vec<CookieCells> {
        self.cookies
            .iter()
            .map(|c| CookieCells {
                name: c.name.clone(),
                value: c.value.clone(),
                domain: c.domain.clone(),
                path: c.path.clone(),
                expires: expires_label(c.expires, now),
                http_only: c.http_only,
                secure: c.secure,
            })
            .collect()
    }

    /// Removes the `ix`th listed cookie from the shared jar and relists.
    pub fn delete_cookie(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let (Some(c), Some(exec)) = (self.cookies.get(ix), executor::shared(cx)) {
            exec.cookies().delete(&c.domain, &c.path, &c.name);
        }
        self.list_cookies(cx);
        self.tick_expiry(cx);
        cx.notify();
    }

    // Snapshot the jar's cookies for the final URL's host. Listing happens on a new result
    // or a delete, never per frame, so rendering never takes the jar's lock.
    fn list_cookies(&mut self, cx: &App) {
        let host = done(self.send.read(cx)).and_then(|r| {
            let url = url::Url::parse(r.response.as_ref()?.url()).ok()?;
            url.host_str().map(str::to_owned)
        });
        self.cookies = match (host, executor::shared(cx)) {
            (Some(host), Some(exec)) => exec
                .cookies()
                .list()
                .into_iter()
                .filter(|c| cookie_matches(&host, &c.domain, c.host_only))
                .collect(),
            _ => Vec::new(),
        };
    }

    /// The selected tab.
    pub fn tab(&self) -> ResponseTab {
        self.tab
    }

    // The focus handle of the shown tab `step` places from position `pos`, none past an end.
    fn neighbour_focus(&self, pos: usize, step: isize) -> Option<FocusHandle> {
        let to = pos.checked_add_signed(step)?;
        let tab = *self.strip_tabs().get(to)?;
        Some(self.tab_focus[tab as usize].clone())
    }

    /// The focus handle of `tab`'s strip tab.
    pub fn tab_focus(&self, tab: ResponseTab) -> FocusHandle {
        self.tab_focus[tab as usize].clone()
    }

    /// Selects `tab`.
    pub fn set_tab(&mut self, tab: ResponseTab, cx: &mut Context<Self>) {
        self.tab = tab;
        // A few event rows need more than a short pane keeps for them.
        if tab == ResponseTab::Events {
            self.make_room(style::EVENTS_PANE_WANTED, cx);
        }
        self.tick_expiry(cx);
        cx.notify();
    }

    /// Moves focus to the body, showing the Body tab first. A body that is not on screen
    /// (no response yet, an error, a dropped body) leaves nothing focusable inside, so the
    /// pane itself takes focus and the window never loses its owner.
    pub fn focus_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dropped = self
            .send
            .read(cx)
            .restored()
            .is_some_and(|r| r.body_dropped);
        let offered = matches!(self.send.read(cx).state(), SendState::Done(_))
            && self.error_view(cx).is_none()
            && !dropped;
        if !offered {
            self.focus.focus(window, cx);
            return;
        }
        if self.tab != ResponseTab::Body {
            self.set_tab(ResponseTab::Body, cx);
        }
        // The editor mode keeps the caret in its own input; the other modes use the body's
        // handle, which the rows list and the find chord share.
        let body = self.body.read(cx);
        match body.shown() {
            Shown::Editor => body.editor().read(cx).focus_handle(cx).focus(window, cx),
            _ => body.body_focus().clone().focus(window, cx),
        }
    }

    /// The Console tab's level filter; `None` shows every entry.
    pub fn console_filter(&self) -> Option<ConsoleLevel> {
        self.console_filter
    }

    /// Shows only entries of `filter`'s level in the Console tab, or all for `None`.
    pub fn set_console_filter(&mut self, filter: Option<ConsoleLevel>, cx: &mut Context<Self>) {
        self.console_filter = filter;
        cx.notify();
    }

    /// Whether the body view is on screen: the Body tab of a finished send without an error.
    pub fn body_shown(&self, cx: &App) -> bool {
        self.tab == ResponseTab::Body
            && matches!(self.send.read(cx).state(), SendState::Done(_))
            && self.error_view(cx).is_none()
    }

    /// "streaming" while a response is live, the end label once a stream response is done,
    /// `None` for anything else.
    pub fn stream_status(&self, cx: &App) -> Option<String> {
        let send = self.send.read(cx);
        match send.state() {
            SendState::Streaming => Some("streaming".to_owned()),
            SendState::Done(r) => r
                .response
                .as_ref()
                .and_then(|r| r.stream_end())
                .map(|e| live_stream::end_label(e).to_owned()),
            _ => None,
        }
    }

    /// The tail rows on screen while a response streams; empty otherwise.
    pub fn live_rows(&self, cx: &App) -> Vec<String> {
        self.send
            .read(cx)
            .live()
            .map(|l| (0..l.row_count()).filter_map(|ix| l.row(ix)).collect())
            .unwrap_or_default()
    }

    /// Lines that fell off the front of the live tail.
    pub fn live_skipped(&self, cx: &App) -> u64 {
        self.send.read(cx).live().map_or(0, |l| l.skipped_lines())
    }

    /// How many times this view has rendered; tests read it to check what a change redraws.
    #[doc(hidden)]
    pub fn renders(&self) -> usize {
        self.renders
    }

    /// The body view.
    pub fn body(&self) -> &Entity<BodyView> {
        &self.body
    }

    /// The status line of the received response, if any.
    pub fn summary(&self, cx: &App) -> Option<Summary> {
        let result = done(self.send.read(cx))?;
        let r = result.response.as_ref()?;
        Some(Summary {
            status: r.status(),
            reason: r.status_text().to_owned(),
            time: format_time(r.response_time()),
            size: format_size(r.size()),
            truncated: r.truncated(),
            saved: self.saved_labels(cx).map(|(label, _)| label),
        })
    }

    // The "From ..." label and its tooltip while a kept response is shown, in local time.
    fn saved_labels(&self, cx: &App) -> Option<(String, String)> {
        let restored = self.send.read(cx).restored()?;
        let now = last_response::wall_now(cx);
        let (label, tooltip) = last_response::local_labels(restored.saved_at, now);
        Some((format!("From {label}"), tooltip))
    }

    /// The count a tab shows: headers, listed cookies, assertion outcomes plus script test
    /// entries, or console entries of the shown result; `None` for Body and Timeline, or with
    /// no result.
    pub fn tab_count(&self, tab: ResponseTab, cx: &App) -> Option<usize> {
        if tab == ResponseTab::Events {
            return match self.send.read(cx).live() {
                Some(l) => Some(l.events().len() + l.dropped_events() as usize),
                None => self.shown_events(cx).map(|e| e.len()),
            };
        }
        let result = done(self.send.read(cx))?;
        match tab {
            ResponseTab::Headers => result.response.as_ref().map(|r| r.headers().len()),
            ResponseTab::Cookies => Some(match self.cookies_view {
                CookiesView::Response => {
                    (result.response.as_ref()).map_or(0, |r| r.set_cookies().count())
                }
                CookiesView::Jar => self.cookies.len(),
            }),
            ResponseTab::Tests => Some(result.assertions.len() + result.tests.len()),
            ResponseTab::Console => Some(result.console.len()),
            ResponseTab::Compare => self.comparison().and_then(|c| compare_view::badge(&c)),
            ResponseTab::Body | ResponseTab::Timeline | ResponseTab::Events => None,
        }
    }

    /// Whether any assertion of the shown result failed.
    pub fn assertions_failed(&self, cx: &App) -> bool {
        done(self.send.read(cx)).is_some_and(|r| {
            r.assertions
                .iter()
                .any(|o| matches!(o.verdict(), Verdict::Fail { .. }))
        })
    }

    /// Whether `tab`'s count badge takes the danger colour: only Tests, and only when an
    /// assertion failed or a script test failed or errored.
    pub fn count_is_danger(&self, tab: ResponseTab, cx: &App) -> bool {
        let tests_failed = || {
            done(self.send.read(cx)).is_some_and(|r| {
                r.tests
                    .iter()
                    .any(|t| matches!(test_line(t).0, "fail" | "error"))
            })
        };
        tab == ResponseTab::Tests && (self.assertions_failed(cx) || tests_failed())
    }

    /// The tone of `tab`'s count badge: danger as [`Self::count_is_danger`], warning when Tests
    /// passed with a check skipped, success when it has a pass and reads as "No tests" no more,
    /// else muted.
    pub fn count_tone(&self, tab: ResponseTab, cx: &App) -> Tone {
        if self.count_is_danger(tab, cx) {
            return Tone::Danger;
        }
        let passed = |r: &RequestResult| {
            r.assertions.iter().any(|o| assertion_line(o).0 == "pass")
                || r.tests.iter().any(|t| test_line(t).0 == "pass")
        };
        let skipped = |r: &RequestResult| {
            r.assertions
                .iter()
                .any(|o| assertion_line(o).0 == "skipped")
                || r.tests.iter().any(|t| test_line(t).0 == "skipped")
        };
        match done(self.send.read(cx)) {
            Some(r) if tab == ResponseTab::Tests && passed(&r) && !shows_no_tests(&r) => {
                if skipped(&r) {
                    Tone::Warning
                } else {
                    Tone::Success
                }
            }
            _ => Tone::Muted,
        }
    }

    /// The Tests badge's words: "{n} failed" when a check failed or a script errored, and
    /// "{n} passed, {m} skipped" when a pass sits beside skips; `None` keeps the plain number.
    pub fn badge_text(&self, tab: ResponseTab, cx: &App) -> Option<String> {
        let tone = self.count_tone(tab, cx);
        if !matches!(tone, Tone::Danger | Tone::Warning) {
            return None;
        }
        let c = check_counts(&*done(self.send.read(cx))?);
        Some(match tone {
            Tone::Danger => format!("{} failed", c.failed + c.errored),
            _ => format!("{} passed, {} skipped", c.passed, c.skipped),
        })
    }

    /// `tab`'s name in the strip's overflow menu: its label, followed by its badge in
    /// brackets when it shows one, as "Tests (1 failed)" or "Headers (12)".
    pub fn menu_label(&self, tab: ResponseTab, cx: &App) -> SharedString {
        let badge = (self.badge_text(tab, cx)).or_else(|| {
            (self.tab_count(tab, cx))
                .filter(|n| *n > 0)
                .map(|n| n.to_string())
        });
        match badge {
            Some(badge) => format!("{} ({badge})", tab.label()).into(),
            None => tab.label().into(),
        }
    }

    /// The "Cancelled" block: a calm note that keeps Retry when the host has one.
    fn cancelled_view(&self) -> ErrorView {
        ErrorView {
            kind: ErrorKind::Cancelled,
            title: "Cancelled",
            sentence: None,
            message: None,
            tone: ErrorTone::Neutral,
            retry: self.retry,
            add_variable: None,
            use_environments: Vec::new(),
        }
    }

    /// The error block's contents; `None` while idle, sending or fine. Retry needs both the
    /// error's kind to allow it and the host to offer it.
    pub fn error_view(&self, cx: &App) -> Option<ErrorView> {
        let mut view = match self.send.read(cx).state() {
            SendState::Idle | SendState::Sending | SendState::Streaming => return None,
            SendState::Cancelled => return Some(self.cancelled_view()),
            SendState::Failed(message) => describe_failure(message),
            // A cancel that lands before the run polls reports the request as Skipped.
            SendState::Done(r) if matches!(r.outcome, Outcome::Cancelled | Outcome::Skipped) => {
                return Some(self.cancelled_view());
            }
            // A failed subscription with a response keeps its body; `stream_error` says why.
            SendState::Done(r) if is_stream_error(r) => return None,
            SendState::Done(r) => self.done_error(r, cx)?,
        };
        view.retry &= self.retry;
        Some(view)
    }

    /// The banner under the tab strip for a subscription that failed with a response (a refused
    /// handshake's page, or a transcript its socket ended); `None` for anything else.
    pub fn stream_error(&self, cx: &App) -> Option<ErrorView> {
        let r = match self.send.read(cx).state() {
            SendState::Done(r) if is_stream_error(r) => r,
            _ => return None,
        };
        let mut view = self.done_error(r, cx)?;
        view.retry &= self.retry;
        Some(view)
    }

    // A finished run's error, redacted: a script error can quote a secret.
    fn done_error(&self, r: &RequestResult, cx: &App) -> Option<ErrorView> {
        let e = r.error.as_ref()?;
        let send = self.send.read(cx);
        let env =
            (send.root()).and_then(|root| send.workspace().read(cx).selected_environment(root, cx));
        let mut view = describe_error(e, &r.redact(&e.to_string()), env.as_deref());
        // The sentence quotes the error's own fields, such as a secret host.
        view.sentence = view.sentence.map(|s| r.redact(&s));
        Some(view)
    }

    // The one-row banner for `stream_error`: icon, title, sentence and a Retry when offered.
    fn stream_error_row(&self, cx: &App) -> Option<AnyElement> {
        let view = self.stream_error(cx)?;
        let sentence = view.sentence.clone().unwrap_or_default();
        Some(
            h_flex()
                .id("response-stream-error")
                .test_support()
                .aria_label(format!("{}: {sentence}", view.title))
                .w_full()
                .gap(style::SPACE_2)
                .items_center()
                .child(Icon::new(IconName::CircleAlert).text_color(cx.theme().danger))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(cx.theme().danger)
                        .child(view.title),
                )
                // The sentence is cut to one line; its tooltip holds the whole text.
                .child(
                    div()
                        .id("response-stream-error-sentence")
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_size(style::TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .tooltip(ui::text_tooltip(
                            "response-stream-error-tooltip",
                            sentence.clone(),
                        ))
                        .child(sentence),
                )
                .when(view.retry, |row| row.child(retry_button(cx)))
                .into_any_element(),
        )
    }

    // The "Ask again" button under a "scripts not executed" notice while the collection is
    // blocked: it clears the decision so the next send asks the trust question afresh.
    fn ask_again_button(&self, result: &RequestResult, cx: &mut Context<Self>) -> Option<Button> {
        if !result
            .warnings
            .contains(&apinata_core::Warning::ScriptsNotRun)
        {
            return None;
        }
        let root = self.send.read(cx).root()?.to_path_buf();
        let state = cx.try_global::<AppState>()?;
        // From the approvals model, never the disk: render stays free of file reads.
        let approvals = state.approvals.clone();
        (approvals.read(cx).script_decision(&root) == Some(apinata_core::ScriptDecision::Blocked))
            .then(|| {
                Button::new("response-warning-ask-again")
                    .ghost()
                    .label("Ask again")
                    .on_click(cx.listener(move |_, _, _, cx| {
                        if crate::script_gate::ask_again(&approvals, &root, cx) {
                            cx.notify();
                        }
                    }))
            })
    }

    /// The result's warnings, then each skipped post-response variable, as displayed: in
    /// sentence case, with core's backticks as plain quotes; the CLI and MCP keep core's text.
    pub fn warnings(&self, cx: &App) -> Vec<String> {
        done(self.send.read(cx))
            .map(|r| {
                let skipped = (r.post_response_vars.iter())
                    .filter(|o| matches!(o.result(), apinata_core::VarResult::Skipped { .. }));
                (r.warnings.iter().map(ToString::to_string))
                    .chain(skipped.map(|o| ui::plain_quotes(&o.to_string())))
                    .map(|w| sentence_case(&w))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One title per hop: "<method> <url> <status>".
    pub fn hop_titles(&self, cx: &App) -> Vec<String> {
        done(self.send.read(cx))
            .and_then(|r| {
                let response = r.response.as_ref()?;
                Some(
                    response
                        .timeline()
                        .iter()
                        .map(|h| format!("{} {} {}", h.method, h.url, h.status))
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    /// Each hop's negotiated version as shown, in timeline order; None where it is unknown.
    pub fn hop_versions(&self, cx: &App) -> Vec<Option<String>> {
        done(self.send.read(cx))
            .and_then(|r| {
                let response = r.response.as_ref()?;
                Some(
                    response
                        .timeline()
                        .iter()
                        .map(|h| h.version().map(|v| v.to_string()))
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    /// Each hop's peer as shown (`127.0.0.1:8080`, `proxy host:port`), in timeline order; None
    /// where the address is unknown.
    pub fn hop_peers(&self, cx: &App) -> Vec<Option<String>> {
        done(self.send.read(cx))
            .and_then(|r| {
                let response = r.response.as_ref()?;
                Some(response.timeline().iter().map(|h| h.peer_label()).collect())
            })
            .unwrap_or_default()
    }

    /// `Host override: <name -> address>` for each hop a host override mapped.
    pub fn hop_override_lines(&self, cx: &App) -> Vec<String> {
        done(self.send.read(cx))
            .and_then(|r| {
                let response = r.response.as_ref()?;
                Some(
                    (response.timeline().iter())
                        .filter_map(override_line)
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    /// `PAC: <route or cause>` for each hop that went through PAC, in timeline order.
    pub fn hop_pac_lines(&self, cx: &App) -> Vec<String> {
        done(self.send.read(cx))
            .and_then(|r| {
                let response = r.response.as_ref()?;
                Some(
                    (response.timeline().iter())
                        .filter_map(|h| pac_line(h, &r))
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    // Where `tab` sits in the strip; the ids keep `ResponseTab::ALL` positions.
    fn tab_position(&self, tab: ResponseTab) -> usize {
        self.strip_tabs()
            .iter()
            .position(|t| *t == tab)
            .unwrap_or(0)
    }

    // The strip position of the tab holding focus, if one does.
    fn focused_tab_position(&self, window: &Window) -> Option<usize> {
        self.strip_tabs()
            .iter()
            .position(|t| self.tab_focus[*t as usize].is_focused(window))
    }

    // The tabs the strip lists: the pane's own, then Compare while a source is set.
    fn strip_tabs(&self) -> Rc<[ResponseTab]> {
        let events = self.events_tab.then_some(ResponseTab::Events);
        if self.streaming {
            return [ResponseTab::Body].into_iter().chain(events).collect();
        }
        let extra = self.compare.is_some().then_some(ResponseTab::Compare);
        let mut tabs: Vec<ResponseTab> = self.tabs.iter().copied().chain(extra).collect();
        // Events sits right after Body, where a reader of a stream looks first.
        if let (Some(events), Some(at)) =
            (events, tabs.iter().position(|t| *t == ResponseTab::Body))
        {
            tabs.insert(at + 1, events);
        }
        tabs.into()
    }

    fn tab_bar(&self, window: &Window, cx: &mut Context<Self>) -> TabBar {
        let this = cx.entity();
        let shown = self.strip_tabs();
        let selected = self.tab_position(self.tab);
        let tabs = shown.iter().enumerate().map(|(pos, &t)| {
            let ix = t as usize;
            let words = self.badge_text(t, cx);
            let badge =
                self.tab_count(t, cx)
                    .and_then(|n| match (self.count_tone(t, cx), &words) {
                        (Tone::Danger, Some(text)) => Some(ui::danger_text_badge(text.clone())),
                        (Tone::Danger, None) => ui::danger_count_badge(n),
                        (Tone::Success, _) => ui::success_count_badge(n),
                        (Tone::Warning, Some(text)) => Some(ui::warning_text_badge(text.clone())),
                        (Tone::Warning, None) => ui::warning_count_badge(n),
                        (Tone::Muted, _) => ui::count_badge(n),
                    });
            let count = badge.map(|b| {
                div()
                    .id(("response-count", ix))
                    .test_support()
                    .when_some(words, |d, text| d.aria_label(text))
                    .child(b)
            });
            // Each tab is its own Tab stop, as in the request's section strip: Enter or Space
            // opens it, and keyboard focus draws an outline over its padded box.
            let focus = &self.tab_focus[ix];
            let pane = this.clone();
            Tab::new().child(
                h_flex()
                    .relative()
                    .track_focus(focus)
                    .on_key_down(move |event, window, cx| {
                        let key = event.keystroke.key.as_str();
                        if event.keystroke.modifiers.modified() {
                            return;
                        }
                        if matches!(key, "enter" | "space") {
                            cx.stop_propagation();
                            pane.update(cx, |p, cx| p.set_tab(t, cx));
                        } else if let Some(step) = crate::ui::arrow_step(key) {
                            cx.stop_propagation();
                            let next = pane.read(cx).neighbour_focus(pos, step);
                            if let Some(next) = next {
                                window.focus(&next, cx);
                            }
                        }
                    })
                    .line_height(relative(style::TAB_LINE_HEIGHT))
                    .px(style::SPACE_0)
                    .gap(style::SPACE_1)
                    .items_center()
                    .child(
                        div()
                            .id(("response-tab", ix))
                            .test_support()
                            .child(t.label()),
                    )
                    .children(count)
                    .when(
                        focus.is_focused(window) && crate::focus_mode::is_keyboard(cx),
                        |d| {
                            d.child(
                                div()
                                    .id(("response-tab-focus", ix))
                                    .test_support()
                                    .absolute()
                                    .inset_0()
                                    .border_1()
                                    .border_color(cx.theme().ring),
                            )
                        },
                    ),
            )
        });
        // The menu shows only while last frame's layout overflowed; the strip's watch
        // redraws once when this frame's layout disagrees.
        let pane = this.clone();
        let picked = Rc::clone(&shown);
        // The strip clips flush with the pane's edge, so a spacer leaves the first tab's ring
        // whole and lines it up with the request column's content.
        ui::tab_strip("response-tabs", cx)
            .prefix(div().w(style::FOCUS_RING_ROOM).flex_none())
            .track_scroll(&self.tab_scroll)
            .selected_index(selected)
            .children(tabs)
            .when(ui::strip_overflows(&self.tab_scroll), |bar| {
                let labels = shown.iter().map(|t| self.menu_label(*t, cx)).collect();
                bar.suffix(ui::strip_overflow(
                    "response-tab-overflow",
                    labels,
                    self.tab_position(self.tab),
                    {
                        let shown = Rc::clone(&shown);
                        move |ix, _, cx| pane.update(cx, |p, cx| p.set_tab(shown[ix], cx))
                    },
                ))
            })
            .on_click(move |ix, _, cx| this.update(cx, |p, cx| p.set_tab(picked[*ix], cx)))
    }

    // The Cookies tab: the view switch and filter over what this response set, or the jar.
    fn cookies_list(&self, result: &Arc<RequestResult>, cx: &mut Context<Self>) -> AnyElement {
        match self.cookies_view {
            CookiesView::Response => self.set_cookies_list(result, cx),
            CookiesView::Jar => self.jar_list(result, cx),
        }
    }

    // The two-way switch between this response's Set-Cookie rows and the jar.
    fn cookies_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = cx.entity().downgrade();
        let selected = match self.cookies_view {
            CookiesView::Response => 0,
            CookiesView::Jar => 1,
        };
        ui::segmented(
            "cookies-view",
            ["This response", "Jar"],
            selected,
            move |ix, _, cx| {
                let view = if *ix == 0 {
                    CookiesView::Response
                } else {
                    CookiesView::Jar
                };
                pane.update(cx, |p, cx| p.set_cookies_view(view, cx)).ok();
            },
        )
        .ids(["cookies-view-response", "cookies-view-jar"])
    }

    // The tab's top row: the view switch, then the filter bar.
    fn cookies_bar(&self, bar: Div, cx: &mut Context<Self>) -> Div {
        h_flex()
            .gap(style::SPACE_2)
            .items_center()
            .w_full()
            .pl(style::FOCUS_RING_ROOM)
            .child(self.cookies_switch(cx))
            .child(div().flex_1().min_w_0().child(bar))
    }

    // An empty Cookies view: the top row stays, so the switch is always reachable.
    fn cookies_empty(
        &self,
        bar: Div,
        id: &'static str,
        text: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .id("response-cookies")
            .test_support()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_2()
            .child(self.cookies_bar(bar, cx))
            .child(div().id(id).test_support().aria_label(text).child(text))
            .into_any_element()
    }

    // Every Set-Cookie of this response, hop by hop, with what the jar did with it. Values
    // show masked unless revealed; a rejected row's result is muted.
    fn set_cookies_list(&self, result: &Arc<RequestResult>, cx: &mut Context<Self>) -> AnyElement {
        let query = self.cookies_filter.read(cx).value().to_string();
        let (table, eyes): (Vec<SetCookieCells>, Vec<Option<bool>>) =
            self.shown_set_cookies(result).into_iter().unzip();
        let shown = set_cookie_shown(&table, &query);
        let bar = self.filter_bar(
            (&self.cookies_filter, "cookies-filter", "cookies-copy"),
            ("Filter cookies", "Copy shown cookies"),
            !shown.is_empty(),
            |p, cx| {
                let filter = p.cookies_filter.read(cx).value().to_string();
                done(p.send.read(cx))
                    .map(|r| {
                        let rows: Vec<SetCookieCells> = (p.shown_set_cookies(&r).into_iter())
                            .map(|(c, _)| c)
                            .collect();
                        set_cookie_lines(&rows, &filter)
                    })
                    .unwrap_or_default()
            },
            "Copied the cookies",
            cx,
        );
        // A kept result has no timeline, so nothing says what its hops set.
        let recorded = (result.response.as_ref()).is_some_and(|r| !r.timeline().is_empty());
        if !recorded {
            let text = "Cookies were not recorded for this result.";
            return self.cookies_empty(bar, "set-cookies-unrecorded", text, cx);
        }
        if table.is_empty() {
            return self.cookies_empty(
                bar,
                "set-cookies-none",
                "This response set no cookies.",
                cx,
            );
        }
        const COLUMNS: [(&str, &str); 4] = [
            ("set-cookie-name", "Name"),
            ("set-cookie-value", "Value"),
            ("set-cookie-hop", "Hop"),
            ("set-cookie-result", "Result"),
        ];
        let keys: Vec<(usize, usize)> = shown.iter().map(|(_, c)| c.key).collect();
        let flags: Vec<Option<bool>> = shown.iter().map(|(ix, _)| eyes[*ix]).collect();
        // While every shown value is masked, Value only needs room for the mask, so Hop takes
        // the free width instead; a revealed value takes it back.
        let revealed = flags.contains(&Some(true));
        let masked_value = style::SET_COOKIE_MASKED_VALUE_WIDTH;
        let columns = COLUMNS
            .iter()
            .enumerate()
            .map(|(i, (cell, heading))| {
                let head = ("set-cookies-head-cell", i);
                let fixed = |w| KvColumn::fixed(head, cell, *heading, w).tip_when_cut();
                match i {
                    1 if revealed => KvColumn::flex(head, cell, *heading),
                    1 => KvColumn::fixed(head, cell, *heading, masked_value),
                    2 if !revealed => KvColumn::flex(head, cell, *heading).tip_when_cut(),
                    0 => fixed(style::SET_COOKIE_FIXED_WIDTHS[0]),
                    _ => fixed(style::SET_COOKIE_FIXED_WIDTHS[i - 1]),
                }
            })
            .collect();
        let values: Vec<String> = shown.iter().map(|(_, c)| c.value.clone()).collect();
        let none_shown = shown.is_empty();
        let rows = shown
            .into_iter()
            .map(|(_, c)| {
                let result: SharedString = c.result.clone().into();
                vec![
                    KvCell::Text(c.name.clone().into()),
                    KvCell::Text(c.value.clone().into()),
                    KvCell::Text(c.hop.clone().into()),
                    if c.rejected {
                        KvCell::Muted(result)
                    } else {
                        KvCell::Text(result)
                    },
                ]
            })
            .collect();
        let revealer = cx.entity().downgrade();
        let view = ui::kv_view("set-cookies-table", columns, rows)
            .row_ids("set-cookie")
            .reveal_column(1)
            .on_reveal(
                ("set-cookie-eye", "set-cookie-reveal", "set-cookie-hide"),
                flags,
                move |row, _, cx| {
                    revealer
                        .update(cx, |p, cx| p.toggle_set_cookie_reveal(keys[row], cx))
                        .ok();
                },
            )
            .on_copy("set-cookie-copy", "Copy value", move |ix, window, cx| {
                copy_with_toast(values[ix].clone(), "Copied the value", window, cx)
            });
        v_flex()
            .id("response-cookies")
            .test_support()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_2()
            .child(self.cookies_bar(bar, cx))
            .child(if none_shown {
                no_match("set-cookies-no-match", "cookies").into_any_element()
            } else {
                // As in the jar view, a narrow pane scrolls the head and rows sideways together
                // instead of squeezing Value away.
                div()
                    .id("set-cookies-hscroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_x_scroll()
                    .child(
                        v_flex()
                            .size_full()
                            .min_w(style::SET_COOKIE_TABLE_MIN_WIDTH)
                            .gap_2()
                            .child(view.head_row("set-cookies-head"))
                            .child(scroll_frame("set-cookies-rows", v_flex().child(view))),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    // The jar's cookies for the final host: a headed read-only table whose head stays above
    // the scrolling rows; each row's delete shows on hover.
    fn jar_list(&self, result: &Arc<RequestResult>, cx: &mut Context<Self>) -> AnyElement {
        const COLUMNS: [(&str, &str); 7] = [
            ("cookie-name", "Name"),
            ("cookie-value", "Value"),
            ("cookie-domain", "Domain"),
            ("cookie-path", "Path"),
            ("cookie-expires", "Expires"),
            ("cookie-http-only", "HttpOnly"),
            ("cookie-secure", "Secure"),
        ];
        let columns = COLUMNS
            .iter()
            .enumerate()
            .map(|(i, (cell, heading))| {
                let head = ("cookies-head-cell", i);
                match i {
                    1 => KvColumn::flex(head, cell, *heading),
                    // Value is the flexible column, so the fixed widths skip index 1.
                    _ => {
                        let fixed = if i == 0 { 0 } else { i - 1 };
                        KvColumn::fixed(head, cell, *heading, style::COOKIE_FIXED_WIDTHS[fixed])
                    }
                }
            })
            .collect();
        let query = self.cookies_filter.read(cx).value().to_string();
        // Values show masked unless revealed, and the filter and the copies read what is shown.
        let (table, eyes): (Vec<CookieCells>, Vec<Option<bool>>) = self
            .shown_cookies(result, SystemTime::now())
            .into_iter()
            .unzip();
        let shown = cookie_shown(&table, &query);
        if self.cookies.is_empty() {
            let bar = self.jar_filter_bar(false, cx);
            return self.cookies_empty(bar, "cookies-none", "No cookies for this host.", cx);
        }
        let originals: Vec<usize> = shown.iter().map(|(ix, _)| *ix).collect();
        let values: Vec<String> = shown.iter().map(|(_, c)| c.value.clone()).collect();
        let flags: Vec<Option<bool>> = originals.iter().map(|&ix| eyes[ix]).collect();
        let keys: Vec<(String, String, String)> = (shown.iter())
            .map(|(_, c)| (c.name.clone(), c.domain.clone(), c.path.clone()))
            .collect();
        let rows = shown
            .into_iter()
            .map(|(_, c)| {
                vec![
                    KvCell::Text(c.name.clone().into()),
                    KvCell::Text(c.value.clone().into()),
                    KvCell::Text(c.domain.clone().into()),
                    KvCell::Text(c.path.clone().into()),
                    KvCell::Text(c.expires.clone().into()),
                    KvCell::Flag(c.http_only),
                    KvCell::Flag(c.secure),
                ]
            })
            .collect();
        let any_shown = !originals.is_empty();
        let none_shown = !any_shown && !query.is_empty();
        let pane = cx.entity().downgrade();
        let revealer = pane.clone();
        let view = ui::kv_view("cookies-table", columns, rows)
            .row_ids("cookie")
            .reveal_column(1)
            .on_reveal(
                ("cookie-eye", "cookie-reveal", "cookie-hide"),
                flags,
                move |row, _, cx| {
                    revealer
                        .update(cx, |p, cx| p.toggle_cookie_reveal(keys[row].clone(), cx))
                        .ok();
                },
            )
            .on_copy("cookie-copy", "Copy value", move |ix, window, cx| {
                copy_with_toast(values[ix].clone(), "Copied the value", window, cx)
            })
            .on_delete("cookie-delete", "Delete cookie", move |ix, _, cx| {
                // The row index counts shown rows; the jar is addressed by the full list.
                pane.update(cx, |p, cx| p.delete_cookie(originals[ix], cx))
                    .ok();
            });
        let bar = self.jar_filter_bar(any_shown, cx);
        v_flex()
            .id("response-cookies")
            .test_support()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_2()
            .child(self.cookies_bar(bar, cx))
            .child(if none_shown {
                no_match("cookies-none", "cookies").into_any_element()
            } else {
                // Seven columns need more than a narrow pane has: the head and rows scroll
                // sideways together instead of squeezing the Value column away.
                div()
                    .id("cookies-hscroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_x_scroll()
                    .child(
                        v_flex()
                            .size_full()
                            .min_w(style::COOKIE_TABLE_MIN_WIDTH)
                            .gap_2()
                            .child(view.head_row("cookies-head"))
                            .child(scroll_frame("cookies-rows", v_flex().child(view))),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    // The Jar view's filter bar; Copy copies the shown jar rows.
    fn jar_filter_bar(&self, any_shown: bool, cx: &mut Context<Self>) -> Div {
        self.filter_bar(
            (&self.cookies_filter, "cookies-filter", "cookies-copy"),
            ("Filter cookies", "Copy shown cookies"),
            any_shown,
            |p, cx| {
                let filter = p.cookies_filter.read(cx).value().to_string();
                done(p.send.read(cx))
                    .map(|r| {
                        let cookies: Vec<CookieCells> = (p.shown_cookies(&r, SystemTime::now()))
                            .into_iter()
                            .map(|(c, _)| c)
                            .collect();
                        cookie_lines(&cookies, &filter)
                    })
                    .unwrap_or_default()
            },
            "Copied the cookies",
            cx,
        )
    }

    // The Headers tab: the filter bar over the rows it keeps; each row has a copy button.
    fn headers_list(&self, result: &Arc<RequestResult>, cx: &mut Context<Self>) -> AnyElement {
        let query = self.headers_filter.read(cx).value().to_string();
        // Values show masked unless revealed, and the filter and the copies read what is shown.
        let rows = self.header_rows(result, cx);
        let all: Vec<(String, String)> = (rows.iter())
            .map(|(name, value, ..)| (name.clone(), value.clone()))
            .collect();
        let shown = header_shown(&all, &query);
        let originals: Vec<usize> = shown.iter().map(|(ix, _)| *ix).collect();
        let flags: Vec<Option<bool>> = originals.iter().map(|&ix| rows[ix].2).collect();
        let locked: Vec<bool> = originals.iter().map(|&ix| rows[ix].3).collect();
        let values: Vec<String> = shown.iter().map(|(_, p)| p.1.clone()).collect();
        let kept: Vec<(String, String)> = shown.into_iter().map(|(_, p)| p.clone()).collect();
        let none_kept = kept.is_empty();
        let pane = cx.entity();
        let table = header_table(["header", "header-name", "header-value"], &kept)
            .headed("headers-head")
            .on_reveal(
                ("header-eye", "header-reveal", "header-hide"),
                flags,
                move |row, _, cx| {
                    pane.update(cx, |p, cx| {
                        p.toggle_reveal((0, Field::Header, originals[row]), cx)
                    });
                },
            )
            .lock_eyes(locked, SCRUBBED_TIP)
            .on_copy("header-copy", "Copy value", move |ix, window, cx| {
                copy_with_toast(values[ix].clone(), "Copied the value", window, cx)
            });
        let bar = self.filter_bar(
            (&self.headers_filter, "headers-filter", "headers-copy"),
            ("Filter headers", "Copy shown headers"),
            !none_kept,
            |p, cx| {
                let filter = p.headers_filter.read(cx).value().to_string();
                done(p.send.read(cx))
                    .map(|r| {
                        let shown: Vec<(String, String)> = (p.header_rows(&r, cx).into_iter())
                            .map(|(name, value, ..)| (name, value))
                            .collect();
                        header_lines(&shown, &filter)
                    })
                    .unwrap_or_default()
            },
            "Copied the headers",
            cx,
        );
        let body = if none_kept && !query.is_empty() {
            scroll_frame(
                "response-headers",
                v_flex().child(no_match("headers-none", "headers")),
            )
        } else {
            scroll_frame("response-headers", v_flex().child(table))
        };
        v_flex()
            .id("response-headers-tab")
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_2()
            .child(bar)
            .child(body)
            .into_any_element()
    }

    // A filter field beside a Copy button. Copy builds its text from the pane when clicked, so
    // a render never does; the field takes the width the button leaves.
    fn filter_bar(
        &self,
        (input, filter_id, copy_id): (&Entity<InputState>, &'static str, &'static str),
        (filter_label, copy_label): (&'static str, &'static str),
        any_shown: bool,
        text: fn(&ResponsePane, &App) -> String,
        toast: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let pane = cx.entity();
        let copy = Button::new(copy_id)
            .ghost()
            .small()
            .label("Copy")
            .accessibility_label(copy_label)
            .tooltip(copy_label)
            .lock(!any_shown, cx)
            .on_click(move |_, window, cx| {
                let body = text(pane.read(cx), cx);
                copy_with_toast(body, toast, window, cx)
            });
        h_flex()
            .gap(style::SPACE_2)
            .items_center()
            .w_full()
            // The focus ring is drawn outside the Copy button; keep it inside the pane.
            .pr(style::SPACE_1)
            .child(
                div()
                    .id(filter_id)
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .aria_label(filter_label)
                    .child(ui::padded_input(
                        Input::new(input)
                            .id(SharedString::from(format!("{filter_id}-input")))
                            .bg(ui::field_fill(false, cx)),
                        input,
                    )),
            )
            .child(copy)
    }

    // The right end of the header row: the status pill, then muted time and size, a note
    // when the body was cut at the size limit, and when a kept response was received.
    fn status_line(
        &self,
        result: &RequestResult,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let r = result.response.as_ref()?;
        let muted = |id: &'static str, text: String| {
            div()
                .id(id)
                .test_support()
                .flex_none()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .aria_label(text.clone())
                .child(text)
        };
        // The size and the cut point share one unit; a compressed body's cut is far below its
        // decoded size.
        let (size_text, cut_text) = size_labels(r.size(), r.body().len() as u64, r.truncated());
        let truncated =
            cut_text.map(|cut| muted("response-truncated", format!("truncated at {cut}")));
        // A kept response shows its date (a narrow pane leaves it out) and a short age in a
        // box as wide as the widest age, so a growing age never reflows the row. The long
        // label is the aria label and joins the tooltip.
        let compact = self.compact.get();
        let now = last_response::wall_now(cx);
        let saved = self.send.read(cx).restored().map(|kept| {
            self.note_age(kept.saved_at, now);
            let (label, tip) = self.saved_labels(cx).unwrap_or_default();
            let tip = format!("{label}\n{tip}");
            let age = last_response::short_age(kept.saved_at, now);
            let date = (!compact).then(|| {
                div()
                    .id("response-saved-date")
                    .test_support()
                    .child(format!("From {}", last_response::local_date(kept.saved_at)))
            });
            // Tabular digits make every count of one form as wide as its candidate here.
            let widest = [
                "just now", "59m ago", "23h ago", "29d ago", "12mo ago", "99y ago",
            ];
            let age_box = v_flex()
                .flex_none()
                .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                .child(div().flex_none().child(age))
                .child(
                    v_flex()
                        .h_0()
                        .invisible()
                        .children(widest.map(|w| div().flex_none().child(w))),
                );
            h_flex()
                .id("response-saved")
                .test_support()
                .gap_1()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .aria_label(label)
                .children(date)
                .child(age_box)
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        });
        // The address is the one fact that gives way: it truncates first when the row is short,
        // and the tooltip keeps the whole peer.
        let peer = r.peer_label().filter(|_| !compact);
        let address = peer.map(|peer| {
            let tip = SharedString::from(peer.clone());
            div()
                .id("response-address")
                .test_support()
                .relative()
                .min_w_0()
                .flex_shrink(1.)
                .max_w(style::RESPONSE_ADDRESS_MAX)
                .truncate()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .aria_label(peer.clone())
                .child(peer)
                .tooltip(ui::text_tooltip("response-address-tooltip", tip))
        });
        self.min.facts_shown(self.facts_key(result, cx));
        // The actions sit as one tight group, as the request toolbar's adjacent buttons do.
        let actions = h_flex()
            .flex_none()
            .gap(style::SPACE_1)
            .items_center()
            .children(
                (self.save_example && !compact)
                    .then(|| save_example_button(r, self.example_blocked(cx), cx)),
            )
            .children((self.save_example && !compact).then(|| export_har_button(r, cx)))
            .children(self.history_button(cx))
            .children(self.compare_button(cx))
            .when(!compact, |row| row.children(self.latest_button(cx)))
            .when(compact, |row| row.child(self.more_menu(r, cx)));
        let mut before: Vec<AnyElement> = vec![
            div()
                .id("response-status")
                .test_support()
                .relative()
                .flex_none()
                .child(ui::status_pill(r.status(), r.status_text()))
                .child(width_probe(&self.min.status))
                .into_any_element(),
        ];
        if self.elapsed {
            before.push(muted("response-time", format_time(r.response_time())).into_any_element());
        }
        before.extend(self.timing_bar(r, cx));
        before.push(muted("response-size", size_text).into_any_element());
        let mut after: Vec<AnyElement> = r
            .stream_end()
            .map(|e| muted("response-stream-end", live_stream::end_label(e).to_owned()))
            .into_iter()
            .chain(truncated)
            .map(IntoElement::into_any_element)
            .collect();
        after.extend(saved.map(IntoElement::into_any_element));
        after.push(actions.into_any_element());
        // Wide, the facts either side of the address are two fixed groups whose probed widths
        // are the row's floor with the address cut away. Compact, they wrap one by one.
        let (before, after) = match compact {
            true => (before, after),
            false => {
                self.min.addressed.set(address.is_some());
                let group = |items: Vec<AnyElement>, cell: &Rc<Cell<f32>>| {
                    h_flex()
                        .relative()
                        .flex_none()
                        .gap(style::SPACE_2)
                        .items_center()
                        .children(items)
                        .child(rem_probe(cell))
                        .into_any_element()
                };
                (
                    vec![group(before, &self.min.facts_before)],
                    vec![group(after, &self.min.facts_after)],
                )
            }
        };
        Some(
            // Wide, one line whose address truncates; compact, it wraps on a row of its own.
            h_flex()
                .when(compact, |row| row.flex_wrap().w_full())
                .when(!compact, |row| row.min_w(self.min.facts(window)))
                .justify_end()
                .gap(style::SPACE_2)
                .items_center()
                .children(before)
                // Nothing when the peer is unknown; a narrow pane leaves it to the Timeline.
                .children(address)
                .children(after)
                .into_any_element(),
        )
    }

    // A thin bar split by the measured phases of the final hop, hover for the breakdown.
    // Absent when that hop was not timed (NTLM, a response loaded from history).
    fn timing_bar(&self, r: &Response, cx: &App) -> Option<AnyElement> {
        let phases = Phases::from(r.timeline().last()?.timing.as_ref()?);
        let segs = timing_bar::segments(&phases);
        if segs.is_empty() {
            return None;
        }
        let tip = timing_bar::tooltip_lines(&phases).join("\n");
        let theme = cx.theme();
        let parts = segs.into_iter().map(|seg| {
            let (id, colour) = match seg.kind {
                PhaseKind::Dns => ("response-timing-dns", theme.chart_1),
                PhaseKind::Connect => ("response-timing-connect", theme.chart_2),
                PhaseKind::Wait => ("response-timing-wait", theme.chart_3),
                PhaseKind::Download => ("response-timing-download", theme.chart_4),
            };
            div()
                .id(id)
                .test_support()
                .h_full()
                .w(relative(seg.fraction))
                .bg(colour)
        });
        Some(
            h_flex()
                .id("response-timing")
                .test_support()
                .flex_none()
                .w(style::TIMING_BAR_WIDTH)
                .h(style::TIMING_BAR_HEIGHT)
                .rounded_full()
                .overflow_hidden()
                .bg(theme.border)
                .children(parts)
                .tooltip(ui::text_tooltip("response-timing-tooltip", tip))
                .into_any_element(),
        )
    }

    // Why the request this pane shows cannot hold examples, per `save_example::offered`;
    // `None` when it can, or when the pane has no request to ask about.
    fn example_blocked(&self, cx: &App) -> Option<&'static str> {
        let (root, item) = self.target.as_ref().and_then(|target| target(cx))?;
        if item.is_draft() {
            return Some(save_example::DRAFT_BLOCKED);
        }
        let workspace = self.send.read(cx).workspace().read(cx);
        let view = workspace.request(&root, &item, cx)?;
        save_example::blocked(&view)
    }

    // The "Previous runs" dropdown of kept sends, newest first, with the shown one checked.
    fn history_button(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.history.is_empty() {
            return None;
        }
        let viewing = self.send.read(cx).viewing();
        let sends: Rc<Vec<KeptSend>> = Rc::new(self.history.clone());
        let muted = cx.theme().muted_foreground;
        let pane = cx.entity().downgrade();
        let button = icon_dropdown(
            "response-history",
            AssetIcon::RotateCcwClock,
            "Previous runs",
            cx,
        )
        .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, cx| {
            // The menu is built once as it opens, so its ages are read then; a closed
            // menu draws no age and keeps no timer armed.
            let now = last_response::wall_now(cx);
            menu = crate::ui::checks_right(menu.scrollable(true));
            for (ix, send) in sends.iter().enumerate() {
                let (seq, line) = (send.seq, history_row(send, now));
                let pane = pane.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        // The target gives way to the summary, which always shows whole.
                        let label = match line.target.is_empty() {
                            true => line.summary.clone(),
                            false => format!("{} · {}", line.target, line.summary),
                        };
                        let tip = line.tip.clone();
                        h_flex()
                            .id(("history-row", ix))
                            .test_support()
                            .accessibility_id(format!("history-row-{ix}"))
                            .aria_label(label)
                            // Filling the menu's width puts every row's check at one edge.
                            .flex_1()
                            .max_w(style::HISTORY_ROW_WIDTH)
                            .min_w_0()
                            .gap(style::SPACE_2)
                            .when(!line.target.is_empty(), |d| {
                                d.child(div().min_w_0().truncate().child(line.target.clone()))
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(muted)
                                    .child(line.summary.clone()),
                            )
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })
                    .checked(viewing == Some(seq))
                    .on_click(move |_, _, cx| {
                        let _ = pane.update(cx, |p, cx| p.show_history(seq, cx));
                    }),
                );
            }
            menu
        });
        Some(tipped("response-history", "Previous runs".into(), button))
    }

    // The error block: an icon in the view's tone, the headline, the sentence, details
    // behind a toggle, and when the view offers it a Retry that sends through the key's action.
    fn error_element(&self, view: &ErrorView, cx: &mut Context<Self>) -> AnyElement {
        let (icon, colour) = match view.tone {
            ErrorTone::Neutral => (IconName::Ban, cx.theme().muted_foreground),
            ErrorTone::Danger => (IconName::CircleAlert, cx.theme().danger),
        };
        let open = self.details_open;
        let toggle = view.message.as_ref().map(|_| {
            Button::new("response-error-details-toggle")
                .ghost()
                .small()
                .label(if open { "Hide details" } else { "Show details" })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.details_open = !this.details_open;
                    cx.notify();
                }))
        });
        let details = view.message.clone().filter(|_| open).map(|text| {
            div()
                .id("response-error-details")
                .test_support()
                .max_w_full()
                .p_2()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(style::RADIUS)
                .text_size(style::TEXT_SMALL)
                .font_family(cx.theme().mono_font_family.clone())
                .child(SelectableText::new("response-error-details-text", text))
        });
        // The outer box scrolls; the inner column is at least as tall as it, so the content
        // centres when it fits and starts at the top, icon included, when it does not.
        let column = v_flex()
            .w_full()
            .min_h_full()
            .items_center()
            .justify_center()
            .gap(style::SPACE_2)
            .child(
                div()
                    .id("response-error-icon")
                    .test_support()
                    .child(Icon::new(icon).large().text_color(colour)),
            )
            .child(
                div()
                    .when(view.tone == ErrorTone::Danger, |d| d.text_color(colour))
                    .child(view.title),
            )
            .children(view.sentence.clone().map(|sentence| {
                div()
                    .text_size(style::TEXT_SMALL)
                    .text_color(cx.theme().muted_foreground)
                    .child(sentence)
            }))
            .children(toggle)
            .children(details)
            .child(
                h_flex()
                    .gap(style::SPACE_2)
                    .children(view.retry.then(|| retry_button(cx)))
                    .children(self.switch_env_buttons(view, cx))
                    .children(self.add_env_button(view, cx)),
            )
            .children(self.error_cookies_list(cx));
        div()
            .id("response-error")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .child(column)
            .into_any_element()
    }

    // Under a failed send with no response, what its scripts did to the jar: one row per
    // cookie with name, origin and verdict, never the value; nothing when there were none.
    fn error_cookies_list(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let result = done(self.send.read(cx)).filter(|r| r.response.is_none())?;
        let rows = error_cookie_cells(&result);
        if rows.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let cell = |id: &'static str, ix: usize, text: String| {
            div()
                .id((id, ix))
                .test_support()
                .aria_label(text.clone())
                .child(text)
        };
        let rows = rows.into_iter().enumerate().map(|(ix, c)| {
            h_flex()
                .gap(style::SPACE_2)
                .child(cell("error-cookie-name", ix, c.name))
                .child(cell("error-cookie-from", ix, c.from).text_color(muted))
                .child(cell("error-cookie-result", ix, c.result))
        });
        Some(
            v_flex()
                .id("response-error-cookies")
                .test_support()
                .items_center()
                .gap_1()
                .pt(style::SPACE_2)
                .text_size(style::TEXT_SMALL)
                .child(div().text_color(muted).child("Cookies set by scripts"))
                .children(rows)
                .into_any_element(),
        )
    }

    // Offered for an undefined variable when the send's collection is known: added to an
    // existing environment, or to a new one when the collection has none.
    fn add_env_button(&self, view: &ErrorView, cx: &mut Context<Self>) -> Option<Button> {
        let name = view.add_variable.clone()?;
        let send = self.send.read(cx);
        let root = send.root()?.to_path_buf();
        let workspace = send.workspace().clone();
        let label = if workspace.read(cx).environments(&root, cx).is_empty() {
            "Create environment"
        } else {
            "Add to environment"
        };
        Some(
            error_action("response-add-env", cx)
                .label(label)
                .accessibility_label(label)
                .on_click(move |_, _, cx| {
                    workspace.update(cx, |w, cx| {
                        w.add_variable_to_environment(&root, &name, cx);
                    });
                }),
        )
    }

    // One button per other environment defining the undefined variable; a click makes it the
    // collection's active environment and sends again as Retry does, queued behind the switch.
    fn switch_env_buttons(&self, view: &ErrorView, cx: &App) -> Vec<Button> {
        let send = self.send.read(cx);
        let Some(root) = send.root().map(Path::to_path_buf) else {
            return Vec::new();
        };
        let workspace = send.workspace().clone();
        (view.use_environments.iter().enumerate())
            .map(|(ix, env)| {
                let label = format!("Switch to {env}");
                let (root, workspace, env) = (root.clone(), workspace.clone(), env.clone());
                error_action(("response-switch-env", ix), cx)
                    .label(label.clone())
                    .accessibility_label(label)
                    .on_click(move |_, window, cx| {
                        workspace.update(cx, |w, cx| {
                            w.select_environment(&root, Some(env.clone()), cx);
                        });
                        window.dispatch_action(SendRequest.boxed_clone(), cx);
                    })
            })
            .collect()
    }

    // An error view under a right-aligned header holding the History dropdown, so kept sends
    // stay reachable when the shown result has no response. Without history the error
    // view stands alone, laid out as before.
    fn under_history(&self, error: AnyElement, cx: &Context<Self>) -> AnyElement {
        let Some(history) = self.history_button(cx) else {
            return error;
        };
        v_flex()
            .size_full()
            .gap_2()
            .child(h_flex().w_full().justify_end().child(history))
            .child(error)
            .into_any_element()
    }

    // "Back to latest", shown only while an older kept send is on screen.
    fn latest_button(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.send.read(cx).viewing()?;
        let send = self.send.clone();
        Some(
            Button::new("response-latest")
                .ghost()
                .compact()
                .label("Back to latest")
                .on_click(move |_, _, cx| send.update(cx, |m, cx| m.back_to_latest(cx)))
                .into_any_element(),
        )
    }

    // The narrow pane's "More actions" menu: the toolbar actions that no longer fit, each
    // offered on the same terms as its full-width button.
    fn more_menu(&self, response: &Response, cx: &Context<Self>) -> impl IntoElement + use<> {
        let example = self
            .save_example
            .then(|| self.example_blocked(cx).is_none() && save_example::can_save(Some(response)));
        let har = self.save_example.then(|| !response.timeline().is_empty());
        let body = self.body.clone();
        let (can_copy, can_save, save_cut) = body.read_with(cx, |b, _| {
            (b.can_copy(), b.can_save(), b.save_lock().is_some())
        });
        let schema = body.read_with(cx, |b, _| {
            b.schema_test_offered()
                .then(|| schema_menu_label(b.schema_test_lock()))
        });
        // The menu has no tooltip, so a cut body says why in the label itself.
        let save_label = if save_cut {
            "Save body (cut at the size limit)"
        } else {
            "Save body"
        };
        let send = self.send.clone();
        let viewing = self.send.read(cx).viewing().is_some();
        let choices = self.compare_menu_choices(cx);
        let pane = cx.entity().downgrade();
        Button::new("response-more")
            .ghost()
            .compact()
            .icon(ui::glyph("response-more", AssetIcon::Ellipsis))
            .tooltip("More actions")
            .accessibility_label("More actions")
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, cx| {
                if let Some(enabled) = example {
                    menu = menu.item(
                        ui::menu_item("Save response as example", !enabled, cx).on_click(
                            |_, window, cx| window.dispatch_action(Box::new(SaveAsExample), cx),
                        ),
                    );
                }
                let copy = body.clone();
                let save = body.clone();
                menu = menu
                    .item(
                        ui::menu_item("Copy body", !can_copy, cx).on_click(move |_, window, cx| {
                            copy.update(cx, |b, cx| b.copy(window, cx))
                        }),
                    )
                    .item(ui::menu_item(save_label, !can_save, cx).on_click(
                        move |_, window, cx| save.update(cx, |b, cx| b.save(window, cx)),
                    ));
                if let Some((label, locked)) = &schema {
                    let body = body.clone();
                    menu = menu.item(ui::menu_item(label.clone(), *locked, cx).on_click(
                        move |_, _, cx| body.update(cx, |b, cx| b.request_schema_test(cx)),
                    ));
                }
                if let Some(enabled) = har {
                    menu =
                        menu.item(ui::menu_item("Export as HAR", !enabled, cx).on_click(
                            |_, window, cx| window.dispatch_action(Box::new(ExportHar), cx),
                        ));
                }
                if viewing {
                    let send = send.clone();
                    menu =
                        menu.item(PopupMenuItem::new("Back to latest").on_click(
                            move |_, _, cx| send.update(cx, |m, cx| m.back_to_latest(cx)),
                        ));
                }
                for choice in choices.iter().cloned() {
                    let pane = pane.clone();
                    menu = menu.item(
                        ui::menu_item(
                            format!("Compare with {}", choice_text(&choice)),
                            choice.disabled.is_some(),
                            cx,
                        )
                        .on_click(move |_, _, cx| {
                            let _ = pane.update(cx, |p, cx| p.compare_with(choice.source, cx));
                        }),
                    );
                }
                menu
            })
    }

    // Reads the widest adjacent pair of tabs from the strip's layout, so switching tabs never
    // moves the floor; the fades and the overflow button are added as constants, since the
    // button shows only while the strip overflows.
    fn tabs_probe(&self) -> impl IntoElement {
        let (scroll, cell, whole_cell, count) = (
            self.tab_scroll.clone(),
            Rc::clone(&self.min.tabs),
            Rc::clone(&self.min.tabs_whole),
            self.strip_tabs().len(),
        );
        canvas(
            move |_, window, _| {
                let at = |ix: usize| scroll.bounds_for_item(ix);
                let pair = (0..count)
                    .filter_map(|ix| {
                        let left = at(ix)?;
                        let right = at(ix + 1).unwrap_or(left);
                        Some(right.right() - left.left())
                    })
                    .fold(Pixels::ZERO, Pixels::max);
                if pair <= Pixels::ZERO {
                    return;
                }
                if let (Some(first), Some(last)) = (at(0), count.checked_sub(1).and_then(at)) {
                    let whole = last.right() - first.left() + style::FOCUS_RING_ROOM;
                    set_width(&whole_cell, whole, window);
                }
                let need = pair
                    + style::resolve(style::STRIP_FADE_WIDTH, window) * 2.
                    + style::resolve(style::TAB_OVERFLOW_WIDTH, window);
                set_width(&cell, need, window);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    // What decides the width of the header's facts: the shown result, whether it is a kept
    // one and which send it is, and which actions are on offer.
    fn facts_key(&self, result: &RequestResult, cx: &Context<Self>) -> u64 {
        let send = self.send.read(cx);
        let mut h = std::hash::DefaultHasher::new();
        (
            std::ptr::from_ref(result) as usize,
            send.restored_ref().is_some(),
            send.viewing(),
            self.history.is_empty(),
            self.compare_menu_choices(cx).is_empty(),
            self.save_example,
            self.elapsed,
        )
            .hash(&mut h);
        h.finish()
    }

    // The narrowest wide header: the whole tabs beside the facts with the address cut. Any
    // narrower and the facts take a row of their own, so a tab is never hidden to keep one row.
    fn wide_floor(&self, window: &Window) -> Pixels {
        let m = &self.min;
        let floor = style::resolve(style::RESPONSE_COMPACT_BELOW, window);
        // Before the strip's first layout the whole width is unknown; its scrolling floor stands in.
        let tabs = match m.tabs_whole.get() > Pixels::ZERO {
            true => m.tabs_whole.get(),
            false => m.tabs.get(),
        };
        match m.facts_known() {
            true => floor.max(m.facts(window) + style::resolve(style::SPACE_2, window) + tabs),
            false => floor,
        }
    }

    // A marker the size of the pane that reads its laid-out width each frame. On crossing
    // the compact threshold it asks for one more frame, which re-renders this pane (the
    // current view) with the matching header.
    fn width_watch(&self, window: &Window) -> impl IntoElement {
        let compact = Rc::clone(&self.compact);
        let height = Rc::clone(&self.height);
        let floor = self.wide_floor(window);
        canvas(
            move |bounds, window, _| {
                height.set(bounds.size.height);
                let narrow = bounds.size.width < floor;
                if narrow != compact.get() {
                    compact.set(narrow);
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    fn tab_body(
        &self,
        result: &Arc<RequestResult>,
        error: Option<ErrorView>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let response = result.response.as_ref();
        // A tab the strip no longer lists (Events on a plain response) reads as Body.
        let tab = match self.tab {
            ResponseTab::Events if !self.events_tab => ResponseTab::Body,
            tab => tab,
        };
        match tab {
            ResponseTab::Body => match error {
                Some(e) => self.error_element(&e, cx),
                None if self
                    .send
                    .read(cx)
                    .restored()
                    .is_some_and(|r| r.body_dropped) =>
                {
                    div()
                        .id("response-body-dropped")
                        .test_support()
                        .p(style::SPACE_2)
                        .text_size(style::TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .child("Body too large to keep")
                        .into_any_element()
                }
                None => div()
                    .id("response-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.body.clone())
                    .into_any_element(),
            },
            ResponseTab::Headers => self.headers_list(result, cx),
            ResponseTab::Cookies => self.cookies_list(result, cx),
            ResponseTab::Timeline => {
                let hops = response.map(|r| r.timeline()).unwrap_or_default();
                let (masks, revealed) = self.timeline_masks(result);
                let shown_root = self.send.read(cx).root().map(Path::to_path_buf);
                let named_reveals: BTreeSet<String> = (cx.try_global::<NamedReveals>())
                    .map(|g| {
                        (g.0.iter())
                            .filter(|(root, _)| *root == shown_root)
                            .map(|(_, name)| name.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                let muted = cx.theme().muted_foreground;
                let mono = cx.theme().mono_font_family.clone();
                // A remembered response kept no hops, so say why and how to get them back.
                let not_saved = "The timeline is not saved. Send the request again to see it.";
                let restored = self.send.read(cx).restored().is_some();
                let follow = self.timeline_follow.clone();
                scroll_frame_following(
                    "response-timeline",
                    &follow,
                    v_flex()
                        .gap_2()
                        .when(restored, |list| {
                            list.child(
                                div()
                                    .id("response-timeline-not-saved")
                                    .test_support()
                                    .aria_label(not_saved)
                                    .p(style::SPACE_2)
                                    .text_size(style::TEXT_SMALL)
                                    .text_color(cx.theme().muted_foreground)
                                    .child(not_saved),
                            )
                        })
                        .children(hops.iter().enumerate().map(|(ix, h)| {
                            let heading = |id: &'static str, text: &'static str| {
                                div()
                                    .id(id)
                                    .test_support()
                                    .pt(style::SPACE_1)
                                    .text_size(style::TEXT_SMALL)
                                    .text_color(muted)
                                    .child(text)
                            };
                            let mask = &masks[ix];
                            let is_on = |field, row| revealed.contains(&(ix, field, row));
                            // A header masked by name is revealed by name, else by its place.
                            let row_on = |field, row, key: Option<&String>| match key {
                                Some(name) => named_reveals.contains(name),
                                None => is_on(field, row),
                            };
                            let pick = |m: &Masked, on: bool| {
                                if on {
                                    m.revealed.clone()
                                } else {
                                    m.hidden.clone()
                                }
                            };
                            // A header table whose masked rows carry an eye.
                            let table =
                                |ids: [&'static str; 3],
                                 eyes: (&'static str, &'static str, &'static str),
                                 field: Field,
                                 pairs: &[(String, String)],
                                 masked: &[Masked],
                                 keys: &[Option<String>],
                                 cx: &mut Context<Self>| {
                                    let key = |row: usize| keys.get(row).and_then(Option::as_ref);
                                    let shown: Vec<(String, String)> = pairs
                                        .iter()
                                        .zip(masked)
                                        .enumerate()
                                        .map(|(row, ((name, _), m))| {
                                            (name.clone(), pick(m, row_on(field, row, key(row))))
                                        })
                                        .collect();
                                    let flags: Vec<Option<bool>> = masked
                                        .iter()
                                        .enumerate()
                                        .map(|(row, m)| {
                                            m.eye().then(|| row_on(field, row, key(row)))
                                        })
                                        .collect();
                                    let pane = cx.entity();
                                    let keys = keys.to_vec();
                                    header_table(ids, &shown)
                                        .on_reveal(eyes, flags, move |row, _, cx| {
                                            let key = keys.get(row).cloned().flatten();
                                            pane.update(cx, |p, cx| match key {
                                                Some(name) => p.toggle_named_reveal(name, cx),
                                                None => p.toggle_reveal((ix, field, row), cx),
                                            });
                                        })
                                        .follow(&follow)
                                };
                            // An eye beside a single value, when redaction changed it.
                            let eye =
                                |m: &Masked,
                                 field: Field,
                                 ids: (&'static str, &'static str, &'static str),
                                 cx: &mut Context<Self>| {
                                    m.eye().then(|| {
                                        let on = is_on(field, 0);
                                        let (face, icon, words) = if on {
                                            (ids.2, AssetIcon::EyeOff, "Hide value")
                                        } else {
                                            (ids.1, AssetIcon::Eye, "Show value")
                                        };
                                        ui::swap_icon_button(ids.0, face, icon, words).on_click(
                                            cx.listener(move |p, _, _, cx| {
                                                p.toggle_reveal((ix, field, 0), cx)
                                            }),
                                        )
                                    })
                                };
                            // The slot and height give the button's focus ring room on every side.
                            let room = |id: &'static str, b: Button| {
                                Followed::new(
                                    id,
                                    &follow,
                                    h_flex()
                                        .p(style::SPACE_1)
                                        .child(action_slot(b, ActionShow::Always)),
                                )
                            };
                            let line_eye = eye(
                                &mask.url,
                                Field::Url,
                                ("hop-line-eye", "hop-line-reveal", "hop-line-hide"),
                                cx,
                            );
                            v_flex()
                                .id(("hop", ix))
                                .test_support()
                                .w_full()
                                .child({
                                    let line = hop_line(h, &pick(&mask.url, is_on(Field::Url, 0)));
                                    h_flex()
                                        .items_start()
                                        .child(
                                            div()
                                                .id("hop-line")
                                                .test_support()
                                                .min_w_0()
                                                .aria_label(line.clone())
                                                .child(line),
                                        )
                                        .children(line_eye.map(|b| room("hop-line-eye-follow", b)))
                                })
                                .when_some(h.peer_label(), |hop, peer| {
                                    hop.child(
                                        div()
                                            .id("hop-peer")
                                            .test_support()
                                            .aria_label(peer.clone())
                                            .text_size(style::TEXT_SMALL)
                                            .text_color(muted)
                                            .child(peer),
                                    )
                                })
                                .when_some(override_line(h), |hop, text| {
                                    hop.child(
                                        div()
                                            .id("hop-override")
                                            .test_support()
                                            .aria_label(text.clone())
                                            .text_size(style::TEXT_SMALL)
                                            .text_color(muted)
                                            .child(text),
                                    )
                                })
                                .when_some(pac_line(h, result), |hop, text| {
                                    hop.child(
                                        div()
                                            .id("hop-pac")
                                            .test_support()
                                            .aria_label(text.clone())
                                            .text_size(style::TEXT_SMALL)
                                            .text_color(muted)
                                            .child(text),
                                    )
                                })
                                .child(heading("hop-request-heading", "Request headers"))
                                .child(table(
                                    [
                                        "hop-request-header",
                                        "hop-request-header-name",
                                        "hop-request-header-value",
                                    ],
                                    (
                                        "hop-request-header-eye",
                                        "hop-request-header-reveal",
                                        "hop-request-header-hide",
                                    ),
                                    Field::RequestHeader,
                                    &h.request_headers,
                                    &mask.request_headers,
                                    &mask.request_named,
                                    cx,
                                ))
                                .when_some(mask.request_body.as_ref(), |hop, body| {
                                    let text = pick(body, is_on(Field::RequestBody, 0));
                                    let body_eye = eye(
                                        body,
                                        Field::RequestBody,
                                        (
                                            "hop-request-body-eye",
                                            "hop-request-body-reveal",
                                            "hop-request-body-hide",
                                        ),
                                        cx,
                                    );
                                    hop.child(heading("hop-request-body-heading", "Request body"))
                                        .child(
                                            h_flex()
                                                .items_start()
                                                .child(
                                                    div()
                                                        .id("hop-request-body")
                                                        .test_support()
                                                        .min_w_0()
                                                        .font_family(mono.clone())
                                                        .aria_label(text.clone())
                                                        .child(text),
                                                )
                                                .children(body_eye.map(|b| {
                                                    room("hop-request-body-eye-follow", b)
                                                })),
                                        )
                                })
                                .child(heading("hop-response-heading", "Response headers"))
                                .child(table(
                                    [
                                        "hop-response-header",
                                        "hop-response-header-name",
                                        "hop-response-header-value",
                                    ],
                                    (
                                        "hop-response-header-eye",
                                        "hop-response-header-reveal",
                                        "hop-response-header-hide",
                                    ),
                                    Field::ResponseHeader,
                                    &h.response_headers,
                                    &mask.response_headers,
                                    &mask.response_named,
                                    cx,
                                ))
                        })),
                )
            }
            ResponseTab::Tests => {
                let none = shows_no_tests(result).then(|| {
                    div()
                        .id("tests-none")
                        .test_support()
                        .aria_label("No tests")
                        .pb(style::SPACE_2)
                        .text_color(cx.theme().muted_foreground)
                        .child("No tests")
                });
                let total = tests_total(result).map(|line| {
                    div()
                        .id("tests-total")
                        .test_support()
                        .aria_label(line.clone())
                        .pb(style::SPACE_2)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(line)
                });
                let heading = assertion_summary(&result.assertions).map(|line| {
                    group_heading(
                        "assertions-heading",
                        "Assertions",
                        "assertions-summary",
                        line,
                        cx,
                    )
                });
                scroll_frame(
                    "response-assertions",
                    v_flex()
                        .children(none)
                        .children(total)
                        .children(heading)
                        .children(result.assertions.iter().enumerate().map(|(ix, o)| {
                            let (verdict, text) = redacted_assertion_line(o, result);
                            // The engine's own words stay on offer behind the plain sentence.
                            let engine = assertion_engine_text(o).map(|raw| result.redact(raw));
                            h_flex()
                                .id(("assertion", ix))
                                .test_support()
                                .aria_label(text.clone())
                                .items_start()
                                .gap(style::SPACE_2)
                                .child(
                                    div()
                                        .id(("assertion-verdict", ix))
                                        .test_support()
                                        .flex_none()
                                        .text_color(verdict_colour(verdict, cx))
                                        .child(verdict),
                                )
                                .child(wrapping_line(("assertion-text", ix), text).when_some(
                                    engine,
                                    |line, tip| {
                                        line.tooltip(ui::text_tooltip("assertion-tooltip", tip))
                                    },
                                ))
                        }))
                        .children(tests_block(result, cx)),
                )
            }
            ResponseTab::Console => {
                let console = self.redacted_console(result);
                self.console_list(&console, cx)
            }
            ResponseTab::Compare => self.compare_body(window, cx),
            ResponseTab::Events => self.events_body(cx),
        }
    }

    /// The console lines of `r` with its secrets redacted, as the session log shows them.
    fn redacted_console(&self, r: &Arc<RequestResult>) -> Rc<[ConsoleEntry]> {
        let mut cache = self.console_redacted.borrow_mut();
        if let Some((shown, lines)) = &*cache
            && Weak::ptr_eq(shown, &Arc::downgrade(r))
        {
            return Rc::clone(lines);
        }
        let lines: Rc<[ConsoleEntry]> = r.redacted_console().into();
        *cache = Some((Arc::downgrade(r), Rc::clone(&lines)));
        lines
    }

    /// The shown result's Timeline masks and the reveals made on them, built once per result.
    fn timeline_masks(&self, r: &Arc<RequestResult>) -> (Rc<[HopMasks]>, BTreeSet<RevealKey>) {
        let mut cache = self.timeline_masks.borrow_mut();
        let fresh = match &*cache {
            Some(m) => !Weak::ptr_eq(&m.result, &Arc::downgrade(r)),
            None => true,
        };
        if fresh {
            let hops = r
                .response
                .as_ref()
                .map(|resp| resp.timeline())
                .unwrap_or_default();
            let headers = r.response.as_ref().map(|resp| resp.headers());
            *cache = Some(ResultMasks {
                result: Arc::downgrade(r),
                hops: hops.iter().map(|h| hop_masks(r, h)).collect(),
                headers: (headers.unwrap_or_default().iter())
                    .map(|(name, value)| Masked {
                        hidden: r.redact_header(name, value),
                        revealed: value.clone(),
                    })
                    .collect(),
                cookies: HashMap::new(),
                revealed: BTreeSet::new(),
                revealed_cookies: BTreeSet::new(),
                revealed_set: BTreeSet::new(),
            });
        }
        // The block above fills the cache whenever it is empty or stale.
        #[allow(clippy::expect_used)]
        let m = cache
            .as_ref()
            .expect("the masks cache is filled for this result");
        (Rc::clone(&m.hops), m.revealed.clone())
    }

    // Each response header of `r` as shown: its name, its value masked unless revealed, its eye
    // (None without one, else whether the row is revealed), and whether that eye is locked. A
    // kept value that lost a secret shows as kept, behind a locked eye.
    fn header_rows(&self, r: &Arc<RequestResult>, cx: &App) -> Vec<HeaderRow> {
        let send = self.send.read(cx);
        let scrubbed: &[usize] = send.restored_ref().map_or(&[], |r| &r.masked_headers);
        self.timeline_masks(r);
        let cache = self.timeline_masks.borrow();
        // `timeline_masks` has just filled the cache for `r`.
        #[allow(clippy::expect_used)]
        let m = cache
            .as_ref()
            .expect("the masks cache is filled for this result");
        let headers = r.response.as_ref().map(|r| r.headers());
        (headers.unwrap_or_default().iter())
            .zip(m.headers.iter())
            .enumerate()
            .map(|(ix, ((name, _), mask))| {
                if scrubbed.contains(&ix) {
                    return (name.clone(), mask.revealed.clone(), Some(false), true);
                }
                let on = m.revealed.contains(&(0, Field::Header, ix));
                let value = if on { &mask.revealed } else { &mask.hidden };
                (name.clone(), value.clone(), mask.eye().then_some(on), false)
            })
            .collect()
    }

    // The jar's cookies for the shown host with each value masked unless revealed, and the
    // cookie's eye as `header_rows` has it.
    fn shown_cookies(
        &self,
        r: &Arc<RequestResult>,
        now: SystemTime,
    ) -> Vec<(CookieCells, Option<bool>)> {
        self.timeline_masks(r);
        let mut cache = self.timeline_masks.borrow_mut();
        // `timeline_masks` has just filled the cache for `r`.
        #[allow(clippy::expect_used)]
        let m = cache
            .as_mut()
            .expect("the masks cache is filled for this result");
        self.cookie_table(now)
            .into_iter()
            .map(|mut c| {
                // Every value is hidden, as the console hides the whole Set-Cookie header.
                let mask = (m.cookies.entry(c.value.clone())).or_insert_with(|| Masked {
                    hidden: if c.value.is_empty() {
                        String::new()
                    } else {
                        MASKED.to_owned()
                    },
                    revealed: c.value.clone(),
                });
                let key = (c.name.clone(), c.domain.clone(), c.path.clone());
                let on = m.revealed_cookies.contains(&key);
                c.value = if on { &mask.revealed } else { &mask.hidden }.clone();
                (c, mask.eye().then_some(on))
            })
            .collect()
    }

    // This response's Set-Cookie rows with each value masked unless revealed, and its eye.
    fn shown_set_cookies(&self, r: &Arc<RequestResult>) -> Vec<(SetCookieCells, Option<bool>)> {
        self.timeline_masks(r);
        let cache = self.timeline_masks.borrow();
        // `timeline_masks` has just filled the cache for `r`.
        #[allow(clippy::expect_used)]
        let m = cache
            .as_ref()
            .expect("the masks cache is filled for this result");
        set_cookie_cells(r)
            .into_iter()
            .map(|mut c| {
                // An empty value has nothing to hide, so it gets no eye.
                if c.value.is_empty() {
                    return (c, None);
                }
                let on = m.revealed_set.contains(&c.key);
                if !on {
                    c.value = MASKED.to_owned();
                }
                (c, Some(on))
            })
            .collect()
    }

    // Flips one Set-Cookie row's value between masked and revealed.
    fn toggle_set_cookie_reveal(&mut self, key: (usize, usize), cx: &mut Context<Self>) {
        if let Some(m) = self.timeline_masks.borrow_mut().as_mut()
            && !m.revealed_set.remove(&key)
        {
            m.revealed_set.insert(key);
        }
        cx.notify();
    }

    // Flips one cookie's value between masked and revealed.
    fn toggle_cookie_reveal(&mut self, key: (String, String, String), cx: &mut Context<Self>) {
        if let Some(m) = self.timeline_masks.borrow_mut().as_mut()
            && !m.revealed_cookies.remove(&key)
        {
            m.revealed_cookies.insert(key);
        }
        cx.notify();
    }

    // Flips every Timeline header named `name` in the shown collection between masked and
    // revealed, for the session.
    fn toggle_named_reveal(&mut self, name: String, cx: &mut Context<Self>) {
        let key = (self.send.read(cx).root().map(Path::to_path_buf), name);
        let set = &mut cx.default_global::<NamedReveals>().0;
        if !set.remove(&key) {
            set.insert(key);
        }
        cx.notify();
    }

    // Flips one Timeline value between masked and revealed.
    fn toggle_reveal(&mut self, key: RevealKey, cx: &mut Context<Self>) {
        if let Some(m) = self.timeline_masks.borrow_mut().as_mut()
            && !m.revealed.remove(&key)
        {
            m.revealed.insert(key);
        }
        cx.notify();
    }

    // A filter and Copy header over the entries the filter keeps, each a coloured level
    // beside its monospace text with whitespace kept, since scripts log formatted JSON.
    fn console_list(&self, console: &[ConsoleEntry], cx: &mut Context<Self>) -> AnyElement {
        let filter = self.console_filter;
        let selected = CONSOLE_FILTERS
            .iter()
            .position(|f| *f == filter)
            .unwrap_or(0);
        let this = cx.entity();
        let pick = ui::segmented(
            "console-filter",
            ["All", "Log", "Debug", "Info", "Warn", "Error"],
            selected,
            move |ix, _, cx| {
                let f = CONSOLE_FILTERS.get(*ix).copied().flatten();
                this.update(cx, |p, cx| p.set_console_filter(f, cx));
            },
        );
        // Copy reads the shown result when clicked, so a render never builds the text.
        let pane = cx.entity();
        let copy = Button::new("console-copy")
            .ghost()
            .small()
            .label("Copy")
            .on_click(move |_, _, cx| {
                let p = pane.read(cx);
                let text = done(p.send.read(cx))
                    .map(|r| console_text(&p.redacted_console(&r), p.console_filter))
                    .unwrap_or_default();
                cx.write_to_clipboard(ClipboardItem::new_string(text))
            });
        let header = h_flex()
            .gap(style::SPACE_2)
            .pb(style::SPACE_2)
            .items_center()
            .child(pick)
            .child(copy);
        let kept: Vec<&ConsoleEntry> = console.iter().filter(|e| keeps(filter, e)).collect();
        let none = kept
            .is_empty()
            .then(|| ui::empty_state("console-none", "Nothing was logged.", &[]));
        // The scroll frame is not virtualised, so the pane draws from one row and character
        // budget across entries and notes how many entries it left out.
        let (bodies, left) = console_shown(kept.iter().map(|e| e.text.as_str()));
        let left_note = (left > 0).then(|| {
            let entries = if left == 1 { "entry" } else { "entries" };
            div()
                .id("console-more-entries")
                .test_support()
                .pt(style::SPACE_2)
                .text_color(cx.theme().muted_foreground)
                .child(format!("{left} more {entries}; use Copy for the full log"))
        });
        let rows = kept
            .into_iter()
            .zip(bodies)
            .enumerate()
            .map(|(ix, (e, body))| {
                let colour = level_colour(e.level, cx);
                h_flex()
                    .id(("console", ix))
                    .test_support()
                    .items_start()
                    .gap(style::SPACE_2)
                    .child(
                        div()
                            .id(("console-level", ix))
                            .test_support()
                            .flex_none()
                            .text_color(colour)
                            .child(e.level.as_str()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("[{}]", e.phase.as_str())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(console_body("console-more", ix, body, cx)),
                    )
            });
        scroll_frame(
            "response-console",
            v_flex()
                .child(header)
                .children(none)
                .children(rows)
                .children(left_note),
        )
    }
}

// The hop's first timeline line: "{method} {url} {status} · {version} · {time}", the version
// part left out when it is unknown. The console dock words its hops the same way.
fn hop_line(h: &apinata_core::Hop, url: &str) -> String {
    let version = h.version().map(|v| format!(" · {v}")).unwrap_or_default();
    let time = format_time(h.elapsed);
    format!("{} {url} {}{version} · {time}", h.method, h.status)
}

// Each of the hop's values masked as the session log masks the same exchange, and as shown
// once revealed. A reveal still hides the handshake credentials.
fn hop_masks(r: &RequestResult, h: &apinata_core::Hop) -> HopMasks {
    let header = |pairs: &[(String, String)]| -> Vec<Masked> {
        pairs
            .iter()
            .map(|(name, value)| {
                // The console hides a credential header whole, so the Timeline does too. The
                // other handshake headers keep their scheme. Neither can be revealed.
                if is_credential_header(name) && is_handshake_header(name) {
                    let text = r.redact_header(name, value);
                    return Masked {
                        hidden: text.clone(),
                        revealed: text,
                    };
                }
                // A name that says credential hides even a literal no secret list knows.
                if is_named_credential(name, value) {
                    return Masked {
                        hidden: NAME_MASK.to_owned(),
                        revealed: value.clone(),
                    };
                }
                let one = |text: &str| mask_handshake(name, text);
                let redacted = if is_handshake_header(name) {
                    r.redact(value)
                } else {
                    r.redact_header(name, value)
                };
                Masked {
                    hidden: one(&redacted),
                    revealed: one(value),
                }
            })
            .collect()
    };
    HopMasks {
        url: Masked {
            hidden: r.redact_url(&h.url),
            revealed: h.url.clone(),
        },
        request_headers: header(&h.request_headers),
        request_named: named(&h.request_headers),
        request_body: hop_body_text(&h.request_body, h.request_body_size).map(|text| Masked {
            hidden: r.redact(&text),
            revealed: text,
        }),
        response_headers: header(&h.response_headers),
        response_named: named(&h.response_headers),
    }
}

// Whether the Timeline masks header `name` for its name, with a reveal kept by name. A handshake
// credential stays locked instead, and an empty value has nothing to hide.
fn is_named_credential(name: &str, value: &str) -> bool {
    is_credential_name(name)
        && !value.is_empty()
        && !(is_credential_header(name) && is_handshake_header(name))
}

// For each header, its lower-cased name when the Timeline masks it by name.
fn named(pairs: &[(String, String)]) -> Vec<Option<String>> {
    (pairs.iter())
        .map(|(name, value)| is_named_credential(name, value).then(|| name.to_ascii_lowercase()))
        .collect()
}

// The hop's PAC route as the timeline shows it, or None when PAC did not choose it.
fn pac_line(hop: &apinata_core::Hop, r: &RequestResult) -> Option<String> {
    (hop.pac.as_ref()).map(|route| format!("PAC: {}", r.redact(route)))
}

// The hop's host override as the timeline shows it, or None when DNS answered.
fn override_line(hop: &apinata_core::Hop) -> Option<String> {
    hop.override_from()
        .map(|mapping| format!("Host override: {mapping}"))
}

// Warn and error lines stand out; every other level stays muted.
pub(crate) fn level_colour(level: ConsoleLevel, cx: &App) -> Hsla {
    match level {
        ConsoleLevel::Warn => cx.theme().warning,
        ConsoleLevel::Error => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    }
}

// An entry's text as bounded rows, then a muted note `(more_id, ix)` for the lines left out.
pub(crate) fn console_body(
    more_id: &'static str,
    ix: usize,
    (lines, more): (Vec<String>, Option<String>),
    cx: &App,
) -> Div {
    v_flex()
        .children(lines.into_iter().map(|line| div().child(line)))
        .children(more.map(|note| {
            div()
                .id((more_id, ix))
                .test_support()
                .text_color(cx.theme().muted_foreground)
                .child(note)
        }))
}

/// One console entry's shown rows and its "... N more lines" note, if any.
pub type ConsoleRows = (Vec<String>, Option<String>);

/// How many lines of one console entry the tab shows.
pub const CONSOLE_ROW_LINES: usize = 1000;
/// How many characters of one console entry the tab shows, each line break counting as one.
pub const CONSOLE_ROW_CHARS: usize = 5000;
/// How many lines the Console tab shows across all its entries.
pub const CONSOLE_PANE_LINES: usize = 2000;
/// How many characters the Console tab shows across all its entries, counted as per entry.
pub const CONSOLE_PANE_CHARS: usize = 50_000;

/// The rows the Console tab shows for an entry's `text`, and a "... N more lines" note when
/// lines are left out. A line cut at the character limit ends in "…"; a blank line is one
/// space so it keeps its height; a trailing carriage return is dropped.
pub fn console_rows(text: &str) -> (Vec<String>, Option<String>) {
    let (mut lines, mut chars) = (CONSOLE_ROW_LINES, CONSOLE_ROW_CHARS);
    rows_within(text, &mut lines, &mut chars)
}

/// The rows of each entry text the Console tab shows, as [`console_rows`] but drawing on one
/// budget of [`CONSOLE_PANE_LINES`] and [`CONSOLE_PANE_CHARS`]; the entries left out once it
/// is spent are counted, not returned.
pub fn console_shown<'a>(texts: impl IntoIterator<Item = &'a str>) -> (Vec<ConsoleRows>, usize) {
    let (mut lines, mut chars) = (CONSOLE_PANE_LINES, CONSOLE_PANE_CHARS);
    let mut shown = Vec::new();
    let mut left = 0;
    for text in texts {
        if lines == 0 || chars == 0 {
            left += 1;
            continue;
        }
        let mut entry_lines = lines.min(CONSOLE_ROW_LINES);
        let mut entry_chars = chars.min(CONSOLE_ROW_CHARS);
        let rows = rows_within(text, &mut entry_lines, &mut entry_chars);
        lines -= lines.min(CONSOLE_ROW_LINES) - entry_lines;
        chars -= chars.min(CONSOLE_ROW_CHARS) - entry_chars;
        shown.push(rows);
    }
    (shown, left)
}

// Takes rows of `text` while `lines` and `chars` last, spending them, and notes the rest.
fn rows_within(text: &str, lines: &mut usize, chars: &mut usize) -> (Vec<String>, Option<String>) {
    let total = text.bytes().filter(|b| *b == b'\n').count() + 1;
    let mut rows = Vec::new();
    for line in text.split('\n') {
        if *lines == 0 || *chars == 0 {
            break;
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        *lines -= 1;
        if let Some((cut, _)) = line.char_indices().nth(*chars) {
            rows.push(format!("{}…", &line[..cut]));
            *chars = 0;
            break;
        }
        *chars = chars.saturating_sub(line.chars().count() + 1);
        rows.push(if line.is_empty() {
            " ".to_owned()
        } else {
            line.to_owned()
        });
    }
    let more = total - rows.len();
    let note = match more {
        0 => None,
        1 => Some("... 1 more line".to_owned()),
        n => Some(format!("... {n} more lines")),
    };
    (rows, note)
}

// The Console filter's segments, in order: All, then each level.
pub(crate) const CONSOLE_FILTERS: [Option<ConsoleLevel>; 6] = [
    None,
    Some(ConsoleLevel::Log),
    Some(ConsoleLevel::Debug),
    Some(ConsoleLevel::Info),
    Some(ConsoleLevel::Warn),
    Some(ConsoleLevel::Error),
];

// Whether `filter` shows `e`; a level the filter cannot name shows only under All.
fn keeps(filter: Option<ConsoleLevel>, e: &ConsoleEntry) -> bool {
    filter.is_none_or(|f| f == e.level)
}

/// "{level} [{phase}] {text}".
pub fn console_line(e: &ConsoleEntry) -> String {
    format!("{} [{}] {}", e.level.as_str(), e.phase.as_str(), e.text)
}

/// The lines of the entries `filter` keeps (all for `None`), joined by "\n".
pub fn console_text(entries: &[ConsoleEntry], filter: Option<ConsoleLevel>) -> String {
    entries
        .iter()
        .filter(|e| keeps(filter, e))
        .map(console_line)
        .collect::<Vec<_>>()
        .join("\n")
}

// The script tests under the assertions: a summary, then each entry's verdict, phase and line.
fn tests_block(result: &RequestResult, cx: &App) -> Option<Div> {
    let summary = test_summary(&result.tests)?;
    let rows = result.tests.iter().enumerate().map(|(ix, t)| {
        let (verdict, text) = test_line(t);
        let text = result.redact(&text);
        h_flex()
            .id(("test", ix))
            .test_support()
            .aria_label(text.clone())
            .items_start()
            .gap(style::SPACE_2)
            .child(
                div()
                    .id(("test-verdict", ix))
                    .test_support()
                    .flex_none()
                    .text_color(verdict_colour(verdict, cx))
                    .child(verdict),
            )
            .child(
                div()
                    .id(("test-phase", ix))
                    .test_support()
                    .flex_none()
                    .text_color(cx.theme().muted_foreground)
                    .child(test_source(t.phase)),
            )
            .child(wrapping_line(("test-text", ix), text))
    });
    Some(
        v_flex()
            .pt(style::SPACE_2)
            .child(group_heading(
                "tests-heading",
                "Tests",
                "tests-summary",
                summary,
                cx,
            ))
            .children(rows),
    )
}

// A result line that takes the row's remaining width and wraps there, so a long value never
// runs past the pane's edge.
fn wrapping_line(id: (&'static str, usize), text: String) -> ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .flex_1()
        .min_w_0()
        .child(text)
}

// A group's heading row: its name, then the group's own counts in muted text.
fn group_heading(
    id: &'static str,
    title: &'static str,
    summary_id: &'static str,
    summary: String,
    cx: &App,
) -> Div {
    h_flex()
        .gap(style::SPACE_2)
        .pb(style::SPACE_2)
        .child(
            div()
                .id(id)
                .test_support()
                .aria_label(title)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .id(summary_id)
                .test_support()
                .text_color(cx.theme().muted_foreground)
                .child(summary),
        )
}

/// One History dropdown row's texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLine {
    /// `KeptSend::target`.
    pub target: String,
    /// "{status} · {time} · {size} · {age}".
    pub summary: String,
    /// The target and the exact local time on two lines; the time alone without a target.
    pub tip: String,
}

/// The dropdown row for `send` as seen at `now`.
pub fn history_row(send: &KeptSend, now: SystemTime) -> HistoryLine {
    let summary = format!(
        "{} · {} · {} · {}",
        send.status,
        format_time(send.time),
        format_size(send.size),
        last_response::age_of(send.saved_at, now)
    );
    let target = send.target();
    let time = last_response::local_labels(send.saved_at, now).1;
    let tip = match target.is_empty() {
        true => time,
        false => format!("{target}\n{time}"),
    };
    HistoryLine {
        target,
        summary,
        tip,
    }
}

/// Whether `send` shows a kept result whose body was saved with its secrets scrubbed.
fn body_scrubbed(send: &Entity<SendModel>, cx: &App) -> bool {
    send.read(cx).restored().is_some_and(|r| r.body_masked)
}

/// Bounds a tab's `column` to the space left in the pane and scrolls it vertically, with the
/// always-visible scrollbar (id `{id}-bar`) while it overflows. `id` sits on an outer frame; the
/// column takes `SCROLLBAR_GUTTER` of right padding only while it overflows, so rows stay clear
/// of the bar and a column that fits keeps its full width.
pub(crate) fn scroll_frame(id: &'static str, column: Div) -> AnyElement {
    ScrollFrame {
        id,
        column,
        subject: None,
        follow: None,
    }
    .into_any_element()
}

/// Like `scroll_frame`, but a control inside a [`Followed`] reporting to `follow` is scrolled
/// into view when focus lands on it.
pub(crate) fn scroll_frame_following(
    id: &'static str,
    follow: &FocusFollow,
    column: Div,
) -> AnyElement {
    ScrollFrame {
        id,
        column,
        subject: None,
        follow: Some(follow.clone()),
    }
    .into_any_element()
}

/// Like `scroll_frame`, but the scroll returns to the top whenever `subject` differs from the
/// previous render's, so another entry opens at its first line while the same one keeps its place.
pub(crate) fn scroll_frame_for(id: &'static str, subject: u64, column: Div) -> AnyElement {
    ScrollFrame {
        id,
        column,
        subject: Some(subject),
        follow: None,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct ScrollFrame {
    id: &'static str,
    column: Div,
    subject: Option<u64>,
    follow: Option<FocusFollow>,
}

impl RenderOnce for ScrollFrame {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // The scroll position outlives a render, keyed by the frame's id.
        let scroll = window
            .use_keyed_state(
                SharedString::from(format!("{}-handle", self.id)),
                cx,
                |_, _| ScrollHandle::default(),
            )
            .read(cx)
            .clone();
        if let Some(subject) = self.subject {
            let last = window.use_keyed_state(
                SharedString::from(format!("{}-subject", self.id)),
                cx,
                |_, _| std::cell::Cell::new(None::<u64>),
            );
            if last.read(cx).replace(Some(subject)) != Some(subject) {
                scroll.set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)));
            }
        }
        div()
            .id(self.id)
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                self.column
                    .size_full()
                    .when(ui::overflows_down(&scroll), |c| {
                        c.pr(style::SCROLLBAR_GUTTER)
                    })
                    .children(self.follow.as_ref().map(|f| f.watch(&scroll)))
                    .id(SharedString::from(format!("{}-scroll", self.id)))
                    .overflow_y_scroll()
                    .track_scroll(&scroll),
            )
            .children(ui::scroll_bar_y(
                SharedString::from(format!("{}-bar", self.id)),
                &scroll,
            ))
    }
}

// The Timeline shows every hop's headers, so the handshake credentials (an NTLM message, a
// Negotiate token) are hidden there: the scheme stays and everything after it is masked.
fn is_handshake_header(name: &str) -> bool {
    [
        "authorization",
        "proxy-authorization",
        "www-authenticate",
        "proxy-authenticate",
    ]
    .iter()
    .any(|n| name.eq_ignore_ascii_case(n))
}

// `value` of header `name`, its credential masked when the header carries one.
fn mask_handshake(name: &str, value: &str) -> String {
    if !is_handshake_header(name) {
        return value.to_owned();
    }
    match value.split_once(' ') {
        Some((scheme, _)) => format!("{scheme} [redacted]"),
        None => "[redacted]".to_owned(),
    }
}

/// The `(index, pair)` of each header whose name or value contains `filter`, ignoring case;
/// every header for an empty filter. The index is the position in `pairs`.
pub fn header_shown<'a>(
    pairs: &'a [(String, String)],
    filter: &str,
) -> Vec<(usize, &'a (String, String))> {
    let f = filter.to_lowercase();
    pairs
        .iter()
        .enumerate()
        .filter(|(_, (n, v))| {
            f.is_empty() || n.to_lowercase().contains(&f) || v.to_lowercase().contains(&f)
        })
        .collect()
}

/// The shown headers as `Name: value` lines, which is what the Headers tab's Copy puts on the
/// clipboard.
pub fn header_lines(pairs: &[(String, String)], filter: &str) -> String {
    header_shown(pairs, filter)
        .into_iter()
        .map(|(_, (n, v))| format!("{n}: {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `(index, cookie)` of each cookie whose name, value or domain contains `filter`,
/// ignoring case.
pub fn cookie_shown<'a>(cookies: &'a [CookieCells], filter: &str) -> Vec<(usize, &'a CookieCells)> {
    let f = filter.to_lowercase();
    cookies
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            f.is_empty()
                || [&c.name, &c.value, &c.domain]
                    .iter()
                    .any(|t| t.to_lowercase().contains(&f))
        })
        .collect()
}

/// One cookie as a `Set-Cookie`-style line: `name=value; Domain=..; Path=..` and the
/// `HttpOnly` and `Secure` flags it carries.
pub fn cookie_line(c: &CookieCells) -> String {
    let mut line = format!(
        "{}={}; Domain={}; Path={}",
        c.name, c.value, c.domain, c.path
    );
    if c.http_only {
        line.push_str("; HttpOnly");
    }
    if c.secure {
        line.push_str("; Secure");
    }
    line
}

/// The shown cookies as [`cookie_line`]s, one per line.
pub fn cookie_lines(cookies: &[CookieCells], filter: &str) -> String {
    cookie_shown(cookies, filter)
        .into_iter()
        .map(|(_, c)| cookie_line(c))
        .collect::<Vec<_>>()
        .join("\n")
}

fn no_match(id: &'static str, what: &'static str) -> impl IntoElement {
    let text = format!("No matching {what}");
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .child(text)
}

// The row copy button and the Copy button both confirm with a toast.
fn copy_with_toast(text: String, toast: &'static str, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    window.push_notification(ui::info_toast(toast), cx);
}

/// A read-only name/value table: a fixed, truncated name column that shows a cut name in a
/// tooltip, and a wrapping value column. `ids` names the rows, the name cells and the value
/// cells; the headings take `"{cell id}-head"`.
pub(crate) fn header_table(ids: [&'static str; 3], pairs: &[(String, String)]) -> KvView {
    let columns = vec![
        KvColumn::share(
            SharedString::from(format!("{}-head", ids[1])),
            ids[1],
            "Name",
            style::HEADER_NAME_SHARE,
            style::HEADER_NAME_WIDTH,
        )
        .tip_when_cut(),
        KvColumn::flex(
            SharedString::from(format!("{}-head", ids[2])),
            ids[2],
            "Value",
        ),
    ];
    let rows = pairs
        .iter()
        .map(|(name, value)| {
            vec![
                KvCell::Text(name.clone().into()),
                KvCell::Text(value.clone().into()),
            ]
        })
        .collect();
    ui::kv_view(format!("{}-table", ids[0]), columns, rows).row_ids(ids[0])
}

impl ResponsePane {
    /// The open comparison's source, if the Compare tab is open.
    pub fn compare_source(&self) -> Option<CompareSource> {
        self.compare.as_ref().map(|c| c.source)
    }

    /// The computed comparison; `None` until it lands or with no Compare tab.
    pub fn comparison(&self) -> Option<Arc<Comparison>> {
        self.compare.as_ref()?.comparison.clone()
    }

    /// The Compare tab's rows as drawn; empty with no Compare tab.
    pub fn compare_rows(&self) -> Arc<[CompareRow]> {
        self.compare
            .as_ref()
            .map_or_else(|| Arc::from(Vec::new()), |c| Arc::clone(&c.rows))
    }

    /// How far the Compare list is scrolled; `None` with no Compare tab.
    pub fn compare_scroll_offset(&self) -> Option<gpui_kit::Point<Pixels>> {
        Some(
            self.compare
                .as_ref()?
                .scroll
                .0
                .borrow()
                .base_handle
                .offset(),
        )
    }

    /// The sources to compare the response on screen with: the previous send, then each
    /// saved example. A draft holds no examples.
    pub fn compare_choices(&self, cx: &App) -> Vec<SourceChoice> {
        let examples = self
            .target
            .as_ref()
            .and_then(|target| target(cx))
            .filter(|(_, item)| !item.is_draft())
            .and_then(|(root, item)| {
                self.send
                    .read(cx)
                    .workspace()
                    .read(cx)
                    .request(&root, &item, cx)
            })
            .map(|view| compare_source::example_refs(&view.examples))
            .unwrap_or_default();
        let viewing = self.send.read(cx).viewing();
        compare_source::choices(&self.history, viewing, &examples)
    }

    // The choices the toolbar offers: none for a pane without Compare or without a response.
    fn compare_menu_choices(&self, cx: &App) -> Vec<SourceChoice> {
        let has_response = done(self.send.read(cx)).is_some_and(|r| r.response.is_some());
        match self.compare_offered && has_response {
            true => self.compare_choices(cx),
            false => Vec::new(),
        }
    }

    /// Opens the Compare tab on the response on screen against `source` and computes it off
    /// the UI thread. Does nothing when the source or the response is missing.
    pub fn compare_with(&mut self, source: CompareSource, cx: &mut Context<Self>) {
        self.start_compare(source, true, cx);
    }

    // Redoes the open comparison against the response now on screen, keeping the tab the
    // user is on; a result still in flight is dropped by the generation bump.
    fn recompute_compare(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.compare_source() else {
            return;
        };
        // A send in flight only makes the open result stale; it lands or closes on the next.
        if done(self.send.read(cx)).is_none() {
            self.compare_generation += 1;
            return;
        }
        // Nothing to compare any more (no response, no previous send, example gone): close.
        if !self.start_compare(source, false, cx) {
            self.close_compare(cx);
        }
    }

    // Starts a comparison and reports whether one is now computing. A recompute keeps the
    // open state (rows, scroll, volatile toggle) until the new result replaces it.
    fn start_compare(
        &mut self,
        source: CompareSource,
        select: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(result) = done(self.send.read(cx)) else {
            return false;
        };
        let Some(response) = result.response.as_ref() else {
            return false;
        };
        // `shown_secrets` is empty unless the send on screen was kept, so a send the store
        // never scrubbed is compared as it is shown.
        let send = self.send.read(cx);
        let restored = send.restored_ref();
        let kept_masked = restored.map_or(&[][..], |r| &r.masked_headers);
        let digest_key = cx
            .try_global::<ResponseStore>()
            .and_then(ResponseStore::digest_key);
        let mut new = compare_source::side_of_response(
            response,
            send.shown_secrets(),
            kept_masked,
            restored.and_then(|r| r.header_digests.as_ref()),
            digest_key.as_ref(),
        );
        if let Some(r) = restored {
            new = compare_source::at_rest(new);
            new = compare_source::drop_placeholder_values(
                new,
                r.header_digests.as_ref(),
                digest_key.as_ref(),
            );
        }
        if restored.is_some_and(|r| r.body_dropped) {
            new.base = new.base.with_body_state(BodyState::Dropped);
        }
        let mut live = None;
        // An example is read now; a previous send is read through the store without writing.
        let (ready, peek) = match source {
            CompareSource::Example(ix) => {
                let Some(r) = self.example_response(ix, cx) else {
                    return false;
                };
                (
                    Some(compare_source::side_of_example(&r, digest_key.as_ref())),
                    None,
                )
            }
            CompareSource::Previous => {
                let viewing = send.viewing();
                let Some(seq) = compare_source::previous_seq(&self.history, viewing) else {
                    return false;
                };
                let Some((root, item)) = self.key.as_ref().and_then(|k| k(cx)) else {
                    return false;
                };
                // Only the very send cached: a seq reused after the newest send went names
                // another one, whose save time differs.
                let baseline = self.history.iter().find(|k| k.seq == seq);
                live = (self.live_headers.get(&(root.clone(), item.clone(), seq)))
                    .filter(|(kept, _)| baseline.is_some_and(|b| kept.is(b)))
                    .map(|(_, headers)| headers.clone());
                let (reply, answer) = async_channel::bounded(1);
                last_response::enqueue(
                    StoreOp::PeekSend {
                        root,
                        item,
                        seq,
                        reply,
                    },
                    cx,
                );
                (None, Some(answer))
            }
        };
        self.compare_generation += 1;
        let generation = self.compare_generation;
        if select || self.compare_source() != Some(source) {
            self.compare = Some(CompareState::new(source));
        }
        if select {
            self.set_tab(ResponseTab::Compare, cx);
            self.make_room_for_compare(cx);
        }
        self.compare_task = Some(cx.spawn(async move |this, cx| {
            let old = match (ready, peek) {
                (Some(snapshot), _) => Some(snapshot),
                (None, Some(answer)) => answer.recv().await.ok().flatten().map(|s: Stored| {
                    // A send made live in this session brings back the real values the
                    // store masked.
                    let mut side = compare_source::side_of_stored(&s, digest_key.as_ref());
                    if let Some(headers) = live {
                        side.headers = headers;
                    }
                    side
                }),
                (None, None) => None,
            };
            let computed = match old {
                Some(old) => Some(
                    cx.background_executor()
                        .spawn(async move {
                            let c = compare_source::compare_sides(&old, &new);
                            (c, (old.headers, new.headers))
                        })
                        .await,
                ),
                None => None,
            };
            let _ = this.update(cx, |p, cx| {
                // A newer comparison, or a close, owns the tab now; this result is stale.
                if p.compare_generation != generation {
                    return;
                }
                match (computed, p.compare.as_mut()) {
                    (Some((c, headers)), Some(state)) => {
                        state.comparison = Some(Arc::new(c));
                        state.headers = headers;
                        state.revealed.clear();
                        state.refresh_rows();
                        cx.notify();
                    }
                    // The baseline was evicted or cleared since the list was read.
                    _ => p.close_compare(cx),
                }
            });
        }));
        cx.notify();
        true
    }

    // A comparison needs room to read: when the pane is short, raise the response's share of
    // the split so the list gets a usable height under the toolbar and summary.
    fn make_room_for_compare(&mut self, cx: &mut Context<Self>) {
        self.make_room(style::COMPARE_PANE_WANTED, cx);
    }

    // Raises the response's share of the split until the pane is `wanted` tall, but never
    // past three quarters of the room, so the editor keeps its minimum.
    fn make_room(&mut self, wanted: Pixels, cx: &mut Context<Self>) {
        let (height, room) = (self.height.get(), self.room.get());
        let want = wanted.min(room * 0.75);
        if room <= Pixels::ZERO || height <= Pixels::ZERO || height >= want {
            return;
        }
        let share = 1. - want / room;
        let workspace = self.send.read(cx).workspace().clone();
        workspace.update(cx, |w, cx| w.set_split_ratio(share, cx));
    }

    /// The narrowest width that shows the pane's header whole: the widest pair of adjacent
    /// tabs between the strip's fades, the status chip, and an open comparison's buttons.
    /// `None` before the tab strip's first layout.
    pub fn min_width(&self) -> Option<Pixels> {
        let m = &self.min;
        if let Some(pinned) = m.pinned.get() {
            return Some(pinned);
        }
        let tabs = m.tabs.get();
        // The compare controls sit in the body, a ring's room in from the pane's edge.
        let compare = m.compare() + style::FOCUS_RING_ROOM;
        (tabs > Pixels::ZERO).then(|| tabs.max(m.status.get()).max(compare))
    }

    /// Pins `min_width` to `width` (or unpins it with `None`), so a test can squeeze the pane
    /// below its header to check that the pane clips what overhangs.
    #[doc(hidden)]
    pub fn pin_min_width(&self, width: Option<Pixels>) {
        self.min.pinned.set(width);
    }

    /// The cell the owning editor fills with the room of the split this pane sits in.
    pub fn room_cell(&self) -> Rc<Cell<Pixels>> {
        Rc::clone(&self.room)
    }

    /// Removes the Compare tab, returning to Body when it was selected, and drops any
    /// result still being computed.
    pub fn close_compare(&mut self, cx: &mut Context<Self>) {
        self.compare = None;
        self.min.source.set(Pixels::ZERO);
        self.min.close.set(Pixels::ZERO);
        self.compare_task = None;
        self.compare_generation += 1;
        if self.tab == ResponseTab::Compare {
            self.set_tab(ResponseTab::Body, cx);
        }
        cx.notify();
    }

    // The saved response of example `ix` of the request this pane shows.
    fn example_response(&self, ix: usize, cx: &App) -> Option<apinata_core::ExampleResponse> {
        let (root, item) = self.target.as_ref().and_then(|target| target(cx))?;
        if item.is_draft() {
            return None;
        }
        let workspace = self.send.read(cx).workspace().read(cx);
        let view = workspace.request(&root, &item, cx)?;
        view.examples.get(ix)?.response.clone()
    }

    // The menu of sources, shared by the toolbar button and the tab's source chip.
    // With an `icon` the button shows it beside the chevron and takes `label` as its name and
    // tooltip; without one it shows the label.
    fn compare_menu(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        icon: Option<AssetIcon>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let choices = self.compare_menu_choices(cx);
        if choices.is_empty() {
            return None;
        }
        let pane = cx.entity().downgrade();
        let label: SharedString = label.into();
        // An example's name can be any length, so the label truncates at a fixed width (the
        // tooltip keeps the whole name) and the button's share of the pane's floor stays bounded.
        let tip = label.clone();
        let button = match icon {
            Some(icon) => icon_dropdown(id, icon, label, cx),
            None => Button::new(id)
                .ghost()
                .compact()
                .max_w(style::COMPARE_SOURCE_MAX_WIDTH)
                .accessibility_label(label.clone())
                .child(
                    h_flex()
                        .gap(style::SPACE_1)
                        .min_w_0()
                        .child(div().min_w_0().truncate().child(label))
                        .child(chevron(cx)),
                ),
        };
        let dropdown =
            button.dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, cx| {
                for choice in choices.iter().cloned() {
                    let pane = pane.clone();
                    menu = menu.item(
                        ui::menu_item(choice_text(&choice), choice.disabled.is_some(), cx)
                            .on_click(move |_, _, cx| {
                                let _ = pane.update(cx, |p, cx| p.compare_with(choice.source, cx));
                            }),
                    );
                }
                menu
            });
        Some(tipped(id, tip, dropdown))
    }

    // The toolbar's Compare dropdown; a narrow pane folds it into the more menu.
    fn compare_button(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.compact.get() {
            return None;
        }
        self.compare_menu("response-compare", "Compare", Some(COMPARE_ICON), cx)
    }

    // Reveals or masks both sides of one header row of the comparison.
    fn toggle_compare_reveal(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.compare.as_mut() {
            let before = state.revealed.len();
            state.revealed.retain(|n| !n.eq_ignore_ascii_case(name));
            if state.revealed.len() == before {
                state.revealed.push(name.to_owned());
            }
            state.refresh_rows();
            cx.notify();
        }
    }

    // Holds the real header values of the live send on screen once the store says which seq
    // it kept it as, for a later comparison with it as the previous send.
    fn keep_live_headers(&mut self, cx: &App) {
        let send = self.send.read(cx);
        let Some(kept) = send.kept_as() else {
            return;
        };
        let Some((root, item)) = self.key.as_ref().and_then(|k| k(cx)) else {
            return;
        };
        let key = (root, item, kept.seq);
        // A newer send kept under the same seq replaces the entry; the same one is kept as is.
        if self.live_headers.get(&key).is_some_and(|(k, _)| *k == kept) {
            return;
        }
        if let Some(response) = done(send).as_ref().and_then(|r| r.response.as_ref()) {
            let digest_key = cx
                .try_global::<ResponseStore>()
                .and_then(ResponseStore::digest_key);
            let side: CompareSide = compare_source::side_of_response(
                response,
                send.shown_secrets(),
                &[],
                None,
                digest_key.as_ref(),
            );
            self.live_headers.insert(key, (kept, side.headers));
        }
    }

    // Shows or folds the headers that change on every send.
    fn toggle_volatile(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.compare.as_mut() {
            state.volatile_open = !state.volatile_open;
            state.refresh_rows();
            cx.notify();
        }
    }

    // Whether the live or the shown response is an event-stream, which offers the Events tab.
    fn refresh_events_tab(&mut self, cx: &App) {
        let send = self.send.read(cx);
        self.events_tab = match send.live() {
            Some(live) => live.is_event_stream(),
            None => done(send)
                .and_then(|r| r.response.as_ref().map(is_event_stream_response))
                .unwrap_or(false),
        };
    }

    // Parses the shown event-stream's body off the UI thread, once per result.
    // A stream that just ended seeds the list with the live events until the parse lands.
    fn parse_events(&mut self, from_stream: bool, cx: &mut Context<Self>) {
        self.done_events = None;
        self.events_task = None;
        let Some(result) = done(self.send.read(cx)) else {
            return;
        };
        if !result
            .response
            .as_ref()
            .is_some_and(is_event_stream_response)
        {
            return;
        }
        let weak = Arc::downgrade(&result);
        let seed = self.send.read(cx).ended_events();
        if from_stream && !seed.is_empty() {
            self.done_events = Some((Weak::clone(&weak), Rc::new(seed.to_vec())));
        }
        self.events_task = Some(cx.spawn(async move |this, cx| {
            let parsed = cx
                .background_executor()
                .spawn(async move {
                    result
                        .response
                        .as_ref()
                        .map(Response::events)
                        .unwrap_or_default()
                })
                .await;
            let _ = this.update(cx, |p, cx| {
                // A newer result owns the pane now; this parse is stale.
                let current = done(p.send.read(cx));
                if current.is_some_and(|c| Weak::ptr_eq(&weak, &Arc::downgrade(&c))) {
                    p.done_events = Some((weak, Rc::new(parsed)));
                    cx.notify();
                }
            });
        }));
    }

    // The parsed events of the shown, finished result, once the parse has landed.
    fn shown_events(&self, cx: &App) -> Option<Rc<Vec<SseEvent>>> {
        let (weak, events) = self.done_events.as_ref()?;
        let shown = done(self.send.read(cx))?;
        Weak::ptr_eq(weak, &Arc::downgrade(&shown)).then(|| Rc::clone(events))
    }

    /// The tabs the strip lists, in order.
    pub fn shown_tabs(&self, _cx: &App) -> Vec<ResponseTab> {
        self.strip_tabs().to_vec()
    }

    /// The Events tab's rows: the live events while streaming, else the shown response's.
    pub fn event_rows(&self, cx: &App) -> Vec<EventRow> {
        match self.send.read(cx).live() {
            Some(live) => live_stream::event_rows(live.events()),
            None => self
                .shown_events(cx)
                .map(|e| live_stream::event_rows(&e))
                .unwrap_or_default(),
        }
    }

    /// Events that fell off the front of the live list.
    pub fn events_dropped(&self, cx: &App) -> u64 {
        self.send.read(cx).live().map_or(0, |l| l.dropped_events())
    }

    /// Copies the whole data of event `ix` to the clipboard.
    pub fn copy_event(&mut self, ix: usize, cx: &mut Context<Self>) {
        let data = match self.send.read(cx).live() {
            Some(live) => live.events().get(ix).map(|e| e.data.clone()),
            None => self
                .shown_events(cx)
                .and_then(|e| e.get(ix).map(|e| e.data.clone())),
        };
        if let Some(data) = data {
            cx.write_to_clipboard(ClipboardItem::new_string(data));
        }
    }

    // The Events tab: a muted note when old events were dropped or none came, over the rows.
    fn events_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let live = self.send.read(cx).live().is_some();
        let dropped = self.events_dropped(cx);
        let kept = self.shown_events(cx);
        let count = match (live, &kept) {
            (true, _) => self.send.read(cx).live().map_or(0, |l| l.events().len()),
            (false, Some(e)) => e.len(),
            (false, None) => 0,
        };
        // Stay on the newest event only while a live list already sat at its end.
        if live && count > 0 && self.events_scroll.is_scrolled_to_end().unwrap_or(true) {
            self.events_scroll
                .scroll_to_item(count - 1, ScrollStrategy::Bottom);
        }
        let send = self.send.clone();
        let pane = cx.entity();
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let mono = theme.mono_font_family.clone();
        let list = uniform_list(
            "response-events",
            count,
            move |range: std::ops::Range<usize>, _: &mut Window, cx: &mut App| {
                let rows: Vec<EventRow> = match (&kept, send.read(cx).live()) {
                    (_, Some(l)) => range
                        .clone()
                        .filter_map(|ix| l.events().get(ix))
                        .map(|e| live_stream::event_rows(std::slice::from_ref(e)).remove(0))
                        .collect(),
                    (Some(e), None) => range
                        .clone()
                        .filter_map(|ix| e.get(ix))
                        .map(|e| live_stream::event_rows(std::slice::from_ref(e)).remove(0))
                        .collect(),
                    (None, None) => Vec::new(),
                };
                range
                    .zip(rows)
                    .map(|(ix, row)| {
                        let pane = pane.clone();
                        v_flex()
                            .id(("event-row", ix))
                            .test_support()
                            .h(style::EVENT_ROW_HEIGHT)
                            // Each list item lays out on its own and would shrink to its
                            // text, cutting the separator short; it spans the list instead.
                            .w_full()
                            .overflow_hidden()
                            .px_2()
                            .py_1()
                            // The last row ends the list, so it paints no edge at the dock.
                            .when(ix + 1 < count, |r| r.border_b_1().border_color(border))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                pane.update(cx, |p, cx| p.copy_event(ix, cx));
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_size(style::TEXT_SMALL)
                                    .child(div().font_weight(FontWeight::MEDIUM).child(row.name))
                                    .children(row.id.map(|id| div().text_color(muted).child(id)))
                                    .children(
                                        live_stream::retry_label(row.retry)
                                            .map(|t| div().text_color(muted).child(t)),
                                    ),
                            )
                            .children(row.preview.split('\n').map(|line| {
                                div()
                                    .font_family(mono.clone())
                                    .text_size(style::TEXT_SMALL)
                                    .whitespace_nowrap()
                                    .child(line.to_owned())
                            }))
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.events_scroll)
        .size_full();
        v_flex()
            .id("response-events-tab")
            .test_support()
            .flex_1()
            .min_h_0()
            .w_full()
            .when(dropped > 0, |v| {
                v.child(
                    div()
                        .id("events-note")
                        .test_support()
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .child(format!("{dropped} earlier events not shown")),
                )
            })
            // A live stream's waiting row already speaks for an empty list.
            .when(!live && count == 0, |v| {
                v.child(
                    div()
                        .id("response-events-empty")
                        .test_support()
                        .aria_label("No events")
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .child("No events"),
                )
            })
            .child(
                div()
                    .id("response-events")
                    .test_support()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(list)
                    .vertical_scrollbar(&self.events_scroll),
            )
            .into_any_element()
    }

    // The streaming status row: bare status code, "streaming", the clock and bytes so far.
    fn streaming_facts(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let send = self.send.read(cx);
        let live = send.live()?;
        let muted = |id: &'static str, text: String| {
            div()
                .id(id)
                .test_support()
                .text_size(style::TEXT_SMALL)
                .text_color(cx.theme().muted_foreground)
                .child(text)
        };
        Some(
            h_flex()
                .gap(style::SPACE_2)
                .items_center()
                .child(
                    div()
                        .id("response-status")
                        .test_support()
                        .relative()
                        .flex_none()
                        .child(ui::status_pill(live.status(), ""))
                        .child(width_probe(&self.min.status)),
                )
                .child(muted("response-stream-state", "streaming".to_owned()))
                .children(
                    send.elapsed()
                        .map(|t| muted("response-elapsed", format_time(t))),
                )
                .child(muted("response-size", format_size(live.bytes())))
                .into_any_element(),
        )
    }

    // The streaming view: the tab strip over the tail of the body, one row per line.
    fn live_view(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let strip = div()
            .id("response-tab-strip")
            .test_support()
            .relative()
            // The strip keeps its tabs' width; facts that do not fit beside it wrap below.
            .flex_grow(1.)
            .flex_shrink_0()
            .max_w_full()
            .child(self.tab_bar(window, cx))
            .child(self.tabs_probe());
        let header = h_flex()
            .w_full()
            .flex_wrap()
            .justify_end()
            .items_center()
            .gap(style::SPACE_2)
            .child(strip)
            .children(self.streaming_facts(cx));
        let waiting = self.waiting_row(cx);
        if self.tab == ResponseTab::Events && self.events_tab {
            return v_flex()
                .id("response-streaming")
                .test_support()
                .size_full()
                .gap_2()
                .child(header)
                .children(waiting)
                .child(self.events_body(cx))
                .into_any_element();
        }
        let (count, skipped) = self
            .send
            .read(cx)
            .live()
            .map_or((0, 0), |l| (l.row_count(), l.skipped_lines()));
        // Stay on the newest row only while the list already sat at its end.
        if count > 0 && self.live_scroll.is_scrolled_to_end().unwrap_or(true) {
            self.live_scroll
                .scroll_to_item(count - 1, ScrollStrategy::Bottom);
        }
        let send = self.send.clone();
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let size = BodyView::font_size(cx);
        let list = uniform_list(
            "live-body-list",
            count,
            move |range: std::ops::Range<usize>, _: &mut Window, cx: &mut App| {
                let live = send.read(cx).live();
                range
                    .map(|ix| {
                        div()
                            .id(("live-row", ix))
                            .test_support()
                            .font_family(mono.clone())
                            .text_size(size)
                            .whitespace_nowrap()
                            .child(live.and_then(|l| l.row(ix)).unwrap_or_default())
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.live_scroll)
        .size_full();
        v_flex()
            .id("response-streaming")
            .test_support()
            .size_full()
            .gap_2()
            .child(header)
            // The waiting row and the note sit above the list so every list row shares one height.
            .children(waiting)
            .when(skipped > 0, |v| {
                v.child(
                    div()
                        .id("live-note")
                        .test_support()
                        .text_size(style::TEXT_SMALL)
                        .text_color(muted)
                        .child(format!("{skipped} earlier lines not shown")),
                )
            })
            .child(
                div()
                    .id("live-body")
                    .test_support()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(list)
                    .vertical_scrollbar(&self.live_scroll),
            )
            .into_any_element()
    }

    // Until the first byte of a live response arrives, say so and offer Stop here, so a quiet
    // subscription is not a blank list. Stop keeps the run, unlike Cancel while sending.
    fn waiting_row(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.send.read(cx).live().is_some_and(|l| l.bytes() > 0) {
            return None;
        }
        let send = self.send.clone();
        Some(
            h_flex()
                .id("response-live-waiting")
                .test_support()
                .aria_label("Waiting for data")
                .flex_none()
                .w_full()
                .items_center()
                .gap(style::SPACE_2)
                .child(
                    div()
                        .text_size(style::TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .child("Waiting for data"),
                )
                .child(
                    Button::new("response-stop")
                        .ghost()
                        .small()
                        .label("Stop")
                        .accessibility_label("Stop the stream")
                        .on_click(move |_, _, cx| send.update(cx, |m, cx| m.stop(cx))),
                )
                .into_any_element(),
        )
    }

    // The Compare tab: a fixed summary row over the one scrolling list of rows.
    fn compare_body(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(state) = &self.compare else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let label = self
            .compare_choices(cx)
            .into_iter()
            .find(|c| c.source == state.source)
            .map_or_else(String::new, |c| c.label);
        let text = match &state.comparison {
            Some(c) => compare_view::summary(c),
            None => "Comparing...".to_owned(),
        };
        let pane = cx.entity().downgrade();
        let summary = h_flex()
            .id("compare-summary")
            .test_support()
            .flex_none()
            .w_full()
            .gap(style::SPACE_2)
            .items_center()
            .child(
                div()
                    .relative()
                    .flex_none()
                    .children(self.compare_menu("compare-source", label, None, cx))
                    .child(width_probe(&self.min.source)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(style::TEXT_SMALL)
                    .text_color(muted)
                    .child(text),
            )
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(
                        Button::new("compare-close")
                            .ghost()
                            .compact()
                            .icon(IconName::Close)
                            .tooltip("Close comparison")
                            .accessibility_label("Close comparison")
                            .on_click(move |_, _, cx| {
                                let _ = pane.update(cx, |p, cx| p.close_compare(cx));
                            }),
                    )
                    .child(width_probe_plus(&self.min.close, style::SPACE_2 * 2.)),
            );
        let list = match &state.comparison {
            None => div().into_any_element(),
            Some(_) => {
                let rows = Arc::clone(&state.rows);
                let palette = ComparePalette::new(&state.diff_rows, window, cx);
                let pane = cx.entity().downgrade();
                let toggle: Toggle = Rc::new({
                    let pane = pane.clone();
                    move |_, cx| {
                        let _ = pane.update(cx, |p, cx| p.toggle_volatile(cx));
                    }
                });
                let reveal: RevealHeader = Rc::new(move |name, _, cx| {
                    let _ = pane.update(cx, |p, cx| p.toggle_compare_reveal(name, cx));
                });
                let actions = RowActions { toggle, reveal };
                let list = uniform_list(
                    "compare-rows",
                    rows.len(),
                    move |range: std::ops::Range<usize>, window: &mut Window, cx: &mut App| {
                        range
                            .map(|ix| compare_row(ix, &rows[ix], &palette, &actions, window, cx))
                            .collect::<Vec<_>>()
                    },
                )
                .track_scroll(&state.scroll)
                .size_full()
                // The overlay scrollbar draws over the rows' right end, so keep the eyes clear.
                .pr(style::SCROLLBAR_GUTTER);
                div()
                    .id("compare-list")
                    .test_support()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(list)
                    .vertical_scrollbar(&state.scroll)
                    .into_any_element()
            }
        };
        v_flex()
            .id("response-compare-tab")
            .test_support()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(style::SPACE_1)
            .child(summary)
            .child(list)
            .into_any_element()
    }
}

// A source's menu label, with the reason it cannot be picked when it cannot.
fn choice_text(choice: &SourceChoice) -> String {
    match choice.disabled {
        Some(why) => format!("{} ({why})", choice.label),
        None => choice.label.clone(),
    }
}

impl Render for ResponsePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let compact = self.compact.get();
        if self.body.read(cx).compact() != compact {
            self.body.update(cx, |b, cx| b.set_compact(compact, cx));
        }
        let error = self.error_view(cx);
        if error != self.details_for {
            self.details_open = false;
            self.details_for = error.clone();
        }
        let content = match (self.send.read(cx).state(), &error) {
            // `response-idle` names the whole idle view; the empty state inside has its own id.
            (SendState::Idle, _) => div()
                .id("response-idle")
                .test_support()
                .size_full()
                .child(
                    ui::empty_state(
                        "response-idle-actions",
                        RESPONSE_HINT,
                        &[("Send", &SendRequest)],
                    )
                    .key_context(crate::request_editor::KEY_CONTEXT),
                )
                .into_any_element(),
            (SendState::Streaming, _) => self.live_view(window, cx),
            (SendState::Sending, _) => {
                let (waiting, elapsed) = {
                    let m = self.send.read(cx);
                    (m.is_waiting(), m.elapsed())
                };
                let send = self.send.clone();
                // While a dialog waits on the user nothing is timed, so neither spinner nor
                // clock shows; before the run starts the spinner shows without a clock.
                let status = match waiting {
                    true => div()
                        .id("response-waiting")
                        .test_support()
                        .aria_label("Waiting for your answer")
                        .text_size(style::TEXT_SMALL)
                        .text_color(cx.theme().muted_foreground)
                        .child("Waiting for your answer")
                        .into_any_element(),
                    false => v_flex()
                        .items_center()
                        .gap(style::SPACE_2)
                        .child(
                            div()
                                .id("response-spinner")
                                .test_support()
                                .child(Spinner::new()),
                        )
                        .when_some(elapsed, |col, elapsed| {
                            col.child(
                                div()
                                    .id("response-elapsed")
                                    .test_support()
                                    .text_size(style::TEXT_SMALL)
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format_time(elapsed)),
                            )
                        })
                        .into_any_element(),
                };
                v_flex()
                    .id("response-sending")
                    .test_support()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap(style::SPACE_2)
                    .child(status)
                    .child(
                        Button::new("response-cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(move |_, _, cx| send.update(cx, |m, cx| m.cancel(cx))),
                    )
                    .into_any_element()
            }
            // With no response there is nothing for the tabs to show, so the error stands alone
            // under the History dropdown.
            (SendState::Done(result), Some(e)) if result.response.is_none() => {
                self.under_history(self.error_element(e, cx), cx)
            }
            (SendState::Done(result), _) => {
                let result = Arc::clone(result);
                // One header row that never wraps: the tabs on the left, the status facts
                // right-aligned. A short row cuts the address; once the whole tabs and the facts
                // no longer fit, the pane is compact, the tabs on a whole row over folded facts.
                let header = match compact {
                    true => v_flex(),
                    false => h_flex().justify_end().items_center().overflow_hidden(),
                };
                let header = header
                    .w_full()
                    .gap(style::SPACE_2)
                    .child(
                        div()
                            .id("response-tab-strip")
                            .test_support()
                            .relative()
                            .when(compact, |strip| strip.w_full())
                            // Wide, the floor guarantees room for every tab, so the strip never
                            // shrinks; the address gives way instead.
                            .when(!compact, |strip| {
                                strip.flex_grow(1.).max_w_full().flex_shrink_0()
                            })
                            .on_scroll(ui::wheel_sideways(&self.tab_scroll))
                            .child(self.tab_bar(window, cx))
                            .child(self.tabs_probe())
                            .children(self.tab_fades.render(
                                "response-tab-strip",
                                &self.tab_scroll,
                                self.strip_tabs().len(),
                                cx,
                            ))
                            .child(ui::overflow_watch(
                                &self.tab_scroll,
                                ui::strip_overflows(&self.tab_scroll),
                            ))
                            .child(
                                self.tab_reveal
                                    .watch(&self.tab_scroll, self.tab_position(self.tab)),
                            )
                            .children(self.tab_focus_reveal.watch_focused(
                                &self.tab_scroll,
                                self.focused_tab_position(window),
                            )),
                    )
                    .children(self.status_line(&result, window, cx));
                // Below the full-width strip the content takes the ring room the strip's spacer
                // gives the first tab, so it lines up with the request column's content.
                let below = v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .gap_2()
                    .ring_room(RingSides::LEFT)
                    .children(self.stream_error_row(cx))
                    .children(self.warnings(cx).into_iter().enumerate().map(|(ix, w)| {
                        div()
                            .id(("response-warning", ix))
                            .test_support()
                            .aria_label(w.clone())
                            .text_size(style::TEXT_SMALL)
                            .text_color(cx.theme().muted_foreground)
                            .child(w)
                    }))
                    .children(self.ask_again_button(&result, cx))
                    .child(self.tab_body(&result, error, window, cx));
                v_flex()
                    .size_full()
                    .gap_2()
                    .child(header)
                    .child(below)
                    .into_any_element()
            }
            (SendState::Cancelled | SendState::Failed(_), Some(e)) => {
                self.under_history(self.error_element(e, cx), cx)
            }
            (SendState::Cancelled | SendState::Failed(_), None) => div().into_any_element(),
        };
        // A press anywhere in the pane focuses it first, so the Send and Retry buttons
        // dispatch `SendRequest` from inside the editor that hosts the pane.
        let focus = self.focus.clone();
        self.tick_age(cx);
        v_flex()
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .capture_any_mouse_down(move |_, window, cx| window.focus(&focus, cx))
            .child(content)
            .child(self.width_watch(window))
    }
}

/// An assertion's verdict word and its line: the expression and operator, the value when
/// present, then a fail's actual value and detail or a skip's reason.
pub fn assertion_line(o: &AssertionOutcome) -> (&'static str, String) {
    line_scrubbed(o, |text| text.to_owned())
}

/// As [`assertion_line`] for an outcome of `r`, with the actual value, detail and reason
/// redacted; a server can echo a secret into the left side, while the rest is the file's text.
pub fn redacted_assertion_line(o: &AssertionOutcome, r: &RequestResult) -> (&'static str, String) {
    line_scrubbed(o, |text| r.redact(text))
}

fn line_scrubbed(o: &AssertionOutcome, scrub: impl Fn(&str) -> String) -> (&'static str, String) {
    let a = o.assertion();
    // The editor's plain operator words, so a result reads as the row that set it up.
    let operator = crate::assertions_section::operator_label(a.operator());
    let mut text = format!("{} {operator}", a.expression());
    if let Some(value) = a.value() {
        text.push_str(&format!(" {value}"));
    }
    let verdict = match o.verdict() {
        Verdict::Pass => "pass",
        Verdict::Fail { actual, detail, .. } => {
            text.push_str(&format!(": actual {}", scrub(actual)));
            let parse_error = detail
                .as_deref()
                .and_then(|d| parse_error_sentence(actual, d));
            if let Some(detail) = detail {
                let shown = match parse_error {
                    Some(sentence) => sentence.to_owned(),
                    None => ui::plain_quotes(&scrub(detail)),
                };
                text.push_str(&format!("; {shown}"));
            }
            // Unquoted text reads as a string, which surprises when a number or expression
            // was meant, so the line says how the value was read.
            if parse_error.is_none() && a.compares_as_text() {
                text.push_str(match actual.parse::<f64>() {
                    Ok(_) => "; the expected value is compared as text, but the actual value is a number",
                    Err(_) => "; the expected value is compared as text, as written; quote it to make that explicit",
                });
            }
            "fail"
        }
        Verdict::Skipped { reason, .. } => {
            text.push_str(&format!(": {}", ui::plain_quotes(&scrub(reason))));
            "skipped"
        }
        _ => "skipped",
    };
    (verdict, text)
}

// A script engine's parse error on a check, in plain words. Only a failed right side leaves a
// real actual value; a failed left side reads "undefined", as an undefined value does too.
fn parse_error_sentence(actual: &str, detail: &str) -> Option<&'static str> {
    detail.starts_with("SyntaxError").then_some(match actual {
        "undefined" => "the check is not valid JavaScript",
        _ => "the expected value is not valid JavaScript",
    })
}

/// The script engine's own message that [`assertion_line`] put into plain words, if it did.
pub fn assertion_engine_text(o: &AssertionOutcome) -> Option<&str> {
    match o.verdict() {
        Verdict::Fail {
            actual,
            detail: Some(detail),
            ..
        } => parse_error_sentence(actual, detail).map(|_| detail.as_str()),
        _ => None,
    }
}

/// A script test entry's verdict word and its line: the name, then a fail's detail or a
/// skip's reason; an entry standing for a failed script reads as "error" with its message.
pub fn test_line(t: &TestOutcome) -> (&'static str, String) {
    let (word, message) = match &t.verdict {
        Verdict::Pass => ("pass", None),
        Verdict::Fail { detail, .. } => ("fail", detail.as_deref()),
        Verdict::Skipped { reason, .. } => ("skipped", Some(reason.as_str())),
        _ => ("skipped", None),
    };
    let word = if t.script_error { "error" } else { word };
    let text = match message {
        Some(m) => format!("{}: {}", t.name, ui::plain_quotes(m)),
        None => t.name.clone(),
    };
    (word, text)
}

/// The script a test entry came from, as the Tests tab names it.
pub fn test_source(phase: ScriptPhase) -> &'static str {
    match phase {
        ScriptPhase::PreRequest => "Pre-request script",
        ScriptPhase::PostResponse => "Post-response script",
        _ => "Test script",
    }
}

/// "N passed, N failed, N skipped" over the entries that are not script errors; `None` when
/// there are no entries at all.
pub fn test_summary(tests: &[TestOutcome]) -> Option<String> {
    if tests.is_empty() {
        return None;
    }
    // Count by the same verdict words the rows show, so the two never disagree.
    let count = |word| tests.iter().filter(|t| test_line(t).0 == word).count();
    Some(format!(
        "{} passed, {} failed, {} skipped",
        count("pass"),
        count("fail"),
        count("skipped")
    ))
}

// `text` with its first letter upper-cased, unless it starts with a camelCase file key such
// as `encodeUrl`, whose case is part of its name.
fn sentence_case(text: &str) -> String {
    let first = text.split([' ', ':']).next().unwrap_or_default();
    if first.chars().skip(1).any(|c| c.is_ascii_uppercase()) {
        return text.to_owned();
    }
    crate::import_report::failure_text(text)
}

/// Whether the Tests tab shows "No tests": the request passed and had no check at all to
/// reach a verdict; a request whose checks were all skipped shows its total instead.
pub fn shows_no_tests(result: &RequestResult) -> bool {
    result.has_no_checks()
}

/// The total line over assertions and tests together: "N passed, N failed", then skipped
/// and errored only when non-zero; `None` when [`shows_no_tests`].
pub fn tests_total(result: &RequestResult) -> Option<String> {
    if shows_no_tests(result) {
        return None;
    }
    let c = check_counts(result);
    let mut line = format!("{} passed, {} failed", c.passed, c.failed);
    for (count, shown) in [(c.skipped, "skipped"), (c.errored, "errored")] {
        if count > 0 {
            line.push_str(&format!(", {count} {shown}"));
        }
    }
    Some(line)
}

/// A result's assertions and script tests counted by verdict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckCounts {
    /// Passed checks.
    pub passed: usize,
    /// Failed checks.
    pub failed: usize,
    /// Skipped checks.
    pub skipped: usize,
    /// Script-error entries.
    pub errored: usize,
}

/// Counts `result`'s checks by the verdict words their rows show, so a total built from
/// these never disagrees with the rows.
pub fn check_counts(result: &RequestResult) -> CheckCounts {
    let words = (result.assertions.iter().map(|o| assertion_line(o).0))
        .chain(result.tests.iter().map(|t| test_line(t).0));
    let mut c = CheckCounts::default();
    for word in words {
        match word {
            "pass" => c.passed += 1,
            "fail" => c.failed += 1,
            "error" => c.errored += 1,
            _ => c.skipped += 1,
        }
    }
    c
}

/// A hop's request body as the Timeline shows it: `None` for no body, the text when it is
/// UTF-8 and was kept whole, else "<size> not shown".
pub fn hop_body_text(body: &[u8], size: u64) -> Option<String> {
    if size == 0 {
        return None;
    }
    let whole = size <= HOP_BODY_LIMIT as u64;
    match std::str::from_utf8(body) {
        Ok(text) if whole => Some(text.to_owned()),
        _ => Some(format!("{} not shown", format_size(size))),
    }
}

/// "N passed, N failed, N skipped" over `outcomes`; `None` when there are none.
pub fn assertion_summary(outcomes: &[AssertionOutcome]) -> Option<String> {
    if outcomes.is_empty() {
        return None;
    }
    // Count by the same verdict words the rows show, so the two never disagree.
    let count = |word| {
        outcomes
            .iter()
            .filter(|o| assertion_line(o).0 == word)
            .count()
    };
    Some(format!(
        "{} passed, {} failed, {} skipped",
        count("pass"),
        count("fail"),
        count("skipped")
    ))
}

/// The colour of a verdict word from `assertion_line` or `test_line`: success, danger,
/// else muted.
pub fn verdict_colour(verdict: &str, cx: &App) -> Hsla {
    let theme = cx.theme();
    match verdict {
        "pass" => theme.success,
        "fail" | "error" => theme.danger,
        _ => theme.muted_foreground,
    }
}

/// What the Cookies tab lists: the cookies this response set, or the jar for its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CookiesView {
    /// Every Set-Cookie of the shown response, with what the jar did with it.
    #[default]
    Response,
    /// The jar's cookies for the final URL's host.
    Jar,
}

/// One Set-Cookie row of the This response table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCookieCells {
    /// The cookie's name.
    pub name: String,
    /// The cookie's value.
    pub value: String,
    /// The hop that set it, as [`apinata_core::Hop::label`] words it, with secrets masked;
    /// `<phase> script` for a cookie a script wrote.
    pub hop: String,
    /// `Stored`, `Removed` or the reason it was rejected.
    pub result: String,
    /// Whether the jar rejected it.
    pub rejected: bool,
    /// The hop's index in the timeline (`usize::MAX` for a script) and the row's place there.
    pub key: (usize, usize),
}

/// One cookie a failed send's scripts set or removed, as the error view lists it; it has no
/// value, since nothing on an error result shows one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorCookieCells {
    /// The cookie's name, with secrets masked.
    pub name: String,
    /// Where it came from.
    pub from: String,
    /// `Stored`, `Removed` or the reason it was rejected.
    pub result: String,
}

// The jar's verdict on `c` as the Result cell words it, and whether it was a rejection.
fn cookie_verdict(c: &apinata_core::SetCookie) -> (String, bool) {
    match c.outcome {
        CookieOutcome::Stored => ("Stored".to_owned(), false),
        CookieOutcome::Removed => ("Removed".to_owned(), false),
        CookieOutcome::Rejected(why) => (why.to_string(), true),
        _ => ("Unknown".to_owned(), true),
    }
}

// A script row's origin as shown: `<phase> script`.
fn script_from(origin: apinata_core::CookieOrigin<'_>) -> String {
    match origin.phase() {
        Some(phase) => format!("{} script", phase.as_str()),
        None => "script".to_owned(),
    }
}

// The cookie rows of a result with no response: its scripts' rows, which core keeps on it.
fn error_cookie_cells(r: &RequestResult) -> Vec<ErrorCookieCells> {
    (r.set_cookies())
        .map(|(origin, c)| ErrorCookieCells {
            // A value that never parsed has no name, since its split text may be the value.
            name: match c.outcome {
                CookieOutcome::Rejected(
                    CookieRejection::Unparseable | CookieRejection::NotUtf8,
                ) => "(unparsed)".to_owned(),
                _ => r.redact(&c.name),
            },
            from: script_from(origin),
            result: cookie_verdict(c).0,
        })
        .collect()
}

// Every Set-Cookie of `r`'s response as table rows, values unmasked. A script's row reads
// `<phase> script` in the hop column and keys under `usize::MAX`, past any hop index.
fn set_cookie_cells(r: &RequestResult) -> Vec<SetCookieCells> {
    let Some(resp) = r.response.as_ref() else {
        return Vec::new();
    };
    let hops = resp.timeline();
    let mut place = vec![0usize; hops.len()];
    let mut scripted = 0usize;
    (resp.set_cookies())
        .map(|(origin, c)| {
            let (result, rejected) = cookie_verdict(c);
            let hop_ix = origin
                .hop()
                .and_then(|h| hops.iter().position(|x| std::ptr::eq(x, h)));
            let (hop, key) = match hop_ix {
                Some(h) => {
                    place[h] += 1;
                    // Labelled first, so the mask is not percent-encoded by the URL parser.
                    (r.redact(&hops[h].label()), (h, place[h] - 1))
                }
                None => {
                    scripted += 1;
                    (script_from(origin), (usize::MAX, scripted - 1))
                }
            };
            SetCookieCells {
                name: c.name.clone(),
                value: c.value.clone(),
                hop,
                result,
                rejected,
                key,
            }
        })
        .collect()
}

/// The `(index, row)` of each Set-Cookie row whose name, hop or result contains `filter`,
/// ignoring case.
pub fn set_cookie_shown<'a>(
    rows: &'a [SetCookieCells],
    filter: &str,
) -> Vec<(usize, &'a SetCookieCells)> {
    let f = filter.to_lowercase();
    (rows.iter().enumerate())
        .filter(|(_, c)| {
            f.is_empty()
                || [&c.name, &c.hop, &c.result]
                    .iter()
                    .any(|t| t.to_lowercase().contains(&f))
        })
        .collect()
}

/// The shown Set-Cookie rows as `name=value`, hop and result, tab separated, one per line.
pub fn set_cookie_lines(rows: &[SetCookieCells], filter: &str) -> String {
    set_cookie_shown(rows, filter)
        .into_iter()
        .map(|(_, c)| format!("{}={}\t{}\t{}", c.name, c.value, c.hop, c.result))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One cookie as the Cookies table shows it: text cells plus the two flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieCells {
    /// The cookie's name.
    pub name: String,
    /// The cookie's value.
    pub value: String,
    /// The domain it is stored under.
    pub domain: String,
    /// The path it is scoped to.
    pub path: String,
    /// The expiry as [`expires_label`] words it.
    pub expires: String,
    /// Whether it carried `HttpOnly`.
    pub http_only: bool,
    /// Whether it is sent over secure connections only.
    pub secure: bool,
}

/// A cookie's expiry as a coarse countdown from `now`: "Session" without one, "Expired"
/// once passed, otherwise "in N s", "min", "h" or "d", rounded down to the largest unit.
pub fn expires_label(expires: Option<SystemTime>, now: SystemTime) -> String {
    let Some(at) = expires else {
        return "Session".into();
    };
    let secs = match at.duration_since(now) {
        Ok(left) if !left.is_zero() => left.as_secs(),
        _ => return "Expired".into(),
    };
    match secs {
        0..60 => format!("in {secs} s"),
        60..3_600 => format!("in {} min", secs / 60),
        3_600..86_400 => format!("in {} h", secs / 3_600),
        _ => format!("in {} d", secs / 86_400),
    }
}

/// Whether a cookie for `domain` is sent to `host`: an exact match, or for a domain
/// cookie also any subdomain. Case-insensitive.
pub fn cookie_matches(host: &str, domain: &str, host_only: bool) -> bool {
    let (host, domain) = (host.to_ascii_lowercase(), domain.to_ascii_lowercase());
    host == domain || (!host_only && host.ends_with(&format!(".{domain}")))
}

// Whether the response is an event-stream: its Content-Type says so, or it is a subscription's
// 101 transcript, known by its stream end or, once restored without one, by its GraphQL
// subprotocol. An ordinary 101 has neither.
fn is_event_stream_response(r: &Response) -> bool {
    let header = |name: &str, want: &dyn Fn(&str) -> bool| {
        (r.headers().iter()).any(|(k, v)| k.eq_ignore_ascii_case(name) && want(v))
    };
    let graphql = |v: &str| matches!(v.trim(), "graphql-transport-ws" | "graphql-ws");
    (r.status() == 101 && (r.stream_end().is_some() || header("sec-websocket-protocol", &graphql)))
        || header("content-type", &|v| sse::is_event_stream(v))
}

// Whether a finished run is a subscription that failed but still has a response to show.
fn is_stream_error(r: &RequestResult) -> bool {
    r.response.is_some() && matches!(r.error, Some(RunError::Subscription { .. }))
}

fn done(send: &SendModel) -> Option<Arc<RequestResult>> {
    match send.state() {
        SendState::Done(r) => Some(Arc::clone(r)),
        _ => None,
    }
}

/// A select's chevron, drawn after a ghost dropdown button's label or icon in the button's
/// own colour: an svg paints only with a text colour set on itself, never an inherited one.
pub fn chevron(cx: &App) -> Svg {
    svg()
        .path(AssetIcon::ChevronDown.path())
        .size(style::SELECT_CHEVRON)
        .text_color(cx.theme().secondary_foreground)
}

// A dropdown shown as its icon and a chevron, named by `label`, so the header row stays
// short while the menu reads as one. `tipped` gives it the tooltip.
fn icon_dropdown(
    id: &'static str,
    icon: AssetIcon,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    Button::new(id)
        .ghost()
        .compact()
        .accessibility_label(label.into())
        .icon(ui::glyph(id, icon))
        .child(chevron(cx))
}

// Wraps the dropdown `id` in a tooltip of `label`, findable as `{id}-tooltip`. A menu
// trigger's own tooltip does not show in the running app; a wrapper's does, as on
// Export as HAR.
fn tipped(id: &'static str, label: SharedString, dropdown: impl IntoElement) -> AnyElement {
    div()
        .id(SharedString::from(format!("{id}-tip")))
        .flex_none()
        .tooltip(ui::text_tooltip(format!("{id}-tooltip"), label))
        .child(dropdown)
        .into_any_element()
}

/// The glyph of the header's Export as HAR button.
pub const EXPORT_HAR_ICON: AssetIcon = AssetIcon::FileOutput;

/// The Compare dropdown's icon, a diff so it never reads as the layout toggle.
pub const COMPARE_ICON: AssetIcon = AssetIcon::FileDiff;

// "Export as HAR", an icon button: enabled when the response still has the request it
// answered, which a response kept from an earlier session does not.
// The tooltip sits on a wrapper, which shows it whether or not the button is locked; a
// locked one names the action before the reason, since the icon carries no text.
fn export_har_button(response: &Response, cx: &App) -> AnyElement {
    let button = Button::new("response-export-har")
        .ghost()
        .compact()
        .icon(ui::glyph("response-export-har", EXPORT_HAR_ICON))
        .accessibility_label("Export HAR");
    let (button, tip) = match response.timeline().is_empty() {
        true => (
            button.lock(true, cx),
            format!("Export HAR: {}", crate::har_export::NO_REQUEST),
        ),
        false => (
            button.on_click(|_, window, cx| window.dispatch_action(Box::new(ExportHar), cx)),
            "Export the request and response as a HAR file".to_owned(),
        ),
    };
    div()
        .id("response-export-har-tip")
        .flex_none()
        .tooltip(ui::text_tooltip("response-export-har-tooltip", tip))
        .child(button)
        .into_any_element()
}

// "Save response as example": enabled for a whole response under the size limit of a request that
// can hold examples, otherwise disabled with a tooltip saying why (`blocked` first).
fn save_example_button(response: &Response, blocked: Option<&'static str>, cx: &App) -> Button {
    let id = "response-save-example";
    if blocked.is_none() && save_example::can_save(Some(response)) {
        return ui::icon_button(
            id,
            ui::glyph(id, AssetIcon::Bookmark),
            "Save response as example",
            Some(&SaveAsExample),
        );
    }
    let why = if let Some(why) = blocked {
        why
    } else if response.truncated() {
        "A truncated response cannot be saved as an example"
    } else {
        "Responses of 5 MiB or more cannot be saved as examples"
    };
    ui::icon_button(id, ui::glyph(id, AssetIcon::Bookmark), why, None).lock(true, cx)
}

// A button in the error block. The plain fill barely differs from a light pane, so the
// input outline marks its boundary at the 3:1 a control needs.
fn error_action(id: impl Into<ElementId>, cx: &App) -> Button {
    Button::new(id).border_1().border_color(cx.theme().input)
}

// Points the body's download button at the collection of the latest send; without one the
// button is not offered.
fn wire_download(
    body: &Entity<BodyView>,
    download: &Entity<DownloadModel>,
    send: &Entity<SendModel>,
    window: &mut Window,
    cx: &mut App,
) {
    let root = send.read(cx).root().map(Path::to_path_buf);
    let source = root.map(|root| (download.clone(), root));
    body.update(cx, |b, cx| b.set_download(source, window, cx));
}

fn retry_button(cx: &App) -> Button {
    error_action("response-retry", cx)
        .label("Retry")
        .tooltip_with_action(
            "Retry",
            &SendRequest,
            Some(crate::request_editor::KEY_CONTEXT),
        )
        .on_click(|_, window, cx| window.dispatch_action(SendRequest.boxed_clone(), cx))
}

// The intrinsic widths behind `ResponsePane::min_width`, each written by a probe in layout.
// The tab and status widths outlive an error or a resend on purpose, so the divider and the
// axis hold still between sends; only closing a comparison drops its part.
#[derive(Default)]
struct MinWidths {
    tabs: Rc<Cell<Pixels>>,
    // The tabs laid out whole, from the strip's left edge to the last tab's right edge.
    tabs_whole: Rc<Cell<Pixels>>,
    status: Rc<Cell<Pixels>>,
    // The wide status line's fixed groups either side of the address, in rems so a zoom
    // change keeps them true, whether it has an address, and what they were measured for.
    facts_before: Rc<Cell<f32>>,
    facts_after: Rc<Cell<f32>>,
    addressed: Cell<bool>,
    facts_for: Cell<u64>,
    source: Rc<Cell<Pixels>>,
    close: Rc<Cell<Pixels>>,
    pinned: Cell<Option<Pixels>>,
}

impl MinWidths {
    // What the wide status line needs with its address cut to nothing; read while wide only,
    // so the compact switch it feeds cannot oscillate.
    fn facts(&self, window: &Window) -> Pixels {
        let gaps = if self.addressed.get() { 2. } else { 1. };
        let groups = rems(self.facts_before.get() + self.facts_after.get());
        style::resolve(groups, window) + style::resolve(style::SPACE_2, window) * gaps
    }

    // Whether the facts were measured for what the header shows now.
    fn facts_known(&self) -> bool {
        self.facts_before.get() > 0.
    }

    // Forgets the measured facts when the header's content changes, so the floor falls back
    // to `RESPONSE_COMPACT_BELOW` until the pane next lays them out wide.
    fn facts_shown(&self, key: u64) {
        if self.facts_for.replace(key) != key {
            self.facts_before.set(0.);
            self.facts_after.set(0.);
        }
    }

    // The comparison's two buttons; the close cell also holds the gaps around the text.
    fn compare(&self) -> Pixels {
        self.source.get() + self.close.get()
    }
}

// Stores `width` and asks for one more frame when it changed, so the editor re-reads the
// floor; an unchanged width costs nothing.
fn set_width(cell: &Rc<Cell<Pixels>>, width: Pixels, window: &mut Window) {
    if cell.get() != width {
        cell.set(width);
        window.request_animation_frame();
    }
}

// As `width_probe`, storing the width in rems, so it stays true across a zoom change.
fn rem_probe(cell: &Rc<Cell<f32>>) -> impl IntoElement {
    let cell = Rc::clone(cell);
    canvas(
        move |bounds, window, _| {
            let width = bounds.size.width / window.rem_size();
            if cell.get() != width {
                cell.set(width);
                window.request_animation_frame();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

// A marker the size of its `relative` parent that stores the parent's laid-out width.
fn width_probe(cell: &Rc<Cell<Pixels>>) -> impl IntoElement {
    width_probe_plus(cell, rems(0.))
}

// As `width_probe`, adding `extra` resolved at the current zoom.
fn width_probe_plus(cell: &Rc<Cell<Pixels>>, extra: Rems) -> impl IntoElement {
    let cell = Rc::clone(cell);
    canvas(
        move |bounds, window, _| {
            set_width(
                &cell,
                bounds.size.width + style::resolve(extra, window),
                window,
            )
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}
