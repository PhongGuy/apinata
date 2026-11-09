//! Turns a request file into what would be sent, with every variable expanded.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use url::Url;
use yaml_serde::{Mapping, Value};

use crate::aws_profile::profile_keys;
use crate::collection::Collection;
use crate::context::{Defaults, VarContext};
use crate::decomment::decomment;
use crate::document::read_regular_text;
use crate::dynamic::{Generator, mock};
use crate::edgegrid;
use crate::edit::KeyValue;
use crate::edit::omitted_names;
use crate::edit::{
    AuthSpec, EdgeGridSpec, OAuth1KeyKind, OAuth1Placement, OAuth1SignatureMethod, OAuth1Spec,
    OAuth2Spec, TokenPlacement, TokenSource,
};
use crate::edit::{Param, ParamKind};
use crate::encode_url::{authority_span, encode_url};
use crate::error::ResolveError;
use crate::header_check::header_problem;
use crate::interpolate::{Tracker, Vars, interpolate, interpolate_tracked};
use crate::item_path::ItemPath;
use crate::js;
use crate::multipart::Part;
use crate::network::{
    CertPath, CertificateFiles, CertificateKind, CollectionProxy, NetworkBasis, PacWarning,
    RequestNetwork, RequestProxy, ResolvedCertificate, ResolvedProxy, domain_matches,
    resolve_host_overrides,
};
use crate::oauth1;
use crate::oauth2::OAuth2Error;
use crate::oauth2::config::TokenConfig;
use crate::post_response::has_other_actions;
use crate::prompt::missing_labels;
use crate::request::{RequestFile, RequestKind};
use crate::sigv4::{SigV4, infer_scope};
use crate::url_params::table_escape_spots;

/// A request with its variables resolved, ready to send.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct ResolvedRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    /// What stops a header from being sent, worded without its value; see `header_problem`.
    header_problem: Option<String>,
    body: Body,
    settings: Settings,
    warnings: Vec<Warning>,
    /// Non-empty values substituted from sensitive variables, deduplicated, first-substituted order.
    sensitive_values: Vec<String>,
    /// Whether a sensitive value reaches the host; see `secret_reaches_host`.
    secret_host: bool,
    /// The host as the request file templated it.
    template_host: String,
    /// The URL text as interpolated, before parsing turns a non-ASCII host into punycode.
    url_text: String,
    /// The files the body reads, in body order.
    files: Vec<FileRef>,
    network: RequestNetwork,
    /// Auth the executor applies per hop; never printed by `Debug`.
    send_auth: Option<SendAuth>,
    /// The effective auth's credentials, kept out of logs; never printed by `Debug`.
    auth_secrets: Vec<String>,
    /// Values computed for this send (the OAuth2 token); never printed by `Debug`.
    derived: Vec<String>,
    /// Whether the body was built from a GraphQL body spec.
    graphql: bool,
}

/// Auth that cannot be a header at resolve: digest answers a challenge, WSSE needs a fresh nonce
/// per send, NTLM answers a challenge, and SigV4, OAuth 1.0 and EdgeGrid sign each hop.
#[derive(Clone, PartialEq)]
pub(crate) enum SendAuth {
    Digest {
        username: String,
        password: String,
    },
    AwsV4(SigV4),
    OAuth1(Box<oauth1::Signer>),
    Wsse {
        username: String,
        password: String,
    },
    Ntlm {
        username: String,
        password: String,
        domain: String,
    },
    EdgeGrid(Box<edgegrid::Signer>),
}

/// A file the body reads: the path read, the path before joining to the root, and the text as
/// the request file wrote it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileRef {
    pub(crate) path: PathBuf,
    pub(crate) given: PathBuf,
    pub(crate) written: PathBuf,
}

/// One request's lookup map, the tracker recording its sensitive substitutions and the
/// generator its `{{$name}}` values come from.
struct Scope {
    vars: Vars,
    tracker: Tracker,
    generator: RefCell<Generator>,
}

impl Scope {
    /// Expands `text` over the map, recording any sensitive substitution.
    fn expand(&self, text: &str) -> Result<String, ResolveError> {
        interpolate_tracked(text, &self.vars, Some(&self.tracker), &self.generator)
    }
}

/// The request's transport settings after Bruno's coercions.
#[derive(Debug, Clone, PartialEq)]
struct Settings {
    timeout: Option<Duration>,
    follow_redirects: bool,
    max_redirects: u32,
    forward_authorization_header: bool,
    encode_url: bool,
    omit_headers: Vec<String>,
}

/// Something in the request that Apinata does not honour.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// The collection, a folder or the request has scripts, which are not executed.
    ScriptsNotRun,
    /// The collection, a folder or the request has actions other than post-response
    /// variables, which are not executed.
    ActionsNotRun,
    /// The request sets `encodeUrl: false`, but the URL is normalized anyway.
    EncodeUrlIgnored,
    /// The URL's userinfo was not sent because the request has path params.
    UrlCredentialsDropped,
    /// The OAuth 2.0 token has expired and the auth's settings keep it.
    OAuth2TokenExpired,
    /// A script named a next request that is not in this run, so the run ended.
    NextRequestNotFound,
    /// Scripts jumped between requests more than 10,000 times, so the run ended.
    NextRequestLoop,
    /// AWS SigV4 auth lacks an access key, a secret key or a service, so the request is unsigned.
    AwsV4Unsigned,
    /// The AWS profile could not be read, so the auth fields were used.
    AwsProfileUnavailable,
    /// A proxy auto-config failure sent hops directly.
    ProxyAutoConfig(PacWarning),
    /// NTLM to an http target through an http(s) proxy ended in 401, which a proxy that
    /// switches upstream connections mid-handshake causes as well as wrong credentials.
    NtlmProxyUnauthorized,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Warning::ProxyAutoConfig(w) => {
                return write!(f, "proxy auto-config {w}; connected directly");
            }
            Warning::ScriptsNotRun => "scripts not executed",
            Warning::ActionsNotRun => "actions other than post-response variables are not executed",
            Warning::EncodeUrlIgnored => "encodeUrl: false is not supported; the URL is normalized",
            Warning::UrlCredentialsDropped => {
                "credentials in the URL were not sent because the request has path params"
            }
            Warning::OAuth2TokenExpired => "the OAuth 2.0 token has expired and was sent anyway",
            Warning::NextRequestNotFound => {
                "the next request a script named is not in this run, so the run ended"
            }
            Warning::NextRequestLoop => {
                "scripts jumped between requests more than 10,000 times, so the run ended"
            }
            Warning::AwsV4Unsigned => {
                "AWS SigV4 needs an access key, a secret key and a service; the request is sent unsigned"
            }
            Warning::AwsProfileUnavailable => {
                "the AWS profile could not be read; the auth fields were used"
            }
            Warning::NtlmProxyUnauthorized => {
                "NTLM through an HTTP proxy ended in 401: the credentials may be wrong, or the \
                 proxy may not keep one connection to the server for the handshake; use https \
                 to rule out the proxy"
            }
        })
    }
}

/// A resolved request body.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub enum Body {
    /// No body is sent.
    None,
    /// A JSON, text or XML body, sent as is.
    Text(String),
    /// Form fields, in order; senders encode them as `application/x-www-form-urlencoded`, as
    /// Bruno's `buildFormUrlEncodedPayload` does.
    FormUrlEncoded(Vec<(String, String)>),
    /// Multipart form parts, in order: one per text value or per file path. Files are not
    /// read until the request is sent.
    Multipart(Vec<Part>),
    /// The bytes of one file, read when the request is sent. `content_type` is the selected
    /// entry's type, already set as the `Content-Type` header.
    File {
        /// The file, joined to the collection root unless absolute; never interpolated.
        path: PathBuf,
        /// The entry's interpolated content type, possibly empty.
        content_type: String,
    },
}

impl ResolvedRequest {
    /// Whether the body was built from a GraphQL body spec (file or script-set).
    pub(crate) fn graphql(&self) -> bool {
        self.graphql
    }

    /// The HTTP method, uppercased.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The final URL, ready to send: interpolated, path params filled, `http://` prepended when
    /// it had no scheme, any query apikey set, and normalized as a WHATWG URL parser would.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Why an enabled header cannot be sent (bad name or value), naming it as the file writes it
    /// and never quoting its value. A request with a problem is refused at send.
    pub fn header_problem(&self) -> Option<&str> {
        self.header_problem.as_deref()
    }

    /// Headers in send order: collection, folder and request headers merged, then auth.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The body to send.
    pub fn body(&self) -> &Body {
        &self.body
    }

    /// How long to wait for the response; `None` means no limit.
    pub fn timeout(&self) -> Option<Duration> {
        self.settings.timeout
    }

    /// Whether redirects are followed.
    pub fn follow_redirects(&self) -> bool {
        self.settings.follow_redirects
    }

    /// The most redirects to follow, as configured even when redirects are off.
    pub fn max_redirects(&self) -> u32 {
        self.settings.max_redirects
    }

    /// Whether `Authorization` is kept on a redirect to another origin.
    pub fn forward_authorization_header(&self) -> bool {
        self.settings.forward_authorization_header
    }

    /// The default headers this request switches off, from `settings.omitHeaders`, trimmed.
    pub fn omitted_headers(&self) -> &[String] {
        &self.settings.omit_headers
    }

    /// What Apinata will not honour for this request, in variant order, each at most once.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// The digest or SigV4 auth the executor applies to each hop.
    pub(crate) fn send_auth(&self) -> Option<&SendAuth> {
        self.send_auth.as_ref()
    }

    /// The credentials of the effective auth, as `secret_names::auth_secrets` lists them.
    pub(crate) fn auth_secrets(&self) -> &[String] {
        &self.auth_secrets
    }

    /// The values resolution derived rather than read: the OAuth2 token placed on the request.
    pub(crate) fn derived(&self) -> &[String] {
        &self.derived
    }

    /// The non-empty values resolution substituted from secrets, sensitive overrides, `.env`
    /// and the process environment, without repeats, in first-substituted order.
    pub fn sensitive_values(&self) -> &[String] {
        &self.sensitive_values
    }

    /// The templated host when a sensitive value reaches the host, as `resolve` judged it.
    pub(crate) fn secret_in_host(&self) -> Option<&str> {
        self.secret_host.then_some(self.template_host.as_str())
    }

    /// Joins `values` (what scripts read) to the sensitive values and, when a script changed the
    /// template's host from `written_host`, judges the host again against them; an unchanged
    /// host's placeholders were already judged by the taint-aware tracker. A host found to hold
    /// one is quoted as `<redacted>`, since the edited template may carry the literal secret.
    pub(crate) fn add_sensitive_values(&mut self, values: &[String], written_host: &str) {
        for value in values {
            if !value.is_empty() && !self.sensitive_values.contains(value) {
                self.sensitive_values.push(value.clone());
            }
        }
        if self.template_host == written_host {
            return;
        }
        // The parsed URL holds a non-ASCII host as punycode, so the text before parsing is
        // judged too, as `resolve_prepared` judges it.
        let reaches = [&self.url_text, &self.url]
            .into_iter()
            .any(|text| secret_reaches_host(text, values))
            || Url::parse(&self.url).is_ok_and(|url| secret_in_parsed_host(&url, values));
        if reaches {
            self.secret_host = true;
            self.template_host = "<redacted>".to_owned();
        }
    }

    /// The files the body reads, in body order.
    pub(crate) fn files(&self) -> &[FileRef] {
        &self.files
    }

    /// The proxy and client certificate the collection chose for this request.
    pub fn network(&self) -> &RequestNetwork {
        &self.network
    }
}

/// A request after its auth and body kinds are checked and its headers merged, with every
/// field still a template; scripts edit it before [`resolve_prepared`] interpolates it.
#[derive(Clone)]
#[non_exhaustive]
pub struct PreparedRequest {
    req: RequestFile,
    method: String,
    url: String,
    /// Merged headers with any bearer or apikey header placed as axios would set it.
    headers: Vec<(String, String)>,
    /// The index in `headers` of that auth header, which is dropped if its key expands to empty.
    auth_slot: Option<usize>,
    /// The merged headers before auth, kept until a setter touches the headers, so an untouched
    /// request interpolates in the same order, with the same errors, as the file alone.
    pristine: Option<Vec<(String, String)>>,
    auth: Option<Mapping>,
    body: BodySpec,
    settings: Settings,
    has_settings_block: bool,
}

impl PreparedRequest {
    /// The host as the URL template writes it, before any script edits it.
    pub(crate) fn written_host(&self) -> String {
        template_host(&with_scheme(self.url.clone())).to_owned()
    }

    /// The HTTP method, uppercased.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Sets the method, uppercased.
    pub fn set_method(&mut self, method: impl Into<String>) {
        self.method = method.into().to_ascii_uppercase();
    }

    /// The URL template, before interpolation and path params.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Replaces the URL template.
    pub fn set_url(&mut self, url: impl Into<String>) {
        self.url = url.into();
    }

    /// The merged header templates in send order, bearer and header apikey auth included.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The template of the header `name`, matched case-insensitively; the last one wins, as
    /// it does when the names fold after interpolation.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Sets a header as axios does: the first name matching in any case keeps its spelling and
    /// position and takes the value, and later matches are dropped so they cannot override it.
    pub fn set_header(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        let first = self
            .headers
            .iter()
            .position(|(existing, _)| existing.eq_ignore_ascii_case(&name));
        self.pristine = None;
        match first {
            Some(first) => {
                self.headers[first].1 = value.into();
                self.retain_headers(|i, existing| {
                    i <= first || !existing.eq_ignore_ascii_case(&name)
                });
            }
            None => self.headers.push((name, value.into())),
        }
    }

    /// Removes every header named `name` in any case.
    pub fn remove_header(&mut self, name: &str) {
        self.pristine = None;
        self.retain_headers(|_, existing| !existing.eq_ignore_ascii_case(name));
    }

    /// Replaces all headers, auth headers included.
    pub fn set_headers(&mut self, headers: Vec<(String, String)>) {
        self.pristine = None;
        self.auth_slot = None;
        self.headers = headers;
    }

    /// Keeps the headers `keep` accepts by index and name, following the auth header's index.
    fn retain_headers(&mut self, mut keep: impl FnMut(usize, &str) -> bool) {
        let slot = self.auth_slot.take();
        for (i, header) in std::mem::take(&mut self.headers).into_iter().enumerate() {
            if keep(i, &header.0) {
                if slot == Some(i) {
                    self.auth_slot = Some(self.headers.len());
                }
                self.headers.push(header);
            }
        }
    }

    /// The body template of a JSON, text or XML body; `None` for other kinds.
    pub fn body_text(&self) -> Option<&str> {
        match &self.body {
            BodySpec::Text { data, .. } => Some(data),
            _ => None,
        }
    }

    /// Replaces the body template of a JSON, text or XML body; `false`, changing nothing, for
    /// other kinds.
    pub fn set_body_text(&mut self, text: impl Into<String>) -> bool {
        match &mut self.body {
            BodySpec::Text { data, .. } => {
                *data = text.into();
                true
            }
            _ => false,
        }
    }

    /// A GraphQL body as scripts see it: the file's raw query and variables text as an object,
    /// or what a script or the send left there; `None` for other kinds.
    pub(crate) fn graphql_body(&self) -> Option<serde_json::Value> {
        match &self.body {
            BodySpec::Graphql { query, variables } => {
                Some(serde_json::json!({ "query": query, "variables": variables }))
            }
            BodySpec::GraphqlData(data) => Some(data.clone()),
            _ => None,
        }
    }

    /// Replaces a GraphQL body with a script value; `false`, changing nothing, for other kinds.
    pub(crate) fn set_graphql_body(&mut self, data: serde_json::Value) -> bool {
        let is_graphql = matches!(
            self.body,
            BodySpec::Graphql { .. } | BodySpec::GraphqlData(_)
        );
        if is_graphql {
            self.body = BodySpec::GraphqlData(data);
        }
        is_graphql
    }

    /// After the send, an object GraphQL body becomes the object that was sent, query expanded
    /// and variables parsed, as later phases read it; a text body stays as the script set it.
    pub(crate) fn settle_graphql_body(&mut self, sent: &Body) {
        let object = match &self.body {
            BodySpec::Graphql { .. } => true,
            BodySpec::GraphqlData(data) => data.is_object(),
            _ => false,
        };
        if let (true, Body::Text(text)) = (object, sent)
            && let Ok(data) = serde_json::from_str(text)
        {
            self.body = BodySpec::GraphqlData(data);
        }
    }

    /// Whether the content type, a `Content-Type` header or else the body's default, says JSON.
    pub fn is_json_body(&self) -> bool {
        let default = match &self.body {
            BodySpec::Text { content_type, .. } => *content_type,
            BodySpec::Graphql { .. } | BodySpec::GraphqlData(_) => "application/json",
            _ => "",
        };
        self.header("content-type")
            .unwrap_or(default)
            .to_ascii_lowercase()
            .contains("json")
    }

    /// How long to wait for the response; `None` means no limit.
    pub fn timeout(&self) -> Option<Duration> {
        self.settings.timeout
    }

    /// Sets how long to wait for the response; `None` means no limit.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.settings.timeout = timeout;
    }

    /// Sets the most redirects to follow.
    pub fn set_max_redirects(&mut self, max: u32) {
        self.settings.max_redirects = max;
    }

    /// The auth that applies after inheritance: `none`, `basic`, `bearer`, `apikey`, `oauth2`,
    /// `digest`, `awsv4`, `oauth1`, `wsse`, `ntlm` or `akamai-edgegrid`.
    pub fn auth_mode(&self) -> &str {
        match &self.auth {
            Some(map) => match map.get("type") {
                Some(Value::String(kind)) => kind,
                _ => "none",
            },
            None => "none",
        }
    }

    /// The request's name.
    pub fn name(&self) -> &str {
        self.req.name()
    }

    /// The request's `info.tags`.
    pub fn tags(&self) -> Vec<String> {
        match self.req.document().get(&["info", "tags"]) {
            Some(Value::Sequence(tags)) => tags.iter().filter_map(js::yaml_to_string).collect(),
            _ => Vec::new(),
        }
    }

    /// The enabled path params as `(name, raw value)`, in file order.
    pub fn path_params(&self) -> Vec<(String, String)> {
        let Some(Value::Sequence(params)) =
            self.req.document().get(&[self.req.root_key(), "params"])
        else {
            return Vec::new();
        };
        params
            .iter()
            .filter(|p| text(p.get("type")) == "path" && !js::is_disabled(p))
            .map(|p| (text(p.get("name")), text(p.get("value"))))
            .collect()
    }
}

// Templates are not secrets, but a script may have written a literal value into the URL.
impl fmt::Debug for PreparedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedRequest")
            .field("method", &self.method)
            .field("url", &redact_url(&self.url))
            .finish_non_exhaustive()
    }
}

/// Resolve `req` against `ctx`: [`prepare`] followed by [`resolve_prepared`].
///
/// Fails with [`ResolveError::UnsupportedAuth`] or [`ResolveError::UnsupportedBody`] when the
/// auth or body cannot be sent, with [`ResolveError::InvalidUrl`] when the URL does not parse,
/// with [`ResolveError::InvalidProxy`] when the collection's proxy is unusable, with
/// [`ResolveError::PromptRequired`] when a prompt variable has no answer, and with
/// [`ResolveError::UndefinedVariable`] or [`ResolveError::CyclicVariable`] when a
/// placeholder cannot be expanded.
pub fn resolve(req: &RequestFile, ctx: &VarContext) -> Result<ResolvedRequest, ResolveError> {
    resolve_prepared(prepare(req, ctx)?, ctx)
}

/// Resolves `req` as Send would, then turns it into a JSON POST of `body` with `content-type`
/// and `accept` forced to `application/json`; `fallback` fills a missing timeout.
///
/// Fails as [`resolve`] does, except that the request's own body is never read.
pub(crate) fn introspection(
    req: &RequestFile,
    ctx: &VarContext,
    body: String,
    fallback: Duration,
) -> Result<ResolvedRequest, ResolveError> {
    // Dropping the body first keeps the user's query and variables out of both interpolation
    // and the prompt scan; URL, headers, auth and network resolve exactly as for Send.
    let mut bare = req.clone();
    bare.set_body(crate::edit::BodySpec::None);
    let mut prepared = prepare(&bare, ctx)?;
    // Schema loads ignore the context default, so an inherited timeout still takes `fallback`.
    prepared.set_timeout(settings(bare.document().get(&["settings"]), None).timeout);
    prepared.set_method("POST");
    let mut resolved = resolve_prepared(prepared, ctx)?;
    resolved.body = Body::Text(body);
    for name in ["content-type", "accept"] {
        set_header(
            &mut resolved.headers,
            name.to_owned(),
            "application/json".to_owned(),
        );
    }
    resolved.settings.timeout.get_or_insert(fallback);
    Ok(resolved)
}

/// Checks the auth and body kinds of `req` and merges its headers, interpolating nothing.
///
/// Fails with [`ResolveError::UnsupportedAuth`] or [`ResolveError::UnsupportedBody`].
pub fn prepare(req: &RequestFile, ctx: &VarContext) -> Result<PreparedRequest, ResolveError> {
    prepare_plain(req, ctx).map_err(|e| e.with_hint(|name| ctx.undefined_hint(req, name)))
}

fn prepare_plain(req: &RequestFile, ctx: &VarContext) -> Result<PreparedRequest, ResolveError> {
    let defaults = ctx.defaults_for(req);
    // Auth and body kinds are checked before anything is interpolated, so they fail first.
    let auth = effective_auth(req.document().get(&[req.root_key(), "auth"]), &defaults)?.cloned();
    let body = match req.kind() {
        RequestKind::Graphql => graphql_spec(req.document().get(&[req.root_key(), "body"])),
        RequestKind::Http => body_spec(req.document().get(&[req.root_key(), "body"]))?,
    };
    let raw = raw_headers(req, &defaults);
    let mut headers = raw.clone();
    let auth_slot = auth
        .as_ref()
        .and_then(header_auth)
        .map(|(name, value)| set_header(&mut headers, name, value));
    let settings_block = req.document().get(&["settings"]);
    Ok(PreparedRequest {
        req: req.clone(),
        // The method is sent as written, uppercased, and never interpolated, as axios does.
        method: req.method().to_ascii_uppercase(),
        url: req.url().to_owned(),
        headers,
        auth_slot,
        pristine: Some(raw),
        auth,
        body,
        settings: settings(settings_block, ctx.default_timeout()),
        has_settings_block: matches!(settings_block, Some(Value::Mapping(_))),
    })
}

/// Interpolates `prepared` against `ctx`: path params, basic auth and a query apikey, the body,
/// warnings and the sensitive values substituted.
///
/// Fails with [`ResolveError::InvalidUrl`], [`ResolveError::InvalidProxy`],
/// [`ResolveError::PromptRequired`], [`ResolveError::UndefinedVariable`] or
/// [`ResolveError::CyclicVariable`] as [`resolve`] does.
pub fn resolve_prepared(
    prepared: PreparedRequest,
    ctx: &VarContext,
) -> Result<ResolvedRequest, ResolveError> {
    let PreparedRequest {
        req,
        method,
        url: template,
        headers: edited,
        auth_slot,
        pristine,
        auth,
        body,
        settings,
        has_settings_block,
    } = prepared;
    // The hint is filled once here, where the request is at hand for the leak guard.
    let outcome = (|| -> Result<ResolvedRequest, ResolveError> {
        // Every unanswered prompt is named before any other placeholder can fail.
        let missing = missing_labels(&req, ctx);
        if !missing.is_empty() {
            return Err(ResolveError::PromptRequired { labels: missing });
        }
        let defaults = ctx.defaults_for(&req);
        let (vars, sensitive) = ctx.lookup_for(&req);
        let names = sensitive.clone();
        let scope = Scope {
            vars,
            tracker: ctx.tracker(sensitive),
            generator: RefCell::new(ctx.generator()),
        };
        // Mocked once, so the escape pass below rebuilds the same dynamic values.
        let spots = table_escape_spots(&template, &query_cells(&req.params()));
        let (raw_url, spots) = mock_around(&template, &spots, &mut scope.generator.borrow_mut());
        let url = scope.expand(&raw_url)?;
        let written = written_escapes(&raw_url, &spots, &url, &scope.vars);
        let url = fill_path_params(
            &url,
            req.document().get(&[req.root_key(), "params"]),
            &scope,
        )?;
        // Trim before the empty test so the marks from `written_escapes`, counted from the end,
        // are moved by the padding that fell off the tail.
        let mut url = url;
        let tail_cut = url.len() - url.trim_end_matches(URL_PADDING).len();
        trim_padding(&mut url);
        if url.is_empty() {
            return Err(ResolveError::EmptyUrl);
        }
        let written: Vec<usize> = written
            .into_iter()
            .filter(|from_end| *from_end > tail_cut)
            .map(|from_end| from_end - tail_cut)
            .collect();
        let url_text = with_scheme(url);
        // Bruno runs `encodeUrl` only for a request that has a settings block with it on.
        let encode = has_settings_block && settings.encode_url;
        let encoded = encode.then(|| {
            // A path param may shorten the path, so a mark there can fall off the front.
            let written: Vec<usize> = written
                .iter()
                .filter_map(|from_end| url_text.len().checked_sub(*from_end))
                .collect();
            encode_url(&url_text, &written)
        });
        // Once a secret reached the URL, a parse error names the URL as templated so the error
        // text never carries the value; callers get no `sensitive_values` to scrub it with.
        let mut url = parse_url(encoded.as_deref().unwrap_or(&url_text)).map_err(|err| {
            if scope.tracker.is_empty() {
                err
            } else {
                ResolveError::InvalidUrl {
                    url: redact_url(&with_scheme(template.clone())),
                }
            }
        })?;
        // Any path param, used or not, makes the reference rebuild the URL from its origin,
        // which holds no userinfo.
        // The drop is told to the user, so a credential they wrote is never silently lost.
        let credentials_dropped = has_path_param(req.document().get(&[req.root_key(), "params"]))
            && (!url.username().is_empty() || url.password().is_some());
        if credentials_dropped {
            let _ = url.set_username("");
            let _ = url.set_password(None);
        }

        // Interpolated names then fold case-insensitively, as axios's `AxiosHeaders.set` does.
        // Untouched headers take auth afterwards, as today; edited ones already hold it.
        let untouched = pristine.is_some();
        let mut headers = Vec::new();
        let mut written_names: Vec<(String, String)> = Vec::new();
        for (i, (name, value)) in pristine.unwrap_or(edited).into_iter().enumerate() {
            let written = name.clone();
            let name = scope.expand(&name)?;
            // An apikey whose key expands to empty names no header, and its value is never read.
            if !untouched && auth_slot == Some(i) && name.is_empty() {
                continue;
            }
            written_names.push((name.clone(), written));
            set_header(&mut headers, name, scope.expand(&value)?);
        }
        if let Some(auth) = &auth {
            apply_auth(auth, &scope, &mut headers, &mut url, untouched)?;
        }
        let mut auth_warnings = Vec::new();
        let mut profile_secrets = Vec::new();
        let send_auth = match &auth {
            Some(auth) => per_hop_auth(
                auth,
                &scope,
                &url,
                ctx,
                &mut auth_warnings,
                &mut profile_secrets,
            )?,
            None => None,
        };
        let placed = match &auth {
            Some(auth) => place_oauth2_token(auth, &req, ctx, &mut headers, &mut url)?,
            None => None,
        };
        let mut files = Vec::new();
        let graphql = matches!(body, BodySpec::Graphql { .. } | BodySpec::GraphqlData(_));
        let (body, content_type) = render_body(body, &scope, ctx.root(), &mut files)?;
        match content_type {
            ContentType::None => {}
            // The default content type yields to any header of that name the user enabled.
            ContentType::Default(content_type) => {
                if !headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                {
                    headers.push(("content-type".to_owned(), content_type.to_owned()));
                }
            }
            ContentType::Forced(value) => {
                set_header(&mut headers, "content-type".to_owned(), value);
            }
        }
        let mut warnings = warnings(&req, &defaults, &settings);
        if credentials_dropped {
            warnings.push(Warning::UrlCredentialsDropped);
        }
        let Scope {
            vars,
            tracker,
            generator,
        } = scope;
        let mut sensitive_values = tracker.into_values();
        warnings.extend(auth_warnings);
        for secret in profile_secrets {
            if !secret.is_empty() && !sensitive_values.contains(&secret) {
                sensitive_values.push(secret);
            }
        }
        let mut derived = Vec::new();
        if let Some((secrets, expired, token)) = placed {
            derived.push(token);
            if expired {
                warnings.push(Warning::OAuth2TokenExpired);
            }
            for secret in secrets {
                if !sensitive_values.contains(&secret) {
                    sensitive_values.push(secret);
                }
            }
        }
        // Judge every text the parser could have seen, then the parsed host for what it decoded.
        let secret_host = [Some(&url_text), encoded.as_ref()]
            .into_iter()
            .flatten()
            .any(|text| secret_reaches_host(text, &sensitive_values))
            || url.host_str().is_some_and(|host| {
                let host = host.to_lowercase();
                sensitive_values
                    .iter()
                    .map(|v| url_normal(v))
                    .any(|v| !v.is_empty() && host.contains(&v))
            });
        let url = String::from(url);
        let auth_secrets = crate::secret_names::auth_secrets(
            auth.as_ref(),
            (&headers, &url, send_auth.as_ref()),
            &vars,
        );
        let lookup = (&vars, &generator);
        let network = resolve_network(ctx, lookup, &names, &url, &mut sensitive_values)?;
        let header_problem = first_header_problem(&headers, &written_names, &sensitive_values);
        Ok(ResolvedRequest {
            method,
            url,
            headers,
            header_problem,
            body,
            settings,
            warnings,
            sensitive_values,
            secret_host,
            template_host: template_host(&with_scheme(template)).to_owned(),
            url_text,
            files,
            network,
            send_auth,
            auth_secrets,
            derived,
            graphql,
        })
    })();
    outcome.map_err(|e| e.with_hint(|name| ctx.undefined_hint(&req, name)))
}

/// The first header the HTTP stack would reject. `written` pairs a resolved name with the name
/// as the file spells it, the latest entry winning as `set_header` does. A header the file never
/// wrote (auth, body) is named as resolved, or as an auth header when a secret is in its name,
/// so no message echoes secret text.
fn first_header_problem(
    headers: &[(String, String)],
    written: &[(String, String)],
    secrets: &[String],
) -> Option<String> {
    headers.iter().find_map(|(name, value)| {
        let masked = secrets
            .iter()
            .any(|s| !s.is_empty() && name.contains(s.as_str()));
        let spelled = written
            .iter()
            .rev()
            .find(|(expanded, _)| expanded.eq_ignore_ascii_case(name))
            .map(|(_, spelled)| spelled.as_str());
        let label = match spelled {
            Some(spelled) => spelled,
            None if masked => "an authentication header",
            None => name.as_str(),
        };
        header_problem(label, name, value, masked)
    })
}

/// An internal form POST to a token endpoint: no redirects, no body files, the network settings
/// and timeout of the request the token is for. `template_url` is the URL as the settings wrote
/// it, so a refusal never names a value. `secrets` are the values to hide and to judge against
/// the host.
pub(crate) fn token_request(
    req: &RequestFile,
    ctx: &VarContext,
    template_url: &str,
    url: Url,
    headers: Vec<(String, String)>,
    form: Vec<(String, String)>,
    secrets: &[String],
) -> Result<ResolvedRequest, ResolveError> {
    let (vars, names) = ctx.lookup_for(req);
    let generator = RefCell::new(ctx.generator());
    let mut sensitive_values: Vec<String> = Vec::new();
    for secret in secrets {
        if !secret.is_empty() && !sensitive_values.contains(secret) {
            sensitive_values.push(secret.clone());
        }
    }
    let secret_host = secret_reaches_host(url.as_str(), &sensitive_values)
        || secret_in_parsed_host(&url, &sensitive_values);
    let url = String::from(url);
    let network = resolve_network(
        ctx,
        (&vars, &generator),
        &names,
        &url,
        &mut sensitive_values,
    )?;
    Ok(ResolvedRequest {
        method: "POST".to_owned(),
        url_text: url.clone(),
        url,
        headers,
        header_problem: None,
        body: Body::FormUrlEncoded(form),
        settings: Settings {
            timeout: settings(req.document().get(&["settings"]), ctx.default_timeout()).timeout,
            follow_redirects: false,
            max_redirects: 0,
            forward_authorization_header: false,
            encode_url: false,
            omit_headers: Vec::new(),
        },
        warnings: Vec::new(),
        sensitive_values,
        secret_host,
        template_host: template_host(&with_scheme(template_url.to_owned())).to_owned(),
        files: Vec::new(),
        network,
        send_auth: None,
        auth_secrets: Vec::new(),
        derived: Vec::new(),
        graphql: false,
    })
}

/// Whether the parsed host contains a sensitive value, which catches a percent-encoded secret
/// that the URL parser decodes into the host.
fn secret_in_parsed_host(url: &Url, values: &[String]) -> bool {
    url.host_str().is_some_and(|host| {
        let host = host.to_lowercase();
        values
            .iter()
            .map(|v| url_normal(v))
            .any(|v| !v.is_empty() && host.contains(&v))
    })
}

/// A request a script sends, as its config gave it; never interpolated.
pub(crate) struct ScriptSend {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<String>,
    pub(crate) timeout: Option<Duration>,
}

/// A script's send as a request: axios's redirect limit, no body files, and the network settings
/// `req`'s collection matches to the URL. `sensitive` are the values the script read, judged
/// against the host so a secret never chooses where a request goes.
pub(crate) fn script_request(
    send: ScriptSend,
    req: &RequestFile,
    ctx: &VarContext,
    sensitive: &[String],
) -> Result<ResolvedRequest, ResolveError> {
    let (vars, names) = ctx.lookup_for(req);
    let generator = RefCell::new(ctx.generator());
    let mut sensitive_values: Vec<String> = Vec::new();
    for secret in sensitive {
        if !secret.is_empty() && !sensitive_values.contains(secret) {
            sensitive_values.push(secret.clone());
        }
    }
    if send.url.trim_matches(URL_PADDING).is_empty() {
        return Err(ResolveError::EmptyUrl);
    }
    let url_text = with_scheme(send.url);
    let url = parse_url(&url_text)?;
    let secret_host = secret_reaches_host(&url_text, &sensitive_values)
        || secret_in_parsed_host(&url, &sensitive_values);
    let url = String::from(url);
    let network = resolve_network(
        ctx,
        (&vars, &generator),
        &names,
        &url,
        &mut sensitive_values,
    )?;
    let mut headers = Vec::new();
    for (name, value) in send.headers {
        set_header(&mut headers, name, value);
    }
    let header_problem = first_header_problem(&headers, &[], &sensitive_values);
    Ok(ResolvedRequest {
        method: send.method.to_ascii_uppercase(),
        url,
        headers,
        header_problem,
        body: send.body.map_or(Body::None, Body::Text),
        settings: Settings {
            timeout: send.timeout,
            follow_redirects: true,
            max_redirects: 21,
            forward_authorization_header: false,
            encode_url: false,
            omit_headers: Vec::new(),
        },
        warnings: Vec::new(),
        sensitive_values,
        secret_host,
        // The script's URL is runtime text, so a host holding a secret is never quoted in errors.
        url_text: url_text.clone(),
        template_host: match secret_host {
            true => "<redacted>".to_owned(),
            false => template_host(&url_text).to_owned(),
        },
        files: Vec::new(),
        network,
        send_auth: None,
        auth_secrets: Vec::new(),
        derived: Vec::new(),
        graphql: false,
    })
}

/// The request's proxy and client certificate. It runs after the host judgement, so proxy
/// credentials and the passphrase join `sensitive` too late to make the request host look secret.
/// Only substitutions into the proxy's protocol, host and port mark the proxy.
fn resolve_network(
    ctx: &VarContext,
    (vars, generator): (&Vars, &RefCell<Generator>),
    names: &BTreeSet<String>,
    url: &str,
    sensitive: &mut Vec<String>,
) -> Result<RequestNetwork, ResolveError> {
    let address = ctx.tracker(names.clone());
    let extras = ctx.tracker(names.clone());
    // Every text the send expands is kept as it came out, so the check can confirm its own
    // view of the settings matches what is actually used.
    let actual: RefCell<BTreeMap<String, String>> = RefCell::default();
    let expand = |tracker: &Tracker, text: &str| {
        let value = interpolate_tracked(text, vars, Some(tracker), generator)?;
        actual.borrow_mut().insert(text.to_owned(), value.clone());
        Ok::<String, ResolveError>(value)
    };
    let block = ctx.network();
    let mut secrets = Vec::new();
    let proxy = match &block.proxy {
        CollectionProxy::Inherit => RequestProxy::Inherit,
        CollectionProxy::Off => RequestProxy::Off,
        CollectionProxy::Custom(config) => {
            let mut proxy = ResolvedProxy::new(
                &expand(&address, &config.protocol)?,
                &expand(&address, &config.hostname)?,
                &expand(&address, &config.port)?,
            )?;
            if let Some(auth) = &config.auth {
                let username = expand(&extras, &auth.username)?;
                let password = expand(&extras, &auth.password)?;
                secrets.extend([username.clone(), password.clone()]);
                proxy = proxy.with_credentials(username, password);
            }
            let bypass = expand(&extras, &config.bypass)?;
            RequestProxy::Custom { proxy, bypass }
        }
    };
    // Disabled entries are skipped unexpanded; the first whose domain matches is the only one
    // whose paths and passphrase are expanded.
    let mut certificate = None;
    let mut chosen = None;
    for (index, cert) in block
        .certificates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.enabled)
    {
        let domain = expand(&extras, &cert.domain)?;
        if !domain_matches(&domain, url) {
            continue;
        }
        let path = |written: &String| -> Result<CertPath, ResolveError> {
            Ok(CertPath {
                read: ctx.root().join(expand(&extras, written)?),
                written: written.clone(),
            })
        };
        let files = match &cert.kind {
            CertificateKind::Pem {
                certificate,
                private_key,
            } => CertificateFiles::Pem {
                certificate: path(certificate)?,
                private_key: path(private_key)?,
            },
            CertificateKind::Pkcs12 { file } => CertificateFiles::Pkcs12 { file: path(file)? },
        };
        let passphrase = expand(&extras, &cert.passphrase)?;
        secrets.push(passphrase.clone());
        certificate = Some(ResolvedCertificate::new(domain, files, passphrase));
        chosen = Some(index);
        break;
    }
    let host_overrides =
        resolve_host_overrides(&block.host_overrides, |text| expand(&extras, text))?;
    let secret_in_proxy = !address.is_empty();
    let found = address
        .into_values()
        .into_iter()
        .chain(extras.into_values());
    for value in found.chain(secrets) {
        if !value.is_empty() && !sensitive.contains(&value) {
            sensitive.push(value);
        }
    }
    Ok(RequestNetwork {
        proxy,
        certificate,
        secret_in_proxy,
        host_overrides,
        basis: Some(std::sync::Arc::new(NetworkBasis::new(
            ctx,
            vars,
            url,
            chosen,
            &actual.into_inner(),
        ))),
    })
}

/// URL text as WHATWG reads it, for comparison: ends trimmed of C0 controls and spaces, tabs
/// and newlines removed, lowercased.
fn url_normal(text: &str) -> String {
    text.trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .to_lowercase()
}

/// Where WHATWG finds the host in raw `url`: `(host, authority end)` as byte ranges, mirroring
/// its special-scheme rule that any run of `/` and `\` follows the colon. `None` with no authority.
fn host_bounds(url: &str) -> Option<(std::ops::Range<usize>, usize)> {
    let colon = url.find(':')?;
    let mut scheme = url[..colon].chars();
    let valid = scheme.next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    if !valid {
        return None;
    }
    let scheme = &url[..colon];
    let after = &url[colon + 1..];
    let file = scheme.eq_ignore_ascii_case("file");
    let special = file
        || ["http", "https", "ws", "wss", "ftp"]
            .iter()
            .any(|s| scheme.eq_ignore_ascii_case(s));
    let start = if special && !file {
        url.len() - after.trim_start_matches(['/', '\\']).len()
    } else {
        let mut chars = after.chars();
        let slash = |c: Option<char>| matches!(c, Some('/')) || (file && c == Some('\\'));
        if !(slash(chars.next()) && slash(chars.next())) {
            return None;
        }
        colon + 3
    };
    let delimiters: &[char] = if special {
        &['/', '?', '#', '\\']
    } else {
        &['/', '?', '#']
    };
    let end = url[start..]
        .find(delimiters)
        .map_or(url.len(), |i| start + i);
    // A file host has no userinfo; elsewhere the host follows the last `@`.
    let host_start = match url[start..end].rfind('@') {
        Some(at) if !file => start + at + 1,
        _ => start,
    };
    let host = &url[host_start..end];
    let host_end = if host.starts_with('[') {
        host.find(']').map_or(end, |i| host_start + i + 1)
    } else {
        host.find(':').map_or(end, |i| host_start + i)
    };
    Some((host_start..host_end, end))
}

/// Whether any occurrence of a non-empty sensitive value in `url` overlaps its host, so a value
/// cut by a delimiter still counts when only part of it lands in the host.
fn secret_reaches_host(url: &str, values: &[String]) -> bool {
    let url = url_normal(url);
    let Some((host, _)) = host_bounds(&url) else {
        return false;
    };
    values
        .iter()
        .map(|value| url_normal(value))
        .filter(|needle| !needle.is_empty())
        .any(|needle| {
            (0..host.end).any(|i| {
                url.is_char_boundary(i)
                    && url[i..].starts_with(&needle)
                    && i + needle.len() > host.start
            })
        })
}

/// The host as the template wrote it, for errors; the whole authority past any userinfo when
/// the host itself is empty (a path parameter may fill it).
fn template_host(url: &str) -> &str {
    match host_bounds(url) {
        Some((host, _)) if !host.is_empty() => &url[host],
        Some((host, end)) => &url[host.start..end],
        None => "",
    }
}

/// The oauth2 settings that apply to `req`, following the same inheritance as its auth.
/// `None` for any other auth, an unreadable flow or an auth error.
pub(crate) fn effective_oauth2(req: &RequestFile, ctx: &VarContext) -> Option<OAuth2Spec> {
    let defaults = ctx.defaults_for(req);
    let map = effective_auth(req.document().get(&[req.root_key(), "auth"]), &defaults).ok()??;
    if map.get("type").and_then(js::yaml_to_string).as_deref() != Some("oauth2") {
        return None;
    }
    OAuth2Spec::from_yaml(&Value::Mapping(map.clone()))
}

/// How one `auth` value reads before inheritance is followed.
enum AuthMode<'a> {
    None,
    Inherit,
    Apply(&'a Mapping),
}

/// Classifies an `auth` value; kinds Apinata cannot send are [`ResolveError::UnsupportedAuth`].
fn auth_mode(value: Option<&Value>) -> Result<AuthMode<'_>, ResolveError> {
    let unsupported = |kind: String| Err(ResolveError::UnsupportedAuth { kind });
    match value {
        None | Some(Value::Null) => Ok(AuthMode::None),
        Some(whole @ Value::Mapping(map)) => {
            let kind = map.get("type").and_then(js::yaml_to_string);
            match kind.as_deref() {
                Some(
                    "basic" | "bearer" | "apikey" | "digest" | "awsv4" | "oauth1" | "wsse" | "ntlm"
                    | "akamai-edgegrid",
                ) => Ok(AuthMode::Apply(map)),
                // An oauth2 block without a known flow stays unsupported.
                Some("oauth2") if OAuth2Spec::from_yaml(whole).is_some() => {
                    Ok(AuthMode::Apply(map))
                }
                Some(other) => unsupported(other.to_owned()),
                None => unsupported("unknown".to_owned()),
            }
        }
        Some(other) => match js::yaml_to_string(other).as_deref() {
            Some("none") => Ok(AuthMode::None),
            Some("inherit") => Ok(AuthMode::Inherit),
            Some(kind) => unsupported(kind.to_owned()),
            None => unsupported("unknown".to_owned()),
        },
    }
}

/// The auth mapping that applies to a request whose own auth is `own`.
///
/// `inherit` takes the innermost folder with auth other than none or inherit, else the
/// collection's, where `inherit` also means none. Only the auth that applies is checked.
pub(crate) fn effective_auth<'a>(
    own: Option<&'a Value>,
    defaults: &Defaults<'a>,
) -> Result<Option<&'a Mapping>, ResolveError> {
    match auth_mode(own)? {
        AuthMode::None => return Ok(None),
        AuthMode::Apply(map) => return Ok(Some(map)),
        AuthMode::Inherit => {}
    }
    for folder in defaults.folders.iter().rev() {
        let auth = folder.get("auth");
        if is_pass_through(auth) {
            continue;
        }
        if let AuthMode::Apply(map) = auth_mode(auth)? {
            return Ok(Some(map));
        }
    }
    // At the collection level every pass-through form, a typeless mapping included, means none.
    let auth = defaults.collection.and_then(|block| block.get("auth"));
    if is_pass_through(auth) {
        return Ok(None);
    }
    match auth_mode(auth)? {
        AuthMode::Apply(map) => Ok(Some(map)),
        AuthMode::None | AuthMode::Inherit => Ok(None),
    }
}

/// The level an inherited auth comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthSource {
    /// The collection's `request.auth`, or no auth when it has none.
    Collection,
    /// The folder at this path.
    Folder(ItemPath),
}

/// The auth an item inherits and the level that supplies it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InheritedAuth {
    auth: AuthSpec,
    source: AuthSource,
}

impl InheritedAuth {
    /// The inherited auth, with variables unresolved; `Unsupported` for a kind not modelled.
    pub fn auth(&self) -> &AuthSpec {
        &self.auth
    }

    /// The level that supplies the auth.
    pub fn source(&self) -> &AuthSource {
        &self.source
    }
}

impl Collection {
    /// The auth `item` (a request or a folder) gets from the levels above it, without
    /// resolving variables. Folders are skipped by the same pass-through rule as a send, and
    /// a collection without auth supplies `AuthSpec::None`. Never fails.
    pub fn inherited_auth(&self, item: &ItemPath) -> InheritedAuth {
        // Walk the folders innermost first; the root folder has no file and ends the walk.
        let mut level = item.parent();
        while let Some(path) = level.filter(|p| !p.is_root()) {
            let auth = self
                .folder(&path)
                .and_then(|f| f.document())
                .and_then(|doc| doc.get(&["request", "auth"]));
            if !is_pass_through(auth) {
                return InheritedAuth {
                    auth: AuthSpec::from_yaml(auth),
                    source: AuthSource::Folder(path),
                };
            }
            level = path.parent();
        }
        let auth = self.file().document().get(&["request", "auth"]);
        InheritedAuth {
            auth: if is_pass_through(auth) {
                AuthSpec::None
            } else {
                AuthSpec::from_yaml(auth)
            },
            source: AuthSource::Collection,
        }
    }
}

/// The level an inherited header comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderSource {
    /// The collection's `request.headers`.
    Collection,
    /// The folder at this path.
    Folder(ItemPath),
}

/// One header an item inherits, with the level that supplies it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InheritedHeader {
    name: String,
    value: String,
    source: HeaderSource,
    shadowed: bool,
}

impl InheritedHeader {
    /// The header name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The raw value, with variables unresolved.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The level that supplies the header.
    pub fn source(&self) -> &HeaderSource {
        &self.source
    }

    /// Whether an inner folder or one of the request's `own` rows replaces this header.
    pub fn overridden(&self, own: &[KeyValue]) -> bool {
        self.shadowed
            || own
                .iter()
                .any(|kv| kv.enabled && kv.name.eq_ignore_ascii_case(&self.name))
    }
}

impl Collection {
    /// The headers `item` inherits from the collection and its folders, outermost first.
    pub fn inherited_headers(&self, item: &ItemPath) -> Vec<InheritedHeader> {
        // Layers outermost first: the collection, then each folder down to the item's parent.
        let mut folders = Vec::new();
        let mut level = item.parent();
        while let Some(path) = level.filter(|p| !p.is_root()) {
            level = path.parent();
            folders.push(path);
        }
        let mut layers = vec![(
            HeaderSource::Collection,
            self.file().document().get(&["request", "headers"]),
        )];
        for path in folders.into_iter().rev() {
            let rows = self
                .folder(&path)
                .and_then(|f| f.document())
                .and_then(|doc| doc.get(&["request", "headers"]));
            layers.push((HeaderSource::Folder(path), rows));
        }
        let merged: Vec<(HeaderSource, Vec<(String, String)>)> = layers
            .into_iter()
            .map(|(source, rows)| {
                let mut merged = Vec::new();
                if let Some(rows) = rows {
                    merge_raw_headers(&mut merged, rows);
                }
                (source, merged)
            })
            .collect();
        // The send folds names that differ only in case, so a later row of any case shadows.
        let mut out = Vec::new();
        for (i, (source, rows)) in merged.iter().enumerate() {
            for (j, (name, value)) in rows.iter().enumerate() {
                let later = |n: &String| n.eq_ignore_ascii_case(name);
                let shadowed = rows[j + 1..].iter().any(|(n, _)| later(n))
                    || merged[i + 1..]
                        .iter()
                        .any(|(_, inner)| inner.iter().any(|(n, _)| later(n)));
                out.push(InheritedHeader {
                    name: name.clone(),
                    value: value.clone(),
                    source: source.clone(),
                    shadowed,
                });
            }
        }
        out
    }
}

/// Whether an inherited auth defers to the next level up (or, at the collection, means none):
/// missing, `none`, `inherit`, or a mapping without a `type`, which Bruno reads as mode none.
pub(crate) fn is_pass_through(auth: Option<&Value>) -> bool {
    match auth {
        None | Some(Value::Null) => true,
        Some(Value::Mapping(map)) => map.get("type").is_none(),
        Some(value) => matches!(
            js::yaml_to_string(value).as_deref(),
            Some("none" | "inherit")
        ),
    }
}

/// The enabled headers of the collection, folder and request levels, merged outermost first
/// and not interpolated.
pub(crate) fn raw_headers(req: &RequestFile, defaults: &Defaults<'_>) -> Vec<(String, String)> {
    let layers = defaults
        .collection
        .and_then(|block| block.get("headers"))
        .into_iter()
        .chain(
            defaults
                .folders
                .iter()
                .filter_map(|block| block.get("headers")),
        )
        .chain(req.document().get(&[req.root_key(), "headers"]));
    let mut raw = Vec::new();
    for rows in layers {
        merge_raw_headers(&mut raw, rows);
    }
    raw
}

/// Adds one raw `headers` list to `headers` as Bruno's `mergeHeaders` does: names match
/// exactly, and a repeated name takes the new value in place. Disabled rows and empty names
/// are skipped.
fn merge_raw_headers(headers: &mut Vec<(String, String)>, rows: &Value) {
    let Value::Sequence(rows) = rows else {
        return;
    };
    for row in rows {
        if js::is_disabled(row) {
            continue;
        }
        let name = text(row.get("name"));
        if name.is_empty() {
            continue;
        }
        let value = text(row.get("value"));
        match headers.iter_mut().find(|(existing, _)| *existing == name) {
            Some(slot) => slot.1 = value,
            None => headers.push((name, value)),
        }
    }
}

/// Sets a header as axios does: an existing name, in any case, keeps its spelling and position
/// and takes the new value. Returns the index written.
fn set_header(headers: &mut Vec<(String, String)>, name: String, value: String) -> usize {
    match headers
        .iter()
        .position(|(existing, _)| existing.eq_ignore_ascii_case(&name))
    {
        Some(i) => {
            headers[i].1 = value;
            i
        }
        None => {
            headers.push((name, value));
            headers.len() - 1
        }
    }
}

/// The bearer or header apikey auth as a raw header, as `apply_auth` would set it before
/// interpolation; an apikey with an empty raw key or value sets none.
fn header_auth(auth: &Mapping) -> Option<(String, String)> {
    match auth.get("type").and_then(js::yaml_to_string).as_deref() {
        Some("bearer") => Some((
            "Authorization".to_owned(),
            format!("Bearer {}", text(auth.get("token"))),
        )),
        Some("apikey") => {
            let placement = auth.get("placement").and_then(js::yaml_to_string);
            let (key, value) = (text(auth.get("key")), text(auth.get("value")));
            (placement.as_deref() == Some("header") && !key.is_empty() && !value.is_empty())
                .then_some((key, value))
        }
        _ => None,
    }
}

/// Puts the cached token of an oauth2 `auth` in the header or query parameter its settings
/// name. Returns the values to hide (the token and the settings' secrets), whether it has
/// expired and the token itself, or `None` for any other auth.
fn place_oauth2_token(
    auth: &Mapping,
    req: &RequestFile,
    ctx: &VarContext,
    headers: &mut Vec<(String, String)>,
    url: &mut Url,
) -> Result<Option<(Vec<String>, bool, String)>, ResolveError> {
    if auth.get("type").and_then(js::yaml_to_string).as_deref() != Some("oauth2") {
        return Ok(None);
    }
    let Some(spec) = OAuth2Spec::from_yaml(&Value::Mapping(auth.clone())) else {
        return Err(ResolveError::UnsupportedAuth {
            kind: "oauth2".to_owned(),
        });
    };
    let cfg = TokenConfig::new(&spec, req, ctx).map_err(|e| match e {
        OAuth2Error::Resolve(e) => e,
        OAuth2Error::MissingField { field } => ResolveError::OAuth2MissingField { field },
        _ => ResolveError::UnsupportedAuth {
            kind: "oauth2".to_owned(),
        },
    })?;
    let no_token = || ResolveError::OAuth2NoToken {
        credentials_id: cfg.credentials_id.clone(),
    };
    let entry = ctx
        .oauth2_tokens()
        .lookup(&cfg.key(ctx), &cfg.fingerprint())
        .ok_or_else(no_token)?;
    let field = match cfg.source {
        TokenSource::IdToken => "id_token",
        _ => "access_token",
    };
    let token = entry
        .field(field)
        .filter(|t| !t.is_empty())
        .ok_or_else(no_token)?
        .to_owned();
    match &cfg.placement {
        TokenPlacement::Header { prefix } => {
            let value = format!("{prefix} {token}").trim().to_owned();
            set_header(headers, "Authorization".to_owned(), value);
        }
        TokenPlacement::Query { key } => set_query_param(url, key, &token),
        TokenPlacement::Unset => {
            set_header(headers, "Authorization".to_owned(), token.clone());
        }
    }
    let expired = entry.is_expired(SystemTime::now());
    let mut secrets = cfg.secrets;
    secrets.push(token.clone());
    Ok(Some((secrets, expired, token)))
}

/// Applies basic, bearer or apikey auth with its fields interpolated; bearer and header apikey
/// only when `with_headers`, since otherwise the headers already carry them.
fn apply_auth(
    auth: &Mapping,
    scope: &Scope,
    headers: &mut Vec<(String, String)>,
    url: &mut Url,
    with_headers: bool,
) -> Result<(), ResolveError> {
    let field = |key: &str| scope.expand(&text(auth.get(key)));
    match auth.get("type").and_then(js::yaml_to_string).as_deref() {
        Some("basic") => {
            let credentials = format!("{}:{}", field("username")?, field("password")?);
            let value = format!("Basic {}", STANDARD.encode(credentials));
            set_header(headers, "Authorization".to_owned(), value);
        }
        Some("bearer") if with_headers => {
            let value = format!("Bearer {}", field("token")?);
            set_header(headers, "Authorization".to_owned(), value);
        }
        Some("apikey") => {
            // Bruno tests the raw key, and axios drops a header whose raw value was empty; a key
            // that interpolates to empty names nothing either.
            if text(auth.get("key")).is_empty() {
                return Ok(());
            }
            let placement = auth.get("placement").and_then(js::yaml_to_string);
            match placement.as_deref() {
                Some("header") if with_headers && !text(auth.get("value")).is_empty() => {
                    let key = field("key")?;
                    if !key.is_empty() {
                        set_header(headers, key, field("value")?);
                    }
                }
                Some("query") => {
                    let key = field("key")?;
                    if !key.is_empty() {
                        set_query_param(url, &key, &field("value")?);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

/// The per-hop auth of a digest, awsv4, oauth1, wsse, ntlm or akamai-edgegrid `auth`, its fields
/// interpolated. Digest needs a non-blank username and password. SigV4 takes its keys from a
/// named profile when it can be read (pushing them to `secrets`), infers its scope from `url`'s
/// host, and is `None` with [`Warning::AwsV4Unsigned`] when a key or the service stays blank.
/// OAuth 1.0 and WSSE always sign, and NTLM carries its three fields as they are (blanks
/// included); EdgeGrid fails with [`ResolveError::EdgeGrid`] when it cannot.
fn per_hop_auth(
    auth: &Mapping,
    scope: &Scope,
    url: &Url,
    ctx: &VarContext,
    warnings: &mut Vec<Warning>,
    secrets: &mut Vec<String>,
) -> Result<Option<SendAuth>, ResolveError> {
    let field = |key: &str| scope.expand(&text(auth.get(key)));
    let blank = |value: &str| value.trim().is_empty();
    match auth.get("type").and_then(js::yaml_to_string).as_deref() {
        Some("digest") => {
            let (username, password) = (field("username")?, field("password")?);
            Ok((!blank(&username) && !blank(&password))
                .then_some(SendAuth::Digest { username, password }))
        }
        Some("awsv4") => {
            let mut access_key_id = field("accessKeyId")?;
            let mut secret_access_key = field("secretAccessKey")?;
            let mut session_token = field("sessionToken")?;
            let (service, region) = (field("service")?, field("region")?);
            let profile = field("profileName")?;
            let mut profile_unavailable = false;
            if !blank(&profile) {
                match profile_keys(&ctx.aws_shared_files(), profile.trim()) {
                    Some((access, secret, token)) => {
                        secrets.extend([access.clone(), secret.clone(), token.clone()]);
                        (access_key_id, secret_access_key, session_token) = (access, secret, token);
                    }
                    None => profile_unavailable = true,
                }
            }
            // A blank service or region is inferred from the host, as the signer would.
            let given = |value: &str| {
                if blank(value) {
                    String::new()
                } else {
                    value.to_owned()
                }
            };
            let (service, region) = infer_scope(
                url.host_str().unwrap_or_default(),
                &given(&service),
                &given(&region),
            );
            let signed = !blank(&access_key_id) && !blank(&secret_access_key) && !blank(&service);
            // Pushed in variant order, which `warnings()` promises.
            if !signed {
                warnings.push(Warning::AwsV4Unsigned);
            }
            if profile_unavailable {
                warnings.push(Warning::AwsProfileUnavailable);
            }
            Ok(signed.then_some(SendAuth::AwsV4(SigV4 {
                access_key_id,
                secret_access_key,
                session_token,
                service,
                region,
            })))
        }
        Some("oauth1") => {
            let spec = OAuth1Spec::from_yaml(&Value::Mapping(auth.clone()));
            let signer = oauth1_signer(&spec, scope, ctx.root(), secrets)?;
            Ok(Some(SendAuth::OAuth1(Box::new(signer))))
        }
        Some("wsse") => {
            let (username, password) = (field("username")?, field("password")?);
            secrets.push(password.clone());
            Ok(Some(SendAuth::Wsse { username, password }))
        }
        Some("ntlm") => {
            let (username, password) = (field("username")?, field("password")?);
            let domain = field("domain")?;
            secrets.push(password.clone());
            Ok(Some(SendAuth::Ntlm {
                username,
                password,
                domain,
            }))
        }
        Some("akamai-edgegrid") => {
            // Every string is expanded before the signer's checks, so a variable error wins.
            let mut spec = EdgeGridSpec::from_yaml(&Value::Mapping(auth.clone()));
            for value in [
                &mut spec.access_token,
                &mut spec.client_token,
                &mut spec.client_secret,
                &mut spec.nonce,
                &mut spec.timestamp,
                &mut spec.base_url,
                &mut spec.headers_to_sign,
            ] {
                *value = scope.expand(value)?;
            }
            let signer = edgegrid::Signer::new(&spec, url)
                .map_err(|reason| ResolveError::EdgeGrid { reason })?;
            secrets.extend([spec.access_token, spec.client_token, spec.client_secret]);
            Ok(Some(SendAuth::EdgeGrid(Box::new(signer))))
        }
        _ => Ok(None),
    }
}

/// The signer for `spec`, every field interpolated. The secrets and inline key text are pushed
/// to `secrets`. Fails on an unknown method or placement, then on a missing or unreadable RSA key;
/// the key is read only for an RSA method, a relative file path under `root`.
fn oauth1_signer(
    spec: &OAuth1Spec,
    scope: &Scope,
    root: &Path,
    secrets: &mut Vec<String>,
) -> Result<oauth1::Signer, ResolveError> {
    let field = |value: &Option<String>| scope.expand(value.as_deref().unwrap_or_default());
    let fail = |reason: String| Err(ResolveError::OAuth1 { reason });
    let consumer_secret = field(&spec.consumer_secret)?;
    let token_secret = field(&spec.access_token_secret)?;
    secrets.extend([consumer_secret.clone(), token_secret.clone()]);
    let mut signer = oauth1::Signer {
        consumer_key: field(&spec.consumer_key)?,
        consumer_secret,
        token: field(&spec.access_token)?,
        token_secret,
        callback: field(&spec.callback_url)?,
        verifier: field(&spec.verifier)?,
        method: oauth1::SignatureMethod::HmacSha1,
        key: None,
        timestamp: field(&spec.timestamp)?,
        nonce: field(&spec.nonce)?,
        version: scope.expand(&spec.version)?,
        realm: field(&spec.realm)?,
        placement: oauth1::Placement::Header,
        include_body_hash: spec.include_body_hash,
    };
    use OAuth1SignatureMethod as M;
    use oauth1::SignatureMethod as S;
    signer.method =
        match OAuth1SignatureMethod::parse(&scope.expand(spec.signature_method.as_str())?) {
            M::HmacSha1 => S::HmacSha1,
            M::HmacSha256 => S::HmacSha256,
            M::HmacSha512 => S::HmacSha512,
            M::RsaSha1 => S::RsaSha1,
            M::RsaSha256 => S::RsaSha256,
            M::RsaSha512 => S::RsaSha512,
            M::Plaintext => S::Plaintext,
            other => return fail(format!("unknown signature method `{}`", other.as_str())),
        };
    signer.placement = match OAuth1Placement::parse(&scope.expand(spec.placement.as_str())?) {
        OAuth1Placement::Header => oauth1::Placement::Header,
        OAuth1Placement::Query => oauth1::Placement::Query,
        OAuth1Placement::Body => oauth1::Placement::Body,
        other => return fail(format!("unknown placement `{}`", other.as_str())),
    };
    secrets.extend(signer.sensitive_forms());
    if !signer.method.is_rsa() {
        return Ok(signer);
    }
    // Only RSA reads the key, so a stale one under another method is never expanded.
    let key = match &spec.private_key {
        Some(key) => Some((key.kind, scope.expand(&key.value)?)),
        None => None,
    };
    let pem = match key {
        Some((_, value)) if value.is_empty() => None,
        // Inline text may carry its line breaks as literal `\n`, as a one-line field must.
        Some((OAuth1KeyKind::Text, text)) => {
            secrets.push(text.clone());
            Some(text.replace("\\n", "\n"))
        }
        Some((_, path)) => match read_regular_text(&root.join(&path)) {
            Ok(pem) => Some(pem),
            Err(e) => return fail(format!("cannot read the private key file `{path}`: {e}")),
        },
        None => None,
    };
    let Some(pem) = pem else {
        return fail(format!("{} needs a private key", signer.method.name()));
    };
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::DecodePrivateKey;
    let parsed = rsa::RsaPrivateKey::from_pkcs8_pem(pem.trim())
        .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(pem.trim()));
    match parsed {
        Ok(key) => signer.key = Some(key),
        Err(_) => {
            return fail(
                "the private key is not an RSA private key in PEM (PKCS#1 or PKCS#8)".to_owned(),
            );
        }
    }
    Ok(signer)
}

/// A scalar field as text; a missing or non-scalar field is empty.
fn text(value: Option<&Value>) -> String {
    value.and_then(js::yaml_to_string).unwrap_or_default()
}

/// A body whose kind is known to be sendable, with its fields still raw.
#[derive(Clone)]
enum BodySpec {
    None,
    Text {
        data: String,
        json: bool,
        content_type: &'static str,
    },
    Form(Value),
    Multipart(Value),
    /// The selected entry's raw `filePath` and `contentType`, if any entry is selected.
    File(Option<(String, String)>),
    /// The raw query and the decommented variables text, `{}` when that is empty.
    Graphql {
        query: String,
        variables: String,
    },
    /// A body a script gave a GraphQL request, as the script value: text is expanded, and an
    /// object has its `query` and `variables` text expanded and the variables text parsed.
    GraphqlData(serde_json::Value),
}

/// The `Content-Type` a body asks for.
enum ContentType {
    None,
    /// Sent unless the user set a `Content-Type` header.
    Default(&'static str),
    /// Replaces any user `Content-Type` value, even when empty.
    Forced(String),
}

/// Classifies `http.body`; kinds Apinata cannot send are [`ResolveError::UnsupportedBody`].
fn body_spec(body: Option<&Value>) -> Result<BodySpec, ResolveError> {
    let unsupported = |kind: &str| {
        Err(ResolveError::UnsupportedBody {
            kind: kind.to_owned(),
        })
    };
    let map = match body {
        None | Some(Value::Null) => return Ok(BodySpec::None),
        Some(Value::Sequence(_)) => return unsupported("variants"),
        Some(Value::Mapping(map)) => map,
        Some(_) => return unsupported("unknown"),
    };
    let data = map.get("data");
    let text_body = |json, content_type| BodySpec::Text {
        data: text(data),
        json,
        content_type,
    };
    match map.get("type").and_then(js::yaml_to_string).as_deref() {
        Some("json") if text(data).is_empty() => Ok(BodySpec::None),
        Some("json") => Ok(text_body(true, "application/json")),
        Some("text") => Ok(text_body(false, "text/plain")),
        Some("xml") => Ok(text_body(false, "application/xml")),
        Some("sparql") => Ok(text_body(false, "application/sparql-query")),
        Some("form-urlencoded") => Ok(BodySpec::Form(data.cloned().unwrap_or(Value::Null))),
        Some("multipart-form") => Ok(BodySpec::Multipart(data.cloned().unwrap_or(Value::Null))),
        Some("file") => Ok(BodySpec::File(selected_file(data))),
        Some(kind) => unsupported(kind),
        None => unsupported("unknown"),
    }
}

/// Reads `graphql.body`. Variables are decommented now, before scripts run, so a placeholder
/// inside a comment is never expanded. Empty raw text means `{}`, but text that decomments to
/// nothing stays empty and fails to parse.
fn graphql_spec(body: Option<&Value>) -> BodySpec {
    let field = |key| text(body.and_then(|b| b.get(key)));
    let raw = field("variables");
    BodySpec::Graphql {
        query: field("query"),
        variables: decomment(if raw.is_empty() { "{}" } else { &raw }),
    }
}

/// Sends a GraphQL body. Text is expanded like a text body; an object has its string `query`
/// and `variables` expanded separately, and the variables text parsed only after expansion.
fn render_graphql(
    data: serde_json::Value,
    scope: &Scope,
) -> Result<(Body, ContentType), ResolveError> {
    let content_type = ContentType::Default("application/json");
    let mut object = match data {
        serde_json::Value::Object(object) => object,
        // Expanded, unlike the reference, so a literal `{{x}}` is never sent.
        serde_json::Value::String(text) => {
            return Ok((Body::Text(scope.expand(&text)?), content_type));
        }
        serde_json::Value::Null => return Ok((Body::Text(String::new()), content_type)),
        other => return Ok((Body::Text(js::stringify(&other)), content_type)),
    };
    if let Some(serde_json::Value::String(query)) = object.get_mut("query") {
        *query = scope.expand(query)?;
    }
    if let Some(serde_json::Value::String(variables)) = object.get("variables") {
        let parsed = serde_json::from_str(&scope.expand(variables)?).map_err(|e| {
            ResolveError::GraphqlVariables {
                message: e.to_string(),
            }
        })?;
        object.insert("variables".to_owned(), parsed);
    }
    let body = js::stringify(&serde_json::Value::Object(object));
    Ok((Body::Text(body), content_type))
}

/// The first file entry marked `selected: true`, as its raw path and content type.
fn selected_file(data: Option<&Value>) -> Option<(String, String)> {
    let Some(Value::Sequence(rows)) = data else {
        return None;
    };
    rows.iter()
        .find(|row| matches!(row.get("selected"), Some(Value::Bool(true))))
        .map(|row| (text(row.get("filePath")), text(row.get("contentType"))))
}

/// Interpolates a body, returning it with its content type: a default the request's own
/// header overrides, or for a file body a type that replaces it.
///
/// JSON is decommented first, as bruno-cli does, so variable values are never decommented.
/// Relative multipart and file-mode paths are joined to `root`, and each is recorded in `files`.
fn render_body(
    spec: BodySpec,
    scope: &Scope,
    root: &Path,
    files: &mut Vec<FileRef>,
) -> Result<(Body, ContentType), ResolveError> {
    match spec {
        BodySpec::None => Ok((Body::None, ContentType::None)),
        BodySpec::Text {
            data,
            json,
            content_type,
        } => {
            let data = if json { decomment(&data) } else { data };
            let body = Body::Text(scope.expand(&data)?);
            Ok((body, ContentType::Default(content_type)))
        }
        BodySpec::Form(rows) => {
            let mut fields = Vec::new();
            if let Value::Sequence(rows) = &rows {
                for row in rows {
                    if js::is_disabled(row) {
                        continue;
                    }
                    let name = scope.expand(&text(row.get("name")))?;
                    fields.push((name, scope.expand(&text(row.get("value")))?));
                }
            }
            let content_type = "application/x-www-form-urlencoded";
            Ok((
                Body::FormUrlEncoded(fields),
                ContentType::Default(content_type),
            ))
        }
        BodySpec::Multipart(rows) => {
            let parts = multipart_parts(&rows, scope, root, files)?;
            let content_type = ContentType::Default("multipart/form-data");
            Ok((Body::Multipart(parts), content_type))
        }
        // Query and variables expand separately; the variables parse only after expansion.
        BodySpec::Graphql { query, variables } => {
            let data = serde_json::json!({ "query": query, "variables": variables });
            render_graphql(data, scope)
        }
        BodySpec::GraphqlData(data) => render_graphql(data, scope),
        BodySpec::File(None) => Ok((Body::None, ContentType::Default("application/octet-stream"))),
        // bruno-cli reads the path before interpolation, so only the type is interpolated.
        BodySpec::File(Some((path, content_type))) => {
            let content_type = scope.expand(&content_type)?;
            let body = if path.is_empty() {
                Body::None
            } else {
                files.push(FileRef {
                    path: root.join(&path),
                    given: PathBuf::from(&path),
                    written: PathBuf::from(&path),
                });
                Body::File {
                    path: root.join(path),
                    content_type: content_type.clone(),
                }
            };
            Ok((body, ContentType::Forced(content_type)))
        }
    }
}

/// The parts of a `multipart-form` body. Names and part content types are kept as written;
/// text values are JavaScript `String(v)`, so a list is one comma-joined part, and each item
/// of a file value is its own part. Values and paths are interpolated; other types are dropped.
fn multipart_parts(
    rows: &Value,
    scope: &Scope,
    root: &Path,
    files: &mut Vec<FileRef>,
) -> Result<Vec<Part>, ResolveError> {
    let Value::Sequence(rows) = rows else {
        return Ok(Vec::new());
    };
    let mut parts = Vec::new();
    for row in rows {
        if js::is_disabled(row) {
            continue;
        }
        let name = text(row.get("name"));
        let content_type = text(row.get("contentType"));
        let typed = |part: Part| {
            if content_type.is_empty() {
                part
            } else {
                part.with_content_type(content_type.as_str())
            }
        };
        let value = row.get("value").unwrap_or(&Value::Null);
        match text(row.get("type")).as_str() {
            "text" => {
                let value = js::yaml_js_string(value).unwrap_or_default();
                parts.push(typed(Part::text(&name, scope.expand(&value)?)));
            }
            "file" => {
                // Bruno reads `value || []`: a falsy value means no files, and a truthy scalar,
                // on which Bruno would throw, is one path. Null list items are skipped likewise.
                let items = match value {
                    Value::Sequence(items) => items.as_slice(),
                    Value::Null | Value::Bool(false) => &[],
                    Value::String(s) if s.is_empty() => &[],
                    Value::Number(n) if n.as_f64().is_some_and(|n| n == 0.0 || n.is_nan()) => &[],
                    other => std::slice::from_ref(other),
                };
                for item in items.iter().filter(|item| !item.is_null()) {
                    let written = js::yaml_js_string(item).unwrap_or_default();
                    let path = scope.expand(&written)?;
                    let given = PathBuf::from(js::js_trim(&path));
                    let path = root.join(&given);
                    files.push(FileRef {
                        path: path.clone(),
                        given,
                        written: PathBuf::from(&written),
                    });
                    let part = Part::file(&name, path).with_written(written);
                    parts.push(typed(part));
                }
            }
            _ => {}
        }
    }
    Ok(parts)
}

/// Whether `params` holds an entry of type `path`, enabled or not.
fn has_path_param(params: Option<&Value>) -> bool {
    matches!(params, Some(Value::Sequence(params)) if params.iter().any(|p| text(p.get("type")) == "path"))
}

/// Replaces whole `:name` path segments of `url` with their path param's interpolated value.
///
/// The first path param of that name decides: it must be enabled and interpolate to a
/// non-blank value, else the segment stays as written. The path ends at `?` or `#`.
fn fill_path_params(
    url: &str,
    params: Option<&Value>,
    scope: &Scope,
) -> Result<String, ResolveError> {
    let Some(Value::Sequence(params)) = params else {
        return Ok(url.to_owned());
    };
    let path_start = match authority_span(url) {
        Some(span) => span.end,
        None => url.find('/').unwrap_or(url.len()),
    };
    let path_end = url[path_start..]
        .find(['?', '#'])
        .map_or(url.len(), |j| path_start + j);
    let mut segments = Vec::new();
    for segment in url[path_start..path_end].split('/') {
        let param = segment.strip_prefix(':').and_then(|name| {
            params
                .iter()
                .find(|p| text(p.get("type")) == "path" && text(p.get("name")) == name)
        });
        let param = param.filter(|p| !js::is_disabled(p));
        let value = match param {
            Some(p) => Some(scope.expand(&text(p.get("value")))?),
            None => None,
        };
        match value {
            Some(value) if !js::js_trim(&value).is_empty() => segments.push(value),
            _ => segments.push(segment.to_owned()),
        }
    }
    Ok(format!(
        "{}{}{}",
        &url[..path_start],
        segments.join("/"),
        &url[path_end..]
    ))
}

/// Reads the request's own `settings`; nothing is inherited from folders or the collection.
/// A timeout that is absent, `null` or `inherit` takes `default`.
fn settings(block: Option<&Value>, default: Option<Duration>) -> Settings {
    let get = |key: &str| block.and_then(|block| block.get(key));
    // A number above zero is milliseconds; zero, below or any other string means no limit.
    let timeout = match get("timeout") {
        None | Some(Value::Null) => default,
        Some(Value::String(s)) if s == "inherit" => default,
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|ms| *ms > 0.0)
            .and_then(|ms| Duration::try_from_secs_f64(ms / 1000.0).ok()),
        _ => None,
    };
    Settings {
        timeout,
        follow_redirects: to_bool(get("followRedirects")),
        max_redirects: to_max_redirects(get("maxRedirects")),
        forward_authorization_header: to_bool(get("forwardAuthorizationHeader")),
        encode_url: to_bool(get("encodeUrl")),
        omit_headers: omitted_names(block),
    }
}

/// Bruno's `toMaxRedirects`: a finite non-negative number, truncated; anything else is 5.
pub(crate) fn to_max_redirects(value: Option<&Value>) -> u32 {
    match value {
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map_or(5, |n| n.trunc() as u32),
        _ => 5,
    }
}

/// Bruno's `toBool` with a true default: booleans and the strings `"true"` and `"false"`.
pub(crate) fn to_bool(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) if s == "false" => false,
        _ => true,
    }
}

/// The warnings for `req`: scripts or non-variable actions anywhere above or in it, and
/// `encodeUrl: false`.
fn warnings(req: &RequestFile, defaults: &Defaults<'_>, settings: &Settings) -> Vec<Warning> {
    let blocks: Vec<&Value> = defaults
        .collection
        .into_iter()
        .chain(defaults.folders.iter().copied())
        .chain(req.document().get(&["runtime"]))
        .collect();
    let mut warnings = Vec::new();
    if blocks
        .iter()
        .any(|block| is_non_empty(block.get("scripts")))
    {
        warnings.push(Warning::ScriptsNotRun);
    }
    if blocks
        .iter()
        .any(|block| has_other_actions(block.get("actions")))
    {
        warnings.push(Warning::ActionsNotRun);
    }
    if !settings.encode_url {
        warnings.push(Warning::EncodeUrlIgnored);
    }
    warnings
}

/// Whether a `scripts` value holds anything.
fn is_non_empty(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Sequence(items)) => !items.is_empty(),
        Some(Value::Mapping(items)) => !items.is_empty(),
        _ => false,
    }
}

// Header, query and body values can carry secrets, so Debug shows names and sizes only.
impl fmt::Debug for ResolvedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = |pairs: &[(String, String)]| {
            pairs
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        };
        // A sensitive value may sit in the path, which `redact_url` keeps, so hide the whole URL.
        let url = if self.sensitive_values.is_empty() {
            redact_url(&self.url)
        } else {
            "<redacted>".to_owned()
        };
        f.debug_struct("ResolvedRequest")
            .field("method", &self.method)
            .field("url", &url)
            .field("headers", &names(&self.headers))
            .field("body", &self.body)
            .field("settings", &self.settings)
            .field("warnings", &self.warnings)
            .field("sensitive_values", &self.sensitive_values.len())
            .field(
                "network",
                &network_summary(&self.network, &self.sensitive_values),
            )
            .finish()
    }
}

/// The network block for `Debug`: whole when nothing sensitive was substituted, else only the
/// proxy mode and whether a certificate was chosen, since any field may carry a secret.
fn network_summary(network: &RequestNetwork, sensitive: &[String]) -> String {
    if sensitive.is_empty() {
        return format!("{network:?}");
    }
    let mode = match network.proxy() {
        RequestProxy::Inherit => "Inherit",
        RequestProxy::Off => "Off",
        RequestProxy::Custom { .. } => "Custom",
    };
    format!("{mode}, certificate: {}", network.certificate().is_some())
}

// Bodies can carry secrets, so Debug shows only the size or the field names.
impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Body::None => f.write_str("None"),
            Body::Text(text) => f.debug_struct("Text").field("bytes", &text.len()).finish(),
            Body::FormUrlEncoded(fields) => {
                let names: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
                f.debug_struct("FormUrlEncoded")
                    .field("fields", &names)
                    .finish()
            }
            Body::Multipart(parts) => f.debug_tuple("Multipart").field(parts).finish(),
            Body::File { .. } => f.write_str("File"),
        }
    }
}

/// `url` with its userinfo, query values and valueless query parts replaced by `<redacted>`,
/// for Debug output.
pub(crate) fn redact_url(url: &str) -> String {
    const REDACTED: &str = "<redacted>";
    let mut out = String::with_capacity(url.len());
    let mut rest = url;
    // Userinfo is everything before an `@` inside the authority.
    if let Some(span) = authority_span(rest) {
        let (scheme, after) = rest.split_at(span.start);
        out.push_str(scheme);
        let authority_end = span.end - span.start;
        match after[..authority_end].rfind('@') {
            Some(at) => {
                out.push_str(REDACTED);
                rest = &after[at..];
            }
            None => rest = after,
        }
    }
    let Some(q) = rest.find('?') else {
        out.push_str(rest);
        return out;
    };
    out.push_str(&rest[..=q]);
    let query_end = rest.find('#').filter(|h| *h > q).unwrap_or(rest.len());
    let params: Vec<String> = rest[q + 1..query_end]
        .split('&')
        .map(|param| match param.split_once('=') {
            Some((name, _)) => format!("{name}={REDACTED}"),
            // A valueless part may itself be the secret, as in `?s3cret`.
            None if param.is_empty() => String::new(),
            None => REDACTED.to_owned(),
        })
        .collect();
    out.push_str(&params.join("&"));
    out.push_str(&rest[query_end..]);
    out
}

/// Prepends `http://` to a URL without an explicit scheme and applies Bruno's `encodeUrl` when
/// `encode`, as bruno-cli does, then parses it as a WHATWG URL the way axios's `new URL` would.
fn parse_url(url: &str) -> Result<Url, ResolveError> {
    Url::parse(url).map_err(|_| ResolveError::InvalidUrl {
        url: redact_url(url),
    })
}

/// The enabled query params that `set_params` writes into the URL, as `(name, value)` text.
fn query_cells(params: &[Param]) -> Vec<(&str, &str)> {
    params
        .iter()
        .filter(|p| p.enabled && p.kind == ParamKind::Query && !js::js_trim(&p.name).is_empty())
        .map(|p| (p.name.as_str(), p.value.as_str()))
        .collect()
}

/// `mock` over `template` one piece at a time between the escapes at `spots`, returning the
/// mocked text and where each escape moved; a generated `{{$name}}` never holds a `%`, so the
/// pieces mock exactly as the whole would.
fn mock_around(template: &str, spots: &[usize], generator: &mut Generator) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(template.len());
    let mut moved = Vec::with_capacity(spots.len());
    let mut from = 0;
    for &spot in spots {
        out.push_str(&mock(&template[from..spot], generator));
        moved.push(out.len());
        from = spot;
    }
    out.push_str(&mock(&template[from..], generator));
    (out, moved)
}

/// Where the escapes at `spots` in `raw` land in its expansion `expanded`, as byte offsets
/// from the end, which path params and an added scheme leave unchanged.
/// Each text between escapes is expanded on its own; if that does not rebuild `expanded`
/// exactly, no escape is claimed and the whole query is encoded content-blind.
fn written_escapes(raw: &str, spots: &[usize], expanded: &str, vars: &Vars) -> Vec<usize> {
    let mut rebuilt = String::with_capacity(expanded.len());
    let mut marks = Vec::new();
    let mut from = 0;
    for &spot in spots {
        let Ok(piece) = interpolate(&raw[from..spot], vars) else {
            return Vec::new();
        };
        rebuilt.push_str(&piece);
        marks.push(rebuilt.len());
        rebuilt.push_str(&raw[spot..spot + 3]);
        from = spot + 3;
    }
    let Ok(piece) = interpolate(&raw[from..], vars) else {
        return Vec::new();
    };
    rebuilt.push_str(&piece);
    if rebuilt != expanded {
        return Vec::new();
    }
    marks.iter().map(|mark| expanded.len() - mark).collect()
}

/// The characters a pasted URL often carries at its ends; inner ones stay and fail to parse.
pub(crate) const URL_PADDING: [char; 4] = [' ', '\t', '\r', '\n'];

/// Drops padding from both ends of `url` in place.
fn trim_padding(url: &mut String) {
    let kept = url.trim_end_matches(URL_PADDING).len();
    url.truncate(kept);
    let lead = url.len() - url.trim_start_matches(URL_PADDING).len();
    url.drain(..lead);
}

/// Prefixes `http://` when `url` has no explicit scheme, as Bruno does before parsing. Padding
/// around the URL goes first, so the scheme test and the error text see what was meant. Bruno
/// itself leaves the ends alone; this is a deliberate superset.
pub(crate) fn with_scheme(mut url: String) -> String {
    trim_padding(&mut url);
    if has_explicit_scheme(&url) {
        url
    } else {
        format!("http://{url}")
    }
}

/// Port of Bruno's `hasExplicitScheme`: a WHATWG scheme followed by `://`, so `localhost:8080`
/// has none.
fn has_explicit_scheme(url: &str) -> bool {
    authority_span(url).is_some()
}

/// `URLSearchParams.set`: the first `key` takes `value` and later ones are dropped, or the pair
/// is appended; the whole query is then re-serialized form-encoded.
fn set_query_param(url: &mut Url, key: &str, value: &str) {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut found = false;
    for (name, old) in url.query_pairs().into_owned() {
        if name != key {
            pairs.push((name, old));
        } else if !found {
            found = true;
            pairs.push((name, value.to_owned()));
        }
    }
    if !found {
        pairs.push((key.to_owned(), value.to_owned()));
    }
    url.query_pairs_mut().clear().extend_pairs(pairs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextOptions;

    // A request whose network has no basis cannot be judged, so a set check refuses it unsent.
    #[tokio::test]
    async fn a_request_with_no_network_basis_is_refused_under_a_check() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
        let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/\"\n";
        std::fs::write(dir.path().join("opencollection.yml"), root).unwrap();
        std::fs::write(dir.path().join("r.yml"), req).unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
        let mut resolved = resolve(&c.tree().requests()[0], &ctx).unwrap();
        assert!(resolved.network.basis.is_some());
        resolved.network.basis = None;
        let check = crate::NetworkCheck::new(dir.path().join("ack.json"), |_| async {
            crate::NetworkAnswer::Send
        });
        let exec = crate::Executor::new(crate::ExecOptions::new().network_check(check)).unwrap();
        let err = exec.send(&resolved).await.unwrap_err();
        assert!(matches!(err, crate::ExecError::NetworkDeclined), "{err:?}");
    }

    // The OAuth 2 token fetch takes the timeout of the request it is for, default included.
    #[test]
    fn a_token_request_inherits_the_context_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
        let req = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"http://127.0.0.1:9/\"\n";
        std::fs::write(dir.path().join("opencollection.yml"), root).unwrap();
        std::fs::write(dir.path().join("r.yml"), req).unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new())
            .unwrap()
            .with_default_timeout(Some(Duration::from_secs(3)));
        let url = Url::parse("http://127.0.0.1:9/token").unwrap();
        let token = token_request(
            &c.tree().requests()[0],
            &ctx,
            "http://127.0.0.1:9/token",
            url,
            Vec::new(),
            Vec::new(),
            &[],
        )
        .unwrap();
        assert_eq!(token.timeout(), Some(Duration::from_secs(3)));
    }

    // The three NTLM fields are interpolated into the send auth; blanks stay blank.
    #[test]
    fn ntlm_fields_are_interpolated_into_the_send_auth() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nrequest:\n  variables:\n    - name: u\n      value: alice\n    - name: pw\n      value: s3cret\n    - name: d\n      value: CORP\n";
        let req = |auth: &str| {
            format!(
                "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"https://x.test/\"\n  auth:\n{auth}"
            )
        };
        std::fs::write(dir.path().join("opencollection.yml"), root).unwrap();
        std::fs::write(dir.path().join("a.yml"), req("    type: ntlm\n    username: \"{{u}}\"\n    password: \"{{pw}}\"\n    domain: \"{{d}}\"\n")).unwrap();
        std::fs::write(dir.path().join("b.yml"), req("    type: ntlm\n")).unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
        let by = |name: &str| {
            c.tree()
                .requests()
                .iter()
                .find(|r| r.path().ends_with(name))
                .unwrap()
        };
        let a = resolve(by("a.yml"), &ctx).unwrap();
        assert!(
            matches!(a.send_auth(), Some(SendAuth::Ntlm { username, password, domain })
            if username == "alice" && password == "s3cret" && domain == "CORP")
        );
        let b = resolve(by("b.yml"), &ctx).unwrap();
        assert!(
            matches!(b.send_auth(), Some(SendAuth::Ntlm { username, password, domain })
            if username.is_empty() && password.is_empty() && domain.is_empty())
        );
    }
}
