//! `list_cookies` and `clear_cookies`: the session's cookie jar, shown without values.

use std::time::UNIX_EPOCH;

use apinata_core::CookieJar;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One stored cookie, its value withheld.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct CookieDoc {
    pub name: String,
    pub domain: String,
    pub path: String,
    /// When it expires, in unix seconds; absent for a session cookie.
    pub expires: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
}

/// What `list_cookies` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct CookieList {
    pub cookies: Vec<CookieDoc>,
}

/// `clear_cookies`'s arguments.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClearArgs {
    /// Clear only this domain's cookies; every cookie when absent.
    pub domain: Option<String>,
}

/// What `clear_cookies` returns.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(crate) struct Cleared {
    pub removed: usize,
}

/// Every unexpired cookie in `jar`, sorted by domain, path and name, without its value.
pub(crate) fn list(jar: &CookieJar) -> CookieList {
    let cookies = jar
        .list()
        .into_iter()
        .map(|c| CookieDoc {
            name: c.name,
            domain: c.domain,
            path: c.path,
            expires: c
                .expires
                .map(|at| at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())),
            secure: c.secure,
            http_only: c.http_only,
        })
        .collect();
    CookieList { cookies }
}

/// Remove `domain`'s cookies, or every cookie when `domain` is `None`, and count them.
pub(crate) fn clear(jar: &CookieJar, domain: Option<&str>) -> Cleared {
    let removed = match domain {
        Some(domain) => jar.delete_domain(domain),
        None => {
            let removed = jar.list().len();
            jar.clear();
            removed
        }
    };
    Cleared { removed }
}
