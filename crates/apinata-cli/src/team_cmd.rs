//! `apinata team`: sign in to a team server by password or single sign-on, show the kept
//! sign-ins and sign out, through the same core flows and keychain entries as the app. Server
//! text is shown with hidden characters escaped, and no token or token id is ever printed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use apinata_core::team::{
    self, Account, HttpTeamClient, SSO_TIMEOUT, ServerUrl, TeamError, TeamStore, escape_hidden,
    redact_token,
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
    /// kept account and its expiry without checking the token.
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
        /// Sign in in a browser: prints the address to open and waits for it to come back.
        #[arg(long)]
        sso: bool,
    },
    /// Revoke the sign-in on the server and forget it here.
    SignOut {
        /// The server address.
        server: String,
    },
    /// Show the kept sign-ins and check each with its server.
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
        } => sign_in(server, username.as_deref(), *sso, args.no_keychain),
        TeamAction::SignOut { server } => sign_out(server, args.no_keychain),
        TeamAction::Status { server, format } => {
            status(server.as_deref(), *format, args.no_keychain)
        }
    };
    match result {
        Ok(code) => ExitCode::from(code),
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

/// The Decision's exit codes: refusals are 1, the server's own trouble 2, local trouble 3.
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
    let line = crate::env_cmd::read_secret_line("Password: ")
        .map_err(|e| Failure::usage(format!("{e:#}")))?;
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Err(Failure::usage("the password on stdin is empty"));
    }
    Ok(line.to_owned())
}

fn sign_in(
    server: &str,
    username: Option<&str>,
    sso: bool,
    no_keychain: bool,
) -> Result<u8, Failure> {
    let server = parse_server(server)?;
    refuse_without_keychain(no_keychain, "sign-in")?;
    let file = the_team_file()?;
    // Read before the Ctrl+C handler exists, so Ctrl+C at the prompt ends the process at once.
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
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                .map_err(|e| Failure::usage(e.to_string()))?;
        let (watch_flag, watch_handle) = (flag.clone(), handle.clone());
        let watcher = tokio::spawn(async move {
            #[cfg(unix)]
            interrupt.recv().await;
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            watch_flag.cancel();
            watch_handle.cancel();
        });
        let result = match (username, password) {
            (Some(username), Some(password)) => {
                team::sign_in(&api, &*secrets, &file, username, &password, &flag).await
            }
            _ => sso_sign_in(&api, &secrets, &file, &handle, &shown_server).await,
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

/// Single sign-on: the start URL and loopback port go to stderr, and the wait lasts until the
/// browser comes back, `SSO_TIMEOUT` passes or Ctrl+C cancels `handle`.
async fn sso_sign_in(
    api: &HttpTeamClient,
    secrets: &Arc<dyn SecretStore>,
    file: &Path,
    handle: &CancelHandle,
    shown_server: &str,
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
    refuse_without_keychain(no_keychain, "sign-out")?;
    let file = the_team_file()?;
    let shown_server = shown(server.as_str());
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
        }
    }

    fn text(&self) -> String {
        let who = format!("{} ({})", self.display_name, self.username);
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
    let file = the_team_file()?;
    let store = TeamStore::load(&file)?;
    let accounts: Vec<Account> = match server {
        Some(text) => {
            let server = parse_server(text)?;
            store.account(&server).cloned().into_iter().collect()
        }
        None => store.accounts().to_vec(),
    };
    let mut code = 0;
    let mut rows = Vec::new();
    if accounts.is_empty() {
        code = EXIT_REFUSED;
        match server.map(ServerUrl::parse) {
            Some(Ok(server)) => {
                let s = shown(server.as_str());
                eprintln!("error: not signed in to {s}; {}", sign_in_hint(&s));
            }
            _ => eprintln!("error: not signed in to any team server"),
        }
    }
    let secrets = (!no_keychain).then(platform_store);
    let runtime = runtime()?;
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
