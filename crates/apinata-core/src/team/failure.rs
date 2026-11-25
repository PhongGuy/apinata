//! Reading a failed sync's git output as the reason a team sync stopped.

use crate::{SyncError, redact_url};

use super::redact_token;

/// Why a team push or pull failed, in the terms the sync loop acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyncFailure {
    /// The server did not accept the token (401).
    Unauthenticated,
    /// The token is valid but may not do this (403).
    Forbidden,
    /// The server has no such repository (404).
    NotFound,
    /// The server could not be reached.
    Offline {
        /// Git's text.
        message: String,
    },
    /// The remote moved on; a pull comes first.
    NonFastForward,
    /// The server's hook refused the pushed commits, for secrets or other reasons: the listed
    /// findings and `more` unlisted.
    Refused {
        /// The findings the hook listed.
        findings: Vec<Finding>,
        /// Findings beyond the listed ones.
        more: usize,
    },
    /// Anything else, with git's text.
    Other {
        /// Git's text.
        message: String,
    },
}

/// One thing the server's hook refused, as its `potluck-reject` line names it. The fields hold the
/// server's text exactly; pass each through [`escape_hidden`](super::escape_hidden) before showing it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Finding {
    /// The kind of refusal, such as `secret_env_file` or `format_unsupported`.
    pub code: String,
    /// The ref the commit was pushed to.
    pub git_ref: String,
    /// The file holding it, relative to the repository.
    pub path: String,
    /// The rule that matched.
    pub rule: String,
    /// What the server added about it: names and key paths, never values.
    pub detail: Option<String>,
}

impl Finding {
    /// A finding of kind `code` in `path` on `git_ref`, matched by `rule`, without detail.
    pub fn new(
        code: impl Into<String>,
        git_ref: impl Into<String>,
        path: impl Into<String>,
        rule: impl Into<String>,
    ) -> Finding {
        Finding {
            code: code.into(),
            git_ref: git_ref.into(),
            path: path.into(),
            rule: rule.into(),
            detail: None,
        }
    }

    /// This finding with `detail` attached.
    pub fn with_detail(self, detail: impl Into<String>) -> Finding {
        Finding {
            detail: Some(detail.into()),
            ..self
        }
    }

    /// Whether this finding is a secret rather than another kind of refusal.
    pub fn is_secret(&self) -> bool {
        self.code.starts_with("secret_")
    }
}

/// What curl, through git, prints when the server is unreachable, drops the connection, or sits
/// behind a proxy whose upstream is down. Each one clears on its own, so the loop retries it.
const OFFLINE_SIGNS: &[&str] = &[
    "Failed to connect to",
    "Could not resolve host:",
    "Connection reset by peer",
    "Connection timed out",
    "Operation timed out",
    "Empty reply from server",
    "transfer closed with outstanding read data",
    "Send failure:",
    "SSL_ERROR_SYSCALL",
    "The requested URL returned error: 502",
    "The requested URL returned error: 503",
    "The requested URL returned error: 504",
];

/// The failure `err` stands for. Git's text is matched against what git prints for each HTTP
/// refusal and for an unreachable host, before a push rejection's reason is read; every message
/// kept has URL userinfo and tokens masked.
pub fn classify(err: &SyncError) -> SyncFailure {
    let stderr = match err {
        SyncError::Failed { stderr }
        | SyncError::Partial { stderr, .. }
        | SyncError::Rejected { stderr, .. } => stderr.as_str(),
        _ => "",
    };
    let message = || redact_token(&redact_url(&err.to_string()));
    if stderr.contains("could not read Username for") || stderr.contains("returned error: 401") {
        return SyncFailure::Unauthenticated;
    }
    if stderr.contains("returned error: 403") {
        return SyncFailure::Forbidden;
    }
    if (stderr.contains("fatal: repository '") && stderr.contains("' not found"))
        || stderr.contains("returned error: 404")
    {
        return SyncFailure::NotFound;
    }
    if OFFLINE_SIGNS.iter().any(|sign| stderr.contains(sign)) {
        return SyncFailure::Offline { message: message() };
    }
    if let SyncError::Rejected { reason, stderr } = err {
        if reason == "non-fast-forward" || reason == "fetch first" {
            return SyncFailure::NonFastForward;
        }
        let (findings, more) = findings(stderr);
        if !findings.is_empty() || more > 0 {
            return SyncFailure::Refused { findings, more };
        }
    }
    SyncFailure::Other { message: message() }
}

/// The hook's `remote: potluck-reject <code> <ref> <path> <rule>` lines and the count from its
/// `potluck-reject-more <n>` line. Git pads remote lines with spaces, so each is trimmed first. The
/// ref and path arrive percent-encoded, so a line is exactly five space-separated fields; any
/// other field count or a malformed escape drops the line.
///
/// A `potluck-reject-detail <text>` line directly after a kept finding attaches to it; anywhere
/// else, or with a malformed escape, it is ignored.
fn findings(stderr: &str) -> (Vec<Finding>, usize) {
    let mut found: Vec<Finding> = Vec::new();
    let mut more = 0;
    // Set only on the line right after a kept finding, so a detail never reaches an older one.
    let mut open = false;
    for line in stderr.lines() {
        let line = line.trim();
        let Some(line) = line.strip_prefix("remote: ") else {
            continue;
        };
        let line = line.trim();
        let after_finding = std::mem::take(&mut open);
        if let Some(text) = line.strip_prefix("potluck-reject-detail ") {
            if after_finding && let Some(last) = found.last_mut() {
                last.detail = percent_decode(text.trim());
            }
        } else if let Some(count) = line.strip_prefix("potluck-reject-more ") {
            more = count.trim().parse().unwrap_or(more);
        } else if line.starts_with("potluck-reject ") {
            let parts: Vec<&str> = line.split(' ').collect();
            if let [_, code, git_ref, path, rule] = parts.as_slice()
                && let (Some(git_ref), Some(path)) = (percent_decode(git_ref), percent_decode(path))
            {
                found.push(Finding {
                    code: (*code).to_owned(),
                    git_ref,
                    path,
                    rule: (*rule).to_owned(),
                    detail: None,
                });
                open = true;
            }
        }
    }
    (found, more)
}

/// `text` with each `%XX` (either hex case) turned into its byte, read as UTF-8 with invalid
/// bytes shown as U+FFFD. A `%` not followed by two hex digits yields `None`.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            // from_str_radix accepts a leading `+`, which is not a hex digit.
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}
