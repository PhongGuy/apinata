//! What can go wrong talking to a team server or keeping its account.

/// A failed team server call, sign-in or account change. The texts never carry a token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TeamError {
    /// The server address cannot be used; the text says what to enter.
    #[error("{0}")]
    BadServerUrl(String),
    /// The server does not offer password sign-in.
    #[error("This server does not offer password sign-in.")]
    PasswordSignInOff,
    /// The credentials or the token were not accepted.
    #[error("The server did not accept the credentials.")]
    Unauthenticated,
    /// The server refused the call for this user.
    #[error("Not allowed: {0}")]
    Forbidden(String),
    /// The server has no such resource.
    #[error("Not found: {0}")]
    NotFound(String),
    /// The resource already exists or clashes with another.
    #[error("Conflict: {0}")]
    Conflict(String),
    /// A value was refused, naming the field when one is to blame.
    #[error("{message}")]
    Invalid {
        /// The field at fault, when the server or the check names one.
        field: Option<String>,
        /// What is wrong with it.
        message: String,
    },
    /// Too many calls; wait before trying again.
    #[error("Too many attempts; try again later.")]
    RateLimited {
        /// How long the server asked to wait, when it said.
        retry_after_secs: Option<u64>,
    },
    /// The server is up but cannot serve the call now; carries its message, which may be empty.
    #[error("The server is unavailable.")]
    Unavailable(String),
    /// Any other error the server reported.
    #[error("The server failed ({status} {code}): {message}")]
    Server {
        /// The HTTP status.
        status: u16,
        /// The server's error code.
        code: String,
        /// The server's message.
        message: String,
    },
    /// The server could not be reached.
    #[error("The server could not be reached: {0}")]
    Offline(String),
    /// The HTTP client could not be set up on this machine; no call was made.
    #[error("The HTTP client could not be built: {0}")]
    Client(String),
    /// The server took too long to answer.
    #[error("The server took too long to answer.")]
    Timeout,
    /// The server answered with something this client does not understand.
    #[error("Unexpected answer from the server: {0}")]
    Protocol(String),
    /// The keychain could not keep or give back the token.
    #[error("The token could not be kept: {0}")]
    Secrets(String),
    /// The team file could not be read or written.
    #[error("The team file could not be used: {0}")]
    Store(String),
    /// Another process kept the team file locked for the whole wait.
    #[error("Another Apinata window or command is changing the team file; try again.")]
    Busy,
    /// The caller cancelled the call before it kept anything.
    #[error("Cancelled.")]
    Cancelled,
    /// The server does not offer single sign-on.
    #[error("This server does not offer single sign-on.")]
    SsoOff,
    /// The sign-in provider or the server refused the single sign-on.
    #[error("The sign-in was refused.")]
    SsoRefused,
    /// No account on the server is linked to the provider's sign-in.
    #[error("No account on this server is linked to that sign-in.")]
    AccountNotLinked,
    /// The browser came back too late for the server to finish the sign-in.
    #[error("The sign-in took too long in the browser.")]
    SsoExpired,
    /// The server refused the returned code: expired, reused or not this app's.
    #[error("The sign-in could not be completed.")]
    SsoCodeInvalid,
    /// The browser came back with a sign-in this app did not start.
    #[error("The browser returned a sign-in this app did not start.")]
    SsoStateMismatch,
    /// The browser did not come back in time, or the wait was cancelled.
    #[error("No sign-in came back from the browser.")]
    SsoTimeout,
    /// The sign-in could not be started on this computer.
    #[error("The sign-in could not be started: {0}")]
    SsoListen(String),
}

/// The server's error envelope: `{"error": {"code", "message", "field"}}`.
#[derive(serde::Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(serde::Deserialize)]
struct ErrorDetail {
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    field: Option<String>,
}

impl TeamError {
    /// Maps a non-2xx answer by its `error.code`; a body that is not the error envelope keeps
    /// only the status. Server messages pass through `redact_token`, so no token rides along.
    pub(crate) fn from_response(status: u16, retry_after: Option<u64>, body: &[u8]) -> TeamError {
        let Ok(ErrorBody { error }) = serde_json::from_slice::<ErrorBody>(body) else {
            return TeamError::Server {
                status,
                code: String::new(),
                message: String::new(),
            };
        };
        let message = super::redact_token(&error.message);
        match error.code.as_str() {
            "unauthenticated" => TeamError::Unauthenticated,
            "forbidden" => TeamError::Forbidden(message),
            "not_found" => TeamError::NotFound(message),
            "conflict" => TeamError::Conflict(message),
            "invalid_request" => TeamError::Invalid {
                field: error.field,
                message,
            },
            "rate_limited" => TeamError::RateLimited {
                retry_after_secs: retry_after,
            },
            "unavailable" => TeamError::Unavailable(message),
            "oidc_code_invalid" => TeamError::SsoCodeInvalid,
            _ => TeamError::Server {
                status,
                code: super::redact_token(&error.code),
                message,
            },
        }
    }
}
