//! `apinata team`: sign in to a team server by password or single sign-on, show the kept
//! sign-ins and sign out, through the same core flows and keychain entries as the app. Server
//! text is shown with hidden characters escaped, and no token or token id is ever printed.
//! A CI job can instead hand `status` a token through `APINATA_TEAM_TOKEN` for the one server
//! in `APINATA_TEAM_SERVER`; that token never touches the keychain or the team file.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use apinata_core::team::{
    self, Account, HttpTeamClient, Me, SSO_TIMEOUT, ServerUrl, TEAM_SERVER_VAR, TEAM_TOKEN_VAR,
    TeamError, TeamStore, TeamToken, escape_hidden, redact_token,
};
use apinata_core::{CancelFlag, CancelHandle, SecretStore};
use clap::Subcommand;
use serde::Serialize;

use crate::import::{platform_store, team_file};
use crate::{EXIT_USAGE, Format};

/// Signed out, expired or refused.
const EXIT_REFUSED: u8 = 1;
/// The server could not be reached or its answer was not understood.
const EXIT_SERVER: u8 = 2;
/// Stopped with Ctrl+C.
const EXIT_CANCELLED: u8 = 130;

#[derive(clap::Args)]
pub struct TeamArgs {
    #[command(subcommand)]
    pub action: TeamAction,
    /// Do not use the keychain: sign-in and sign-out refuse to run, and status shows only the
    /// kept account and its expiry without checking the token (a token in APINATA_TEAM_TOKEN
    /// is still checked).
    #[arg(long, global = true)]
    pub no_keychain: bool,
}

#[derive(Subcommand)]
pub enum TeamAction {
    /// Sign in with a password read from stdin or a hidden prompt, or with single sign-on.
    #[command(group(clap::ArgGroup::new("how").required(true).args(["username", "sso"])))]
    SignIn {
        /// The server address, such as https://team.example.com.
        server: String,
        /// Sign in as NAME; the password is read from stdin or a hidden prompt.
        #[arg(long, value_name = "NAME")]
        username: Option<String>,
        /// Sign in in a browser: prints the address, opens it in the default browser and waits
        /// for it to come back.
        #[arg(long)]
        sso: bool,
        /// With --sso, only print the address; no browser is opened.
        #[arg(long, requires = "sso")]
        no_browser: bool,
    },
    /// Revoke the sign-in on the server and forget it here.
    SignOut {
        /// The server address.
        server: String,
    },
    /// Show the kept sign-ins, and the token in APINATA_TEAM_TOKEN, and check each with its
    /// server.
    Status {
        /// Only this server; every kept sign-in when absent.
        server: Option<String>,
        /// Print the result as text or as JSON.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
}

/// What ended a command: its exit code and the one line for stderr.
struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn usage(message: impl Into<String>) -> Failure {
        Failure {
            code: EXIT_USAGE,
            message: message.into(),
        }
    }
}

impl From<TeamError> for Failure {
    fn from(e: TeamError) -> Failure {
        Failure {
            code: code_for(&e),
            message: e.to_string(),
        }
    }
}

pub fn run(args: &TeamArgs) -> ExitCode {
    let result = match &args.action {
        TeamAction::SignIn {
            server,
            username,
            sso,
            no_browser,
        } => sign_in(
            server,
            username.as_deref(),
            *sso,
            *no_browser,
            args.no_keychain,
        ),
        TeamAction::SignOut { server } => sign_out(server, args.no_keychain),
        TeamAction::Status { server, format } => {
            status(server.as_deref(), *format, args.no_keychain)
        }
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(failure) if failure.code == EXIT_CANCELLED => {
            eprintln!("{}", failure.message);
            ExitCode::from(failure.code)
        }
        Err(failure) => {
            eprintln!("error: {}", shown(&failure.message));
            ExitCode::from(failure.code)
        }
    }
}

/// Text from the server or the user made safe for a terminal; the token redaction is a
/// backstop, since no flow hands a token to anything printed.
fn shown(text: &str) -> String {
    escape_hidden(&redact_token(text))
}

/// Refusals and signed-out or expired sign-ins are 1, the server's own trouble 2, local trouble 3.
fn code_for(e: &TeamError) -> u8 {
    match e {
        TeamError::Offline(_)
        | TeamError::Timeout
        | TeamError::Protocol(_)
        | TeamError::Unavailable(_)
        | TeamError::Server { .. } => EXIT_SERVER,
        TeamError::BadServerUrl(_)
        | TeamError::Client(_)
        | TeamError::Secrets(_)
        | TeamError::Store(_)
        | TeamError::Busy
        | TeamError::SsoListen(_) => EXIT_USAGE,
        _ => EXIT_REFUSED,
    }
}

fn parse_server(text: &str) -> Result<ServerUrl, Failure> {
    ServerUrl::parse(text).map_err(Failure::from)
}

fn the_team_file() -> Result<PathBuf, Failure> {
    team_file().ok_or_else(|| Failure::usage("this system has no local data directory"))
}

fn refuse_without_keychain(no_keychain: bool, what: &str) -> Result<(), Failure> {
    if no_keychain {
        return Err(Failure::usage(format!(
            "{what} keeps the session token in the keychain, so it does not run with --no-keychain"
        )));
    }
    Ok(())
}

/// A token a CI job supplies through the environment, and the one server it may go to.
struct EnvToken {
    server: ServerUrl,
    token: TeamToken,
}

/// The variable's text, an empty one counting as unset; the error never shows the value.
fn read_var(name: &str) -> Result<Option<String>, Failure> {
    match std::env::var_os(name) {
        None => Ok(None),
        Some(value) if value.is_empty() => Ok(None),
        Some(value) => value
            .into_string()
            .map(Some)
            .map_err(|_| Failure::usage(format!("{name} is not valid text"))),
    }
}

/// Reads `APINATA_TEAM_TOKEN` and `APINATA_TEAM_SERVER`. A token needs its server, both are
/// parsed here, and plain http to another machine is refused, since the token is long-lived.
fn env_token() -> Result<Option<EnvToken>, Failure> {
    let Some(token) = read_var(TEAM_TOKEN_VAR)? else {
        return Ok(None);
    };
    let token = TeamToken::parse(&token).map_err(|_| {
        Failure::usage(format!(
            "{TEAM_TOKEN_VAR} does not hold a team token; copy the whole token the server showed"
        ))
    })?;
    let Some(server) = read_var(TEAM_SERVER_VAR)? else {
        return Err(Failure::usage(format!(
            "{TEAM_TOKEN_VAR} is set but {TEAM_SERVER_VAR} is not; set it to the server the token is for"
        )));
    };
    let server =
        ServerUrl::parse(&server).map_err(|e| Failure::usage(format!("{TEAM_SERVER_VAR}: {e}")))?;
    if server.is_unencrypted() {
        return Err(Failure::usage(format!(
            "{TEAM_SERVER_VAR} is a plain http address, so the token would be sent unencrypted; use https"
        )));
    }
    Ok(Some(EnvToken { server, token }))
}

/// The server a token in the environment is meant for, however well-formed the token is, so
/// sign-out can refuse for it alone.
fn env_server() -> Option<ServerUrl> {
    read_var(TEAM_TOKEN_VAR).ok().flatten()?;
    ServerUrl::parse(&read_var(TEAM_SERVER_VAR).ok().flatten()?).ok()
}

fn runtime() -> Result<tokio::runtime::Runtime, Failure> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Failure::usage(e.to_string()))
}

/// The hint every signed-out or expired sign-in ends with.
fn sign_in_hint(server: &str) -> String {
    format!("run \"apinata team sign-in {server}\"")
}

/// The password from stdin or a hidden prompt, without its line ending; empty is refused.
fn password() -> Result<String, Failure> {
    use std::io::IsTerminal as _;
    let line = crate::env_cmd::read_secret_line("Password: ").map_err(|e| {
        match e.downcast_ref::<crate::env_cmd::PromptCancelled>() {
            Some(cancelled) => Failure {
                code: EXIT_CANCELLED,
                message: cancelled.to_string(),
            },
            None => Failure::usage(format!("{e:#}")),
        }
    })?;
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Err(Failure::usage(if std::io::stdin().is_terminal() {
            "the password is empty"
        } else {
            "the password on stdin is empty"
        }));
    }
    Ok(line.to_owned())
}

fn sign_in(
    server: &str,
    username: Option<&str>,
    sso: bool,
    no_browser: bool,
    no_keychain: bool,
) -> Result<u8, Failure> {
    let server = parse_server(server)?;
    refuse_without_keychain(no_keychain, "sign-in")?;
    let file = the_team_file()?;
    // The prompt handles its own Ctrl+C, so it comes before the handler for the sign-in.
    let password = match username {
        Some(_) if !sso => Some(password()?),
        _ => None,
    };
    let secrets = platform_store();
    let api = HttpTeamClient::new(server.clone())?;
    let shown_server = shown(server.as_str());
    let runtime = runtime()?;
    runtime.block_on(async move {
        let flag = CancelFlag::new();
        let handle = CancelHandle::new();
        // The handler must exist before anything is sent, or an early Ctrl+C kills the process
        // with nothing revoked.
        #[cfg(unix)]
        let interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt());
        #[cfg(windows)]
        let interrupt = tokio::signal::windows::ctrl_c();
        let mut interrupt = interrupt.map_err(|e| Failure::usage(e.to_string()))?;
        let (watch_flag, watch_handle) = (flag.clone(), handle.clone());
        let watcher = tokio::spawn(async move {
            interrupt.recv().await;
            // A call in flight runs to its end, so core can give back a token already issued.
            eprintln!("Cancelling...");
            watch_flag.cancel();
            watch_handle.cancel();
        });
        let result = match (username, password) {
            (Some(username), Some(password)) => {
                team::sign_in(&api, &*secrets, &file, username, &password, &flag).await
            }
            _ => sso_sign_in(&api, &secrets, &file, &handle, &shown_server, no_browser).await,
        };
        watcher.abort();
        let cancelled = flag.is_cancelled() || handle.is_cancelled();
        match result {
            Ok(account) => {
                outln!("{}", signed_in_line(&account, &shown_server));
                Ok(0)
            }
            Err(_) if cancelled => {
                eprintln!("Cancelled; nothing was kept.");
                Ok(EXIT_CANCELLED)
            }
            Err(e) => Err(e.into()),
        }
    })
}

/// Single sign-on: the start URL and loopback port go to stderr, the URL is opened in a browser
/// unless `no_browser` or the environment rules it out, and the wait lasts until the browser
/// comes back, `SSO_TIMEOUT` passes or Ctrl+C cancels `handle`.
async fn sso_sign_in(
    api: &HttpTeamClient,
    secrets: &Arc<dyn SecretStore>,
    file: &Path,
    handle: &CancelHandle,
    shown_server: &str,
    no_browser: bool,
) -> Result<Account, TeamError> {
    let Some(provider) = api_info_sso(api).await? else {
        return Err(TeamError::SsoOff);
    };
    if handle.is_cancelled() {
        return Err(TeamError::Cancelled);
    }
    let pending = team::begin_sso(api).await?;
    let (host, port) = loopback(pending.url());
    eprintln!(
        "Open this address in a browser to sign in to {shown_server} with {}:",
        shown(&provider)
    );
    eprintln!("  {}", shown(pending.url()));
    if !no_browser && !browser_unlikely() && !open_browser(pending.url()) {
        eprintln!("could not open a browser; open the address above");
    }
    eprintln!(
        "Waiting up to {} minutes for the browser on {host} port {port}; press Ctrl+C to cancel.",
        SSO_TIMEOUT.as_secs() / 60
    );
    eprintln!("Over SSH, forward the port first: ssh -L {port}:{host}:{port} <host>");
    let finished = pending.finish(api, &**secrets, file, SSO_TIMEOUT, handle);
    // A refused redeem is about the browser sign-in, not a username and password.
    finished.await.map_err(|e| match e {
        TeamError::Unauthenticated => TeamError::SsoRefused,
        e => e,
    })
}

/// True where a browser would not reach the person at the keyboard: a CI job, a Unix
/// desktop session with no display, or an SSH login to a Mac or Windows machine.
fn browser_unlikely() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    if set("CI") {
        return true;
    }
    if cfg!(any(target_os = "macos", windows)) {
        set("SSH_CONNECTION")
    } else {
        !set("DISPLAY") && !set("WAYLAND_DISPLAY")
    }
}

/// Starts the first opener that will run, detached from the terminal and without the team
/// token; it is never waited on, so a slow or failing opener cannot hold up the sign-in.
fn open_browser(url: &str) -> bool {
    use std::process::Stdio;
    for mut opener in open::commands(url) {
        opener
            .env_remove(TEAM_TOKEN_VAR)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(mut child) = opener.spawn() {
            // Reaped off the main thread, so a finished opener leaves no zombie behind.
            std::thread::spawn(move || child.wait());
            return true;
        }
    }
    false
}

async fn api_info_sso(api: &HttpTeamClient) -> Result<Option<String>, TeamError> {
    use apinata_core::team::TeamApi as _;
    Ok(api.server_info().await?.sso.map(|p| p.name))
}

/// The loopback host and port the start URL tells the server to send the browser back to.
fn loopback(start: &str) -> (&'static str, String) {
    let Ok(url) = url::Url::parse(start) else {
        return ("127.0.0.1", String::new());
    };
    let value = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    };
    let host = if value("loopback").as_deref() == Some("v6") {
        "[::1]"
    } else {
        "127.0.0.1"
    };
    (host, value("port").unwrap_or_default())
}

fn signed_in_line(account: &Account, shown_server: &str) -> String {
    let who = format!(
        "{} ({})",
        shown(&account.display_name),
        shown(&account.username)
    );
    match &account.expires_at {
        Some(at) => format!(
            "Signed in to {shown_server} as {who}; the sign-in expires at {}.",
            shown(at)
        ),
        None => format!("Signed in to {shown_server} as {who}."),
    }
}

fn sign_out(server: &str, no_keychain: bool) -> Result<u8, Failure> {
    let server = parse_server(server)?;
    let shown_server = shown(server.as_str());
    if env_server().is_some_and(|env| env == server) {
        return Err(Failure::usage(format!(
            "{shown_server} uses {TEAM_TOKEN_VAR}, and the token comes from the environment; unset the variable, or revoke the token on the server"
        )));
    }
    refuse_without_keychain(no_keychain, "sign-out")?;
    let file = the_team_file()?;
    if TeamStore::load(&file)?.account(&server).is_none() {
        outln!("Not signed in to {shown_server}; nothing to sign out of.");
        return Ok(0);
    }
    let secrets = platform_store();
    let api = HttpTeamClient::new(server)?;
    runtime()?.block_on(team::sign_out(&api, &*secrets, &file))?;
    outln!("Signed out of {shown_server}.");
    Ok(0)
}

/// One kept sign-in as `status` reports it; it never carries the token or its id.
#[derive(Serialize)]
struct Row {
    server: String,
    username: String,
    display_name: String,
    server_admin: bool,
    expires_at: Option<String>,
    /// `signed_in`, `unchecked`, `expired`, `signed_out` or `error`.
    state: &'static str,
    /// `environment` for the token from `APINATA_TEAM_TOKEN`; absent for a kept sign-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'static str>,
}

impl Row {
    fn new(account: &Account, state: &'static str) -> Row {
        Row {
            server: shown(&account.server),
            username: shown(&account.username),
            display_name: shown(&account.display_name),
            server_admin: account.server_admin,
            expires_at: account.expires_at.as_deref().map(shown),
            state,
            source: None,
        }
    }

    /// The user the environment's token belongs to on `server`.
    fn from_env(server: &ServerUrl, me: &Me) -> Row {
        Row {
            server: shown(server.as_str()),
            username: shown(&me.user.username),
            display_name: shown(&me.user.display_name),
            server_admin: me.user.server_admin,
            expires_at: None,
            state: "signed_in",
            source: Some("environment"),
        }
    }

    fn text(&self) -> String {
        let who = format!("{} ({})", self.display_name, self.username);
        if self.source.is_some() {
            return format!(
                "{}: signed in as {who} (token from {TEAM_TOKEN_VAR})",
                self.server
            );
        }
        let expiry = match &self.expires_at {
            Some(at) if self.state == "expired" => format!("expired at {at}"),
            Some(at) => format!("expires at {at}"),
            None => "does not expire".to_owned(),
        };
        match self.state {
            "signed_in" => format!("{}: signed in as {who}, {expiry}", self.server),
            "unchecked" => format!(
                "{}: {who}, {expiry}; the token was not checked (--no-keychain)",
                self.server
            ),
            "expired" => format!("{}: {who}, {expiry}", self.server),
            "signed_out" => format!("{}: signed out ({})", self.server, self.username),
            _ => format!("{}: {who}, could not be checked", self.server),
        }
    }
}

fn status(server: Option<&str>, format: Format, no_keychain: bool) -> Result<u8, Failure> {
    let env = env_token()?;
    let file = the_team_file()?;
    let wanted = server.map(parse_server).transpose()?;
    // The environment's server is checked with its token alone; its kept account, if any, is
    // left out, and every other server is checked through the keychain as before.
    let env = env.filter(|env| wanted.as_ref().is_none_or(|w| *w == env.server));
    // Asked about the env server alone, the team file is not read; asked about all, a file that
    // cannot be read fails only the other servers.
    let (store, unreadable) = match (env.is_some(), wanted.is_some()) {
        (true, true) => (TeamStore::default(), None),
        (true, false) => match TeamStore::load(&file) {
            Ok(store) => (store, None),
            Err(e) => (TeamStore::default(), Some(Failure::from(e))),
        },
        (false, _) => (TeamStore::load(&file)?, None),
    };
    let accounts: Vec<Account> = match &wanted {
        Some(server) => store.account(server).cloned().into_iter().collect(),
        None => store.accounts().to_vec(),
    };
    let accounts: Vec<Account> = accounts
        .into_iter()
        .filter(|a| {
            env.as_ref()
                .is_none_or(|env| a.server != env.server.as_str())
        })
        .collect();
    let mut code = 0;
    let mut rows = Vec::new();
    if accounts.is_empty() && env.is_none() {
        code = EXIT_REFUSED;
        match server.map(ServerUrl::parse) {
            Some(Ok(server)) => {
                let s = shown(server.as_str());
                eprintln!("error: not signed in to {s}; {}", sign_in_hint(&s));
            }
            _ => eprintln!("error: not signed in to any team server"),
        }
    }
    // The keychain opens only when a kept sign-in needs its token.
    let secrets = (!no_keychain && !accounts.is_empty()).then(platform_store);
    let runtime = runtime()?;
    if let Some(env) = &env {
        let (row, row_code) = check_env(env, &runtime);
        code = code.max(row_code);
        rows.extend(row);
    }
    if let Some(failure) = unreadable {
        eprintln!("error: {}", shown(&failure.message));
        code = code.max(failure.code);
    }
    for account in &accounts {
        let (row, row_code) = check(account, secrets.as_ref(), &file, &runtime);
        code = code.max(row_code);
        rows.push(row);
    }
    match format {
        Format::Json => {
            let json =
                serde_json::to_string_pretty(&rows).map_err(|e| Failure::usage(e.to_string()))?;
            outln!("{json}");
        }
        Format::Text => {
            for row in &rows {
                outln!("{}", row.text());
            }
        }
    }
    Ok(code)
}

/// One account's row and exit code. An expired sign-in and `--no-keychain` (no `secrets`)
/// never reach the server; otherwise the server says whether the token still works.
fn check(
    account: &Account,
    secrets: Option<&Arc<dyn SecretStore>>,
    file: &Path,
    runtime: &tokio::runtime::Runtime,
) -> (Row, u8) {
    let server = shown(&account.server);
    if account.expired() {
        eprintln!(
            "error: the sign-in to {server} has expired; {}",
            sign_in_hint(&server)
        );
        return (Row::new(account, "expired"), EXIT_REFUSED);
    }
    let Some(secrets) = secrets else {
        return (Row::new(account, "unchecked"), 0);
    };
    let checked = ServerUrl::parse(&account.server)
        .and_then(HttpTeamClient::new)
        .map(|api| runtime.block_on(async { team::current(&api, &**secrets, file).await }));
    match checked.and_then(|r| r) {
        Ok(me) => {
            let mut row = Row::new(account, "signed_in");
            row.display_name = shown(&me.user.display_name);
            row.server_admin = me.user.server_admin;
            (row, 0)
        }
        Err(TeamError::Unauthenticated) => {
            eprintln!(
                "error: the sign-in to {server} is no longer accepted; {}",
                sign_in_hint(&server)
            );
            (Row::new(account, "signed_out"), EXIT_REFUSED)
        }
        Err(e) => {
            eprintln!("error: {server}: {}", shown(&e.to_string()));
            (Row::new(account, "error"), code_for(&e))
        }
    }
}

/// The environment's token checked with its server, touching neither store. A refused or
/// unreachable check gives no row, only its error line and exit code.
fn check_env(env: &EnvToken, runtime: &tokio::runtime::Runtime) -> (Option<Row>, u8) {
    let server = shown(env.server.as_str());
    let checked = HttpTeamClient::new(env.server.clone())
        .map(|api| runtime.block_on(async { team::whoami(&api, &env.token).await }));
    match checked.and_then(|r| r) {
        Ok(me) => (Some(Row::from_env(&env.server, &me)), 0),
        Err(TeamError::Unauthenticated) => {
            eprintln!("error: {server} does not accept the token in {TEAM_TOKEN_VAR}");
            (None, EXIT_REFUSED)
        }
        Err(e) => {
            eprintln!("error: {server}: {}", shown(&e.to_string()));
            (None, code_for(&e))
        }
    }
}
