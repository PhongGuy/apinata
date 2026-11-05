//! `send_request`: resolve one request under the MCP restrictions, send it, and report the
//! exchange with credential headers redacted.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use apinata_core::{
    AssertionOutcome, Collection, ContextOptions, CookieOutcome, CookieRejection, EnvOverlay,
    ExecError, ItemPath, OAuth2Error, RequestFile, RequestResult, ResolveError, ResolvedRequest,
    Response, RuntimeVars, SecretStore, SetCookie, StreamLimits, TokenCache, VarContext,
    VarOutcome, VarResult, Verdict, normalize_host,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::McpOptions;
use crate::redact::{self, Redactor};
use crate::scripts::{self, ConsoleDoc, TestDoc};

/// The response text an agent gets when it names no budget.
pub(crate) const DEFAULT_BODY_BYTES: u64 = 32 * 1024; // a tool-output budget, not a measurement
/// The most response text one result carries, whatever the agent asks.
pub(crate) const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// `send_request`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendArgs {
    /// The request's item path, such as `users/get.yml`.
    pub path: String,
    /// The environment to resolve against; the server's default when absent.
    pub environment: Option<String>,
    /// Runtime variable overrides, applied over the environment.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// The most body text to return, in bytes; 32 KiB when absent, 1 MiB at most.
    pub max_body_bytes: Option<u64>,
    /// Send without running scripts.
    #[serde(default)]
    pub skip_scripts: bool,
    /// For an event-stream, end it after this many seconds; 30 when absent, 1 to 300.
    pub stream_max_seconds: Option<u64>,
    /// For an event-stream, end it after this many events; 10000 when absent, 1 to 100000.
    pub stream_max_events: Option<u64>,
    /// Cap the request's timeout at this many seconds, 1 to 86400. The lower of the request's own
    /// timeout and this value applies; a request with none gets the cap.
    pub timeout_secs: Option<u64>,
    /// Narrow the returned body text to what this expression matches: a JSONPath (RFC 9535) for
    /// a JSON response, an XPath 1.0 for an XML one, by content type. A path that can match
    /// only one value, such as `$[0].name`, returns that value; other JSONPaths return an array
    /// (`[]` for none), and an XPath one line per match, each ending in a newline (empty text
    /// for none). `max_body_bytes` then cuts the matches,
    /// `truncated` says whether it did, and `total_bytes` stays the whole body's size. A body
    /// that is neither JSON nor XML, or a response truncated at capture that no longer parses,
    /// returns no text and `filter_error`. A bad expression is an
    /// error and nothing is sent.
    pub body_filter: Option<String>,
}

/// The run-wide timeout cap for `secs`, which must be 1..=86400 when given.
pub(crate) fn timeout_cap(secs: Option<u64>) -> Result<Option<Duration>, String> {
    match secs {
        Some(s) if !(1..=86_400).contains(&s) => Err(format!(
            "`timeout_secs` is {s}; it must be 1 to 86400; nothing was sent"
        )),
        other => Ok(other.map(Duration::from_secs)),
    }
}

/// The stream limits for a send: `secs` clamped to 1..=300 (default 30) and `events` to
/// 1..=100000 (default 10000).
pub(crate) fn stream_limits(secs: Option<u64>, events: Option<u64>) -> StreamLimits {
    let secs = secs.unwrap_or(30).clamp(1, 300);
    let events = events.unwrap_or(10_000).clamp(1, 100_000);
    StreamLimits::new()
        .max_duration(Some(Duration::from_secs(secs)))
        .max_events(Some(events))
}

/// The stream end label and parsed event count of `response`, both absent for a response that
/// was not streamed.
pub(crate) fn stream_fields(response: &Response) -> (Option<String>, Option<usize>) {
    match response.stream_end() {
        Some(end) => (Some(end.label().to_owned()), Some(response.events().len())),
        None => (None, None),
    }
}

/// One header as sent or received.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct HeaderDoc {
    pub name: String,
    pub value: String,
}

/// The response body as text, with its type and size.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct BodyDoc {
    pub text: Option<String>,
    pub content_type: Option<String>,
    pub total_bytes: u64,
    /// True when `text` was cut to the byte budget (with `body_filter`, the matched text).
    pub truncated: bool,
    /// Why `body_filter` could not be applied; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_error: Option<String>,
}

/// One assertion's verdict: `pass`, `fail` or `skipped`, with why for the last two.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct AssertionDoc {
    pub expression: String,
    pub verdict: String,
    pub message: Option<String>,
}

/// One post-response variable: `set`, `undefined`, `skipped` or `failed`, with why for the
/// last two.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct VarDoc {
    pub name: String,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One hop of the exchange.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct HopDoc {
    pub method: String,
    pub url: String,
    pub status: u16,
    /// "HTTP/1.1", "HTTP/2" or "HTTP/1.0"; null only for a version outside those.
    pub version: Option<String>,
    /// The peer address the hop reached as `ip:port`; through a proxy it is the proxy's.
    /// Absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_addr: Option<String>,
    /// True when the hop went through a proxy, so `remote_addr` is the proxy's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub via_proxy: bool,
    /// The host override the hop used as `name -> address`; absent when DNS answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_override: Option<String>,
    /// Each Set-Cookie the hop carried, in arrival order, with what the jar did with it.
    /// Absent when the hop set none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cookies: Vec<CookieDoc>,
}

/// One Set-Cookie a hop carried, without its value: `stored` when the jar kept it, `removed`
/// when it deleted a stored cookie, `reason` when the jar rejected it.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct CookieDoc {
    /// The cookie's name; absent for a value that never parsed, whose text could be the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `Domain` attribute as sent; absent when it had none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// The `Path` attribute as sent; absent when it had none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The phase whose script set it: `pre-request`, `post-response` or `tests`; absent for a
    /// hop's cookie.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// True when the jar stored the cookie.
    pub stored: bool,
    /// True when the cookie was already expired and deleted the stored one it matched, or a
    /// script's delete or clear removed it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    /// Why the jar rejected the cookie, such as "Secure cookie over http"; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A sent url as the agent may see it: no userinfo, and no known secret.
fn shown_url(url: &str, redactor: &Redactor) -> String {
    redactor.scrub(&apinata_core::redact_url_userinfo(url))
}

/// `c` as the agent sees it: the value is left out, and the texts a secret could hide in are
/// scrubbed. A value that never parsed has no name, since its split text may be the value.
/// A verdict this crate does not know yet reads as rejected for an unknown reason.
pub(crate) fn cookie_doc(c: &SetCookie, redactor: &Redactor) -> CookieDoc {
    let (stored, removed, reason) = match c.outcome {
        CookieOutcome::Stored => (true, false, None),
        CookieOutcome::Removed => (false, true, None),
        CookieOutcome::Rejected(why) => (false, false, Some(why.to_string())),
        _ => (false, false, Some("Unknown".to_owned())),
    };
    let unparsed = matches!(
        c.outcome,
        CookieOutcome::Rejected(CookieRejection::Unparseable | CookieRejection::NotUtf8)
    );
    CookieDoc {
        name: (!unparsed).then(|| redactor.scrub(&c.name)),
        domain: c.domain.as_deref().map(|d| redactor.scrub(d)),
        path: c.path.as_deref().map(|p| redactor.scrub(p)),
        phase: None,
        stored,
        removed,
        reason,
    }
}

/// What `send_request` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct SendResult {
    pub status: u16,
    pub status_text: String,
    pub duration_ms: u64,
    pub url: String,
    pub request_headers: Vec<HeaderDoc>,
    pub response_headers: Vec<HeaderDoc>,
    pub body: BodyDoc,
    pub assertions: Vec<AssertionDoc>,
    pub post_response_vars: Vec<VarDoc>,
    pub warnings: Vec<String>,
    pub timeline: Vec<HopDoc>,
    /// Each cookie a script of this request set or removed, in write order, with its phase and
    /// what the jar did with it; absent when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub script_cookies: Vec<CookieDoc>,
    /// The answering hop's peer address as `ip:port`; absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_addr: Option<String>,
    /// True when the answering hop went through a proxy, so `remote_addr` is the proxy's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub via_proxy: bool,
    pub tests: Vec<TestDoc>,
    pub console: Vec<ConsoleDoc>,
    /// How an event-stream ended: `closed`, `stopped`, `time_limit`, `event_limit`,
    /// `idle_timeout`, `body_cap` or `error`; absent for an ordinary response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_end: Option<String>,
    /// The count of events parsed from a streamed body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<usize>,
}

/// The variable context for one send: `environment` or the server's default, the launch global
/// environment, the human's `--env-var` and `--global-env-var` values as sensitive, the agent's
/// `vars` as plain, secrets from `store`, read strictly, the session's `runtime` variables,
/// OAuth 2.0 `tokens` and script environment edits (`overlay`). Errors end with "; nothing was sent".
#[allow(clippy::too_many_arguments)]
pub(crate) fn context(
    c: &Collection,
    environment: Option<&str>,
    vars: &BTreeMap<String, String>,
    options: &McpOptions,
    store: Arc<dyn SecretStore>,
    runtime: RuntimeVars,
    tokens: TokenCache,
    overlay: EnvOverlay,
) -> Result<VarContext, String> {
    let mut opts = ContextOptions::new()
        .secrets(store)
        .strict_secrets()
        .runtime_vars(runtime)
        .oauth2_tokens(tokens)
        .env_overlay(overlay)
        .client_certificates(options.certificates().to_vec());
    if let Some(env) = environment.or(options.default_env()) {
        opts = opts.env(env);
    }
    for (name, value) in options.env_vars() {
        opts = opts.sensitive_env_var(name, value);
    }
    if let Some((globals, name)) = options.default_global_env() {
        opts = opts.global_env(Arc::clone(globals), name);
    }
    for (name, value) in options.global_env_vars() {
        opts = opts.sensitive_global_env_var(name, value);
    }
    for (name, value) in vars {
        opts = opts.env_var(name, value);
    }
    VarContext::new(c, opts).map_err(|e| format!("{e}; nothing was sent"))
}

/// A copy of the request at `item`.
pub(crate) fn request_file(c: &Collection, item: &str) -> Result<RequestFile, String> {
    let path = ItemPath::new(item).ok_or_else(|| format!("`{item}` is not an item path"))?;
    c.request(&path)
        .cloned()
        .ok_or_else(|| format!("no request at `{item}`"))
}

/// The Authorization header reqwest builds from `url`'s userinfo: each part percent-decoded,
/// a password that is absent or not UTF-8 left out. Empty when there is no userinfo.
pub(crate) fn url_basic_auth(url: &str) -> Vec<(String, String)> {
    use base64::Engine;
    use percent_encoding::percent_decode_str;
    let Ok(url) = url::Url::parse(url) else {
        return Vec::new();
    };
    let decode = |part: &str| {
        percent_decode_str(part)
            .decode_utf8()
            .ok()
            .map(String::from)
    };
    let Some(user) = decode(url.username()) else {
        return Vec::new();
    };
    let password = url.password().and_then(decode);
    if user.is_empty() && password.is_none() {
        return Vec::new();
    }
    let pair = format!("{user}:{}", password.unwrap_or_default());
    let token = base64::engine::general_purpose::STANDARD.encode(pair);
    vec![("authorization".to_owned(), format!("Basic {token}"))]
}

/// The explicit `Host` header value when it names a host other than the URL's.
pub(crate) fn fronted(req: &ResolvedRequest) -> Option<String> {
    let url_host = normalize_host(req.url());
    req.headers()
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .find(|(_, value)| normalize_host(value) != url_host)
        .map(|(_, value)| value.clone())
}

/// A resolve failure as one line. A variable whose name could reveal part of a sensitive
/// value is not named.
pub(crate) fn resolve_message(err: &ResolveError, file: &RequestFile, ctx: &VarContext) -> String {
    resolve_line(err, |name| ctx.name_inside_sensitive_value(file, name))
}

/// A resolve failure as one line, withholding the variable's name, and any hint naming it,
/// wherever `hidden` says naming it could reveal a sensitive value.
pub(crate) fn resolve_line(err: &ResolveError, hidden: impl Fn(&str) -> bool) -> String {
    match err {
        ResolveError::UndefinedVariable { name, .. } if hidden(name) => {
            "a variable written inside a secret value is not defined; nothing was sent".to_owned()
        }
        ResolveError::CyclicVariable { name } if hidden(name) => {
            "a variable written inside a secret value refers to itself; nothing was sent".to_owned()
        }
        _ => match err.mcp_hint() {
            Some(hint) => format!("{err}; nothing was sent; {hint}"),
            None => format!("{err}; nothing was sent"),
        },
    }
}

/// A failed token preparation as one line. Send failures go to the host policy instead, which
/// files approvals, so they never reach this.
pub(crate) fn oauth2_message(err: &OAuth2Error, file: &RequestFile, ctx: &VarContext) -> String {
    match err {
        OAuth2Error::Resolve(e) => resolve_message(e, file, ctx),
        other => format!("{other}; nothing was sent"),
    }
}

/// A failed send as one line, a connect failure with its plain-words hint. A body file that is
/// missing, unreadable or outside the collection gets one message, so the agent cannot probe
/// which files exist outside it.
pub(crate) fn exec_message(err: &ExecError) -> String {
    match err {
        ExecError::FileOutsideCollection { path, .. } | ExecError::File { path, .. } => format!(
            "body file `{}` is missing, unreadable or outside the collection; nothing was sent",
            path.display()
        ),
        other => other.hinted_line().unwrap_or_else(|| other.to_string()),
    }
}

/// The result of `r`'s exchange `response` for `file`: the last hop's request headers and the
/// response headers with credentials redacted, the body text scrubbed by `redactor` and cut to
/// `max_body` bytes, then the context's warnings, the send's and any URL and params query
/// mismatch in `file`, and the script tests and console.
pub(crate) fn send_result(
    file: &RequestFile,
    ctx: &VarContext,
    r: &RequestResult,
    response: &Response,
    max_body: u64,
    filter: Option<&str>,
    redactor: &Redactor,
) -> SendResult {
    let headers = |pairs: Vec<(String, String)>| {
        pairs
            .into_iter()
            .map(|(name, value)| HeaderDoc { name, value })
            .collect()
    };
    let sent = response
        .timeline()
        .last()
        .map(|hop| redact::request_headers(&hop.request_headers))
        .unwrap_or_default();
    let mut warnings: Vec<String> = ctx.warnings().to_vec();
    warnings.extend(r.warnings.iter().map(ToString::to_string));
    if let Some(mismatch) = file.query_mismatch() {
        warnings.push(format!(
            "URL query and params disagree on {}; {}",
            mismatch.names.join(", "),
            crate::dto::QUERY_MISMATCH_NOTE
        ));
    }
    let (stream_end, events) = stream_fields(response);
    SendResult {
        status: response.status(),
        status_text: response.status_text().to_owned(),
        duration_ms: u64::try_from(response.response_time().as_millis()).unwrap_or(u64::MAX),
        url: shown_url(response.url(), redactor),
        request_headers: headers(sent),
        response_headers: headers(redact::response_headers(response.headers())),
        body: body(response, max_body, filter, redactor),
        assertions: r
            .assertions
            .iter()
            .map(|o| assertion_doc(o, file, ctx))
            .collect(),
        post_response_vars: r.post_response_vars.iter().map(var_doc).collect(),
        warnings,
        timeline: response
            .timeline()
            .iter()
            .map(|hop| HopDoc {
                method: hop.method.clone(),
                url: shown_url(&hop.url, redactor),
                status: hop.status,
                version: hop.version().map(|v| v.to_string()),
                remote_addr: hop.remote_addr().map(|a| a.to_string()),
                via_proxy: hop.via_proxy(),
                host_override: hop.override_from().map(str::to_owned),
                cookies: hop
                    .cookies
                    .iter()
                    .map(|c| cookie_doc(c, redactor))
                    .collect(),
            })
            .collect(),
        script_cookies: (r.set_cookies())
            .filter_map(|(origin, c)| {
                let phase = origin.phase()?;
                let doc = cookie_doc(c, redactor);
                Some(CookieDoc {
                    phase: Some(phase.as_str().to_owned()),
                    ..doc
                })
            })
            .collect(),
        remote_addr: response.remote_addr().map(|a| a.to_string()),
        via_proxy: response.via_proxy(),
        tests: scripts::test_docs(r),
        console: scripts::console_docs(r),
        stream_end,
        events,
    }
}

/// `r`'s body for the agent: binary as the app decides it (invalid UTF-8 with no declared charset)
/// is type and size only; text is scrubbed whole, then cut, so no cut leaves part of a secret.
/// With a `filter` the scrubbed text is narrowed first and the cut applies to the matches.
pub(crate) fn body(
    r: &Response,
    max_body: u64,
    filter: Option<&str>,
    redactor: &Redactor,
) -> BodyDoc {
    let content_type = r
        .headers()
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone());
    // An error with no length is a final character the executor's cap cut short, not binary.
    let invalid = std::str::from_utf8(r.body()).is_err_and(|e| e.error_len().is_some());
    let charset = content_type
        .as_deref()
        .is_some_and(|t| t.to_ascii_lowercase().contains("charset="));
    let max = usize::try_from(max_body.min(MAX_BODY_BYTES)).unwrap_or(usize::MAX);
    let binary = invalid && !charset;
    let mut filter_error = None;
    let (text, truncated) = match filter {
        _ if binary => {
            filter_error = filter.map(|_| {
                apinata_core::BodyFilterError::Unfilterable {
                    content_type: Some("binary".to_owned()),
                }
                .to_string()
            });
            (None, r.truncated())
        }
        None => {
            let (text, cut) = redactor.scrub_cut(r.text(), max, r.truncated());
            (Some(text), cut)
        }
        Some(expression) => {
            let scrubbed = redactor.scrub(r.text());
            match apinata_core::filter_body(content_type.as_deref(), &scrubbed, expression) {
                Ok(matched) => {
                    let (text, cut) = redactor.scrub_cut(&matched, max, false);
                    (Some(text), cut)
                }
                Err(e) => {
                    filter_error = Some(e.to_string());
                    (None, false)
                }
            }
        }
    };
    BodyDoc {
        text,
        content_type,
        total_bytes: r.size(),
        truncated,
        filter_error,
    }
}

/// One assertion outcome as the agent sees it.
pub(crate) fn assertion_doc(
    o: &AssertionOutcome,
    file: &RequestFile,
    ctx: &VarContext,
) -> AssertionDoc {
    let (verdict, message) = match o.verdict() {
        Verdict::Pass => ("pass", None),
        Verdict::Fail { actual, detail, .. } => {
            // Core names a variable it could not use as `{{name}}`; withhold one that could
            // reveal part of a sensitive value.
            let named = |d: &str| {
                let (_, rest) = d.split_once("`{{")?;
                rest.split_once("}}`").map(|(name, _)| name.to_owned())
            };
            let withheld = detail
                .as_deref()
                .and_then(named)
                .is_some_and(|name| ctx.name_inside_sensitive_value(file, &name));
            let detail = if withheld {
                Some("a variable written inside a secret value could not be used")
            } else {
                detail.as_deref()
            };
            let message = match detail {
                Some(detail) => format!("actual {actual}: {detail}"),
                None => format!("actual {actual}"),
            };
            ("fail", Some(message))
        }
        Verdict::Skipped { reason, .. } => ("skipped", Some(reason.clone())),
        _ => ("skipped", Some("unsupported in MCP".to_owned())),
    };
    AssertionDoc {
        expression: o.assertion().expression().to_owned(),
        verdict: verdict.to_owned(),
        message,
    }
}

/// One post-response variable outcome as the agent sees it.
pub(crate) fn var_doc(o: &VarOutcome) -> VarDoc {
    // A result this crate does not know yet reads as skipped without a reason.
    let (result, reason) = match o.result() {
        VarResult::Set => ("set", None),
        VarResult::Undefined => ("undefined", None),
        VarResult::Skipped { reason } => ("skipped", Some(reason.clone())),
        VarResult::Failed { error } => ("failed", Some(error.clone())),
        _ => ("skipped", None),
    };
    VarDoc {
        name: o.name().to_owned(),
        result: result.to_owned(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apinata_core::{SecretError, SecretKey};

    /// A keychain that never answers in time, as `TimeoutStore` reports one.
    struct Locked;

    impl SecretStore for Locked {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Err(SecretError::Unavailable {
                message: "keychain unavailable: no answer within 10 s".to_owned(),
            })
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            unreachable!("sends never write secrets")
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            unreachable!("sends never delete secrets")
        }
    }

    #[test]
    fn a_connect_failure_reads_as_the_shared_hint_line() {
        use apinata_core::ConnectCause;
        let err = ExecError::connect("http://127.0.0.1:1/x", "tcp refused", ConnectCause::Refused);
        assert_eq!(
            exec_message(&err),
            "http://127.0.0.1:1/x: Connection refused: Nothing is listening at 127.0.0.1:1. \
             Is the server running? (tcp refused)"
        );
        assert_eq!(
            exec_message(&ExecError::timeout("http://a.test/")),
            ExecError::timeout("http://a.test/").to_string()
        );
    }

    #[test]
    fn a_locked_keychain_fails_the_context_so_nothing_is_sent() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(
            dir.path().join("opencollection.yml"),
            "opencollection: \"1.0.0\"\ninfo:\n  name: shop\n",
        )
        .expect("write");
        std::fs::create_dir(dir.path().join("environments")).expect("mkdir");
        std::fs::write(
            dir.path().join("environments/dev.yml"),
            "name: dev\nvariables:\n  - name: token\n    secret: true\n",
        )
        .expect("write");
        let c = Collection::open(dir.path()).expect("opens");
        let options = McpOptions::new(dir.path());
        let err = context(
            &c,
            Some("dev"),
            &BTreeMap::new(),
            &options,
            Arc::new(Locked),
            RuntimeVars::new(),
            TokenCache::in_memory(),
            EnvOverlay::new(),
        )
        .expect_err("refused");
        assert!(err.contains("keychain unavailable"), "{err}");
        assert!(err.ends_with("nothing was sent"), "{err}");
    }

    #[test]
    fn a_body_cut_by_the_executor_cap_mid_secret_ends_in_the_marker() {
        let mut redactor = Redactor::default();
        redactor.add_value("Zq7Wk9Px");
        // The executor stopped reading inside the secret; the tool's own cap is far away.
        let text = "aaaa Zq7W";
        let response = Response::new(200)
            .with_body(text.as_bytes().to_vec(), text)
            .with_truncated(9);
        let doc = body(&response, DEFAULT_BODY_BYTES, None, &redactor);
        assert_eq!(doc.text.as_deref(), Some("aaaa <secret>"));
        assert!(doc.truncated);
    }
}
