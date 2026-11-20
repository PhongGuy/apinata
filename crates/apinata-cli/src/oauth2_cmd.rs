//! `apinata oauth2`: the cached OAuth 2.0 tokens `run` uses, shown by expiry, scope and grant
//! but never by value, and cleared per request, folder or collection.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail};
use apinata_core::{
    AuthSource, AuthSpec, Collection, Folder, ItemPath, OAuth2Error, OAuth2Flow, OAuth2Target,
    Persistence, RequestFile, ResolveError, TokenStatus, VarContext,
};
use serde::Serialize;

use crate::import::secret_store;
use crate::run::{self, global_options, token_cache};
use crate::{EXIT_USAGE, Format, OAuth2Action, OAuth2Args, collection_arg, plural, trust};

/// One cached token, its values withheld; the MCP `oauth2_token` fields plus where it lives.
#[derive(Serialize)]
struct TokenDoc {
    /// The request or folder path that defines the auth; `""` for the collection.
    path: String,
    level: &'static str,
    grant_type: &'static str,
    obtained_at: u64,
    expires_at: Option<u64>,
    expired: bool,
    has_refresh_token: bool,
    scope: Option<String>,
}

#[derive(Serialize)]
struct StatusDoc {
    tokens: Vec<TokenDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_only: Option<String>,
}

#[derive(Serialize)]
struct ClearedDoc {
    path: String,
    level: &'static str,
    removed: usize,
}

#[derive(Serialize)]
struct ClearDoc {
    cleared: Vec<ClearedDoc>,
}

/// The level whose auth settings a token belongs to.
enum Level {
    Request(ItemPath),
    Folder(ItemPath),
    Collection,
}

impl Level {
    fn kind(&self) -> &'static str {
        match self {
            Level::Request(_) => "request",
            Level::Folder(_) => "folder",
            Level::Collection => "collection",
        }
    }

    fn path(&self) -> String {
        match self {
            Level::Request(p) | Level::Folder(p) => p.to_string(),
            Level::Collection => String::new(),
        }
    }

    fn label(&self) -> String {
        place(self.kind(), &self.path())
    }
}

/// Whose auth a token action applies to, owning a folder path so the target can borrow it.
enum Subject<'a> {
    Request(&'a RequestFile),
    Folder(ItemPath),
    Collection,
}

/// A level as text: `collection`, or its kind and its path.
fn place(kind: &str, path: &str) -> String {
    match kind {
        "collection" => "collection".to_owned(),
        kind => format!("{kind} {}", trust::shown(path)),
    }
}

/// A token action's subject and the level its auth settings come from.
struct Scope<'a> {
    subject: Subject<'a>,
    level: Level,
}

impl Scope<'_> {
    fn target(&self) -> OAuth2Target<'_> {
        match &self.subject {
            Subject::Request(req) => OAuth2Target::Request(req),
            Subject::Folder(path) => OAuth2Target::Folder(path),
            Subject::Collection => OAuth2Target::Collection,
        }
    }
}

/// Show or clear: 0 when done (clearing nothing included), 3 for any failure.
pub fn run(args: &OAuth2Args) -> ExitCode {
    match go(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(EXIT_USAGE),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Runs the action; `Ok(false)` when some scope of a whole-collection pass failed.
fn go(args: &OAuth2Args) -> anyhow::Result<bool> {
    let root = collection_arg::with_hint(
        trust::directory_root(args.collection.as_deref()),
        "pass the collection with --collection <DIR>",
    )?;
    let c = Collection::open(&root)?;
    let tokens = token_cache();
    // Global variables resolve exactly as for `run`, so the cache key matches the one it wrote.
    let (options, _) = global_options(
        args.global_env.as_deref(),
        args.workspace.as_deref(),
        &args.global_env_var,
    )?;
    let mut options = options
        .oauth2_tokens(tokens.clone())
        .secrets(secret_store(args.no_keychain));
    if let Some(env) = run::chosen_environment(&c, args.env.as_deref(), args.no_env).name() {
        options = options.env(env);
    }
    // Every override is kept sensitive: nothing here prints a value, and a message never should.
    for v in &args.env_var {
        options = options.sensitive_env_var(&v.key, &v.value);
    }
    let ctx = VarContext::new(&c, options)?;
    for warning in ctx.warnings() {
        eprintln!("warning: {warning}");
    }
    let (path, clear) = match &args.action {
        OAuth2Action::Status { path } => (path, false),
        OAuth2Action::Clear { path } => (path, true),
    };
    let scopes = match path {
        Some(raw) => vec![named(&c, &ctx, raw)?],
        None => every_scope(&c),
    };
    let json = matches!(args.format, Format::Json);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        if clear {
            clear_all(&ctx, &scopes, json).await
        } else {
            tokens.load().await;
            status_all(&c, &ctx, &scopes, &tokens.persistence(), json)
        }
    })
}

/// The scope `raw` names: a request or folder path, or `""` or `.` for the collection. Its
/// auth must be OAuth 2.0, inherited or its own.
fn named<'a>(c: &'a Collection, ctx: &VarContext, raw: &str) -> anyhow::Result<Scope<'a>> {
    let path =
        ItemPath::new(raw).ok_or_else(|| anyhow!("`{}` is not an item path", trust::shown(raw)))?;
    let (subject, own) = if path.is_root() {
        (Subject::Collection, Some(Level::Collection))
    } else if let Some(req) = c.request(&path) {
        let own = is_oauth2_spec(&req.auth()).then(|| Level::Request(path.clone()));
        (Subject::Request(req), own)
    } else if let Some(folder) = c.folder(&path) {
        let own = folder_is_oauth2(folder).then(|| Level::Folder(path.clone()));
        (Subject::Folder(path.clone()), own)
    } else {
        bail!("no request or folder at `{}`", trust::shown(raw));
    };
    let level = own.unwrap_or_else(|| inherited(c, &path));
    let scope = Scope { subject, level };
    if !apinata_core::is_oauth2(scope.target(), ctx) {
        // Named by the item asked about, not the level its auth would come from.
        let item = match scope.subject {
            Subject::Request(_) => place("request", path.as_str()),
            Subject::Folder(_) => place("folder", path.as_str()),
            Subject::Collection => place("collection", ""),
        };
        bail!("{item}: its auth is not OAuth 2.0");
    }
    Ok(scope)
}

/// Every level that defines OAuth 2.0 auth of its own: the collection, then folders and
/// requests in tree order. Items that inherit share their parent's token, so they are skipped.
fn every_scope(c: &Collection) -> Vec<Scope<'_>> {
    let mut scopes = Vec::new();
    if is_oauth2_spec(&c.file().auth()) {
        scopes.push(Scope {
            subject: Subject::Collection,
            level: Level::Collection,
        });
    }
    let mut stack = vec![(ItemPath::root(), c.tree())];
    while let Some((path, folder)) = stack.pop() {
        if !path.is_root() && folder_is_oauth2(folder) {
            scopes.push(Scope {
                subject: Subject::Folder(path.clone()),
                level: Level::Folder(path.clone()),
            });
        }
        for req in folder.requests() {
            if let Some(item) = c.item_path(req.path())
                && is_oauth2_spec(&req.auth())
            {
                scopes.push(Scope {
                    subject: Subject::Request(req),
                    level: Level::Request(item),
                });
            }
        }
        for sub in folder.folders().iter().rev() {
            if let Some(child) = path.join(sub.dir_name()) {
                stack.push((child, sub));
            }
        }
    }
    scopes
}

/// The level `item` inherits its auth from.
fn inherited(c: &Collection, item: &ItemPath) -> Level {
    match c.inherited_auth(item).source() {
        AuthSource::Folder(path) => Level::Folder(path.clone()),
        _ => Level::Collection,
    }
}

fn is_oauth2_spec(auth: &AuthSpec) -> bool {
    matches!(auth, AuthSpec::OAuth2(_))
}

fn folder_is_oauth2(folder: &Folder) -> bool {
    folder.file().is_some_and(|f| is_oauth2_spec(&f.auth()))
}

/// The grant the level's auth uses, by its file name.
fn grant_type(c: &Collection, level: &Level) -> &'static str {
    let auth = match level {
        Level::Request(path) => c.request(path).map(RequestFile::auth),
        Level::Folder(path) => c.folder(path).and_then(Folder::file).map(|f| f.auth()),
        Level::Collection => Some(c.file().auth()),
    };
    match auth {
        Some(AuthSpec::OAuth2(spec)) => match spec.flow {
            OAuth2Flow::ClientCredentials => "client_credentials",
            OAuth2Flow::Password => "resource_owner_password_credentials",
            OAuth2Flow::AuthorizationCode => "authorization_code",
            OAuth2Flow::Implicit => "implicit",
            _ => "unknown",
        },
        _ => "unknown",
    }
}

/// `err` as one line, withholding a variable name written inside a secret value.
fn message(err: &OAuth2Error, scope: &Scope<'_>, ctx: &VarContext) -> String {
    let hidden = |name: &str| apinata_core::oauth2_name_hidden(scope.target(), ctx, name);
    let line = match err {
        OAuth2Error::Resolve(ResolveError::UndefinedVariable { name, .. }) if hidden(name) => {
            "a variable written inside a secret value is not defined".to_owned()
        }
        OAuth2Error::Resolve(ResolveError::CyclicVariable { name }) if hidden(name) => {
            "a variable written inside a secret value refers to itself".to_owned()
        }
        other => other.to_string(),
    };
    format!("{}: {line}", scope.level.label())
}

fn status_all(
    c: &Collection,
    ctx: &VarContext,
    scopes: &[Scope<'_>],
    persistence: &Persistence,
    json: bool,
) -> anyhow::Result<bool> {
    let now = SystemTime::now();
    let mut ok = true;
    let mut docs = Vec::new();
    for scope in scopes {
        match apinata_core::oauth2_status(scope.target(), ctx) {
            Ok(Some(status)) => docs.push(token_doc(c, &scope.level, &status, now)),
            Ok(None) => {}
            Err(e) => {
                eprintln!("error: {}", message(&e, scope, ctx));
                ok = false;
            }
        }
    }
    let memory_only = match persistence {
        Persistence::MemoryOnly { reason } => Some(reason.clone()),
        _ => None,
    };
    if json {
        let doc = StatusDoc {
            tokens: docs,
            memory_only,
        };
        outln!("{}", serde_json::to_string(&doc)?);
    } else {
        if let Some(reason) = memory_only {
            eprintln!("note: tokens are kept in memory only, so none is cached: {reason}");
        }
        for doc in &docs {
            outln!("{}", text_line(doc));
        }
    }
    Ok(ok)
}

/// A token as text: where it lives, its grant, its expiry, its scope and its refresh token.
fn text_line(doc: &TokenDoc) -> String {
    let place = place(doc.level, &doc.path);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let expiry = match doc.expires_at {
        _ if doc.expired => "expired".to_owned(),
        Some(at) => format!("expires in {}", span(at.saturating_sub(now))),
        None => "no expiry".to_owned(),
    };
    let scope = match &doc.scope {
        Some(s) if !s.is_empty() => format!("scope \"{}\"", trust::shown(s)),
        _ => "no scope".to_owned(),
    };
    let refresh = if doc.has_refresh_token {
        "refresh token"
    } else {
        "no refresh token"
    };
    format!("{place}: {}, {expiry}, {scope}, {refresh}", doc.grant_type)
}

/// A duration as its two largest units, such as `1h 5m` or `40s`.
fn span(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{s}s"),
        (0, 0, _) => format!("{m}m {s}s"),
        (0, _, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

fn token_doc(c: &Collection, level: &Level, status: &TokenStatus, now: SystemTime) -> TokenDoc {
    let secs = |at: SystemTime| at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    TokenDoc {
        path: level.path(),
        level: level.kind(),
        grant_type: grant_type(c, level),
        obtained_at: secs(status.obtained_at),
        expires_at: status.expires_at.map(secs),
        expired: status.is_expired(now),
        has_refresh_token: status.has_refresh_token,
        scope: status.scope.clone(),
    }
}

async fn clear_all(ctx: &VarContext, scopes: &[Scope<'_>], json: bool) -> anyhow::Result<bool> {
    let mut ok = true;
    let mut cleared = Vec::new();
    for scope in scopes {
        match apinata_core::clear_oauth2_token(scope.target(), ctx).await {
            Ok(0) => {}
            Ok(removed) => cleared.push(ClearedDoc {
                path: scope.level.path(),
                level: scope.level.kind(),
                removed,
            }),
            Err(e) => {
                eprintln!("error: {}", message(&e, scope, ctx));
                ok = false;
            }
        }
    }
    if json {
        outln!("{}", serde_json::to_string(&ClearDoc { cleared })?);
    } else if cleared.is_empty() {
        outln!("no cached token to clear");
    } else {
        for doc in &cleared {
            let place = place(doc.level, &doc.path);
            outln!("cleared {} for {place}", plural(doc.removed, "token"));
        }
    }
    Ok(ok)
}
