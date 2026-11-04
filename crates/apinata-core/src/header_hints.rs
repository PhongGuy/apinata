//! Header names and common values offered while a request header is typed.
//!
//! The names are the request-relevant subset of the IANA message header registry, in
//! canonical casing; the values are the ones people type most for the headers that have a
//! small vocabulary.

/// Request header names in the order they are offered: the ones used daily first.
const NAMES: &[&str] = &[
    "Accept",
    "Authorization",
    "Content-Type",
    "Content-Length",
    "Cache-Control",
    "Cookie",
    "User-Agent",
    "Accept-Encoding",
    "Accept-Language",
    "Accept-Charset",
    "Connection",
    "Host",
    "Origin",
    "Referer",
    "If-Match",
    "If-None-Match",
    "If-Modified-Since",
    "If-Unmodified-Since",
    "If-Range",
    "Range",
    "Content-Encoding",
    "Content-Language",
    "Content-Location",
    "Content-MD5",
    "Content-Disposition",
    "Date",
    "ETag",
    "Expect",
    "Forwarded",
    "From",
    "Max-Forwards",
    "Pragma",
    "Prefer",
    "Proxy-Authorization",
    "TE",
    "Trailer",
    "Transfer-Encoding",
    "Upgrade",
    "Via",
    "Warning",
    "Access-Control-Request-Method",
    "Access-Control-Request-Headers",
    "DNT",
    "Sec-Fetch-Dest",
    "Sec-Fetch-Mode",
    "Sec-Fetch-Site",
    "Upgrade-Insecure-Requests",
    "X-Api-Key",
    "X-Forwarded-For",
    "X-Forwarded-Host",
    "X-Forwarded-Proto",
    "X-Requested-With",
    "X-Request-Id",
    "X-Correlation-Id",
    "X-HTTP-Method-Override",
];

/// Common values per header, keyed by lowercase name.
const VALUES: &[(&str, &[&str])] = &[
    (
        "content-type",
        &[
            "application/json",
            "application/x-www-form-urlencoded",
            "multipart/form-data",
            "text/plain",
            "text/html",
            "application/xml",
            "text/xml",
            "application/octet-stream",
            "application/graphql",
            "application/ld+json",
            "application/merge-patch+json",
            "application/json-patch+json",
            "application/problem+json",
            "text/csv",
            "image/png",
            "image/jpeg",
        ],
    ),
    (
        "accept",
        &[
            "application/json",
            "*/*",
            "text/plain",
            "text/html",
            "application/xml",
            "text/xml",
            "application/octet-stream",
            "application/problem+json",
            "text/event-stream",
            "text/csv",
        ],
    ),
    (
        "cache-control",
        &[
            "no-cache",
            "no-store",
            "max-age=0",
            "must-revalidate",
            "public",
            "private",
            "no-transform",
            "only-if-cached",
        ],
    ),
    (
        "accept-encoding",
        &[
            "gzip",
            "deflate",
            "br",
            "zstd",
            "identity",
            "gzip, deflate, br",
        ],
    ),
    (
        "authorization",
        &[
            "Bearer",
            "Basic",
            "Digest",
            "Negotiate",
            "NTLM",
            "AWS4-HMAC-SHA256",
        ],
    ),
    (
        "proxy-authorization",
        &["Bearer", "Basic", "Digest", "Negotiate", "NTLM"],
    ),
    (
        "accept-language",
        &["en-US", "en", "en-GB", "da", "de", "fr", "es", "*"],
    ),
    ("accept-charset", &["utf-8", "iso-8859-1", "*"]),
    ("connection", &["keep-alive", "close", "Upgrade"]),
    ("upgrade", &["websocket", "h2c"]),
    ("pragma", &["no-cache"]),
    ("expect", &["100-continue"]),
    (
        "transfer-encoding",
        &["chunked", "gzip", "deflate", "identity"],
    ),
    (
        "content-encoding",
        &["gzip", "deflate", "br", "zstd", "identity"],
    ),
    ("te", &["trailers", "gzip", "deflate"]),
    ("x-requested-with", &["XMLHttpRequest"]),
    ("dnt", &["1", "0"]),
    (
        "sec-fetch-mode",
        &["cors", "navigate", "no-cors", "same-origin", "websocket"],
    ),
    (
        "sec-fetch-site",
        &["cross-site", "same-origin", "same-site", "none"],
    ),
    (
        "sec-fetch-dest",
        &["document", "empty", "image", "script", "style"],
    ),
    ("upgrade-insecure-requests", &["1"]),
    (
        "x-http-method-override",
        &["GET", "POST", "PUT", "PATCH", "DELETE"],
    ),
    ("x-forwarded-proto", &["https", "http"]),
    (
        "prefer",
        &["return=minimal", "return=representation", "respond-async"],
    ),
    (
        "access-control-request-method",
        &["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"],
    ),
];

/// The request header names offered, in canonical casing, most used first.
pub fn header_names() -> &'static [&'static str] {
    NAMES
}

/// The common values of the header `name` (compared without case), most used first; empty
/// for a header with no small vocabulary.
pub fn header_values(name: &str) -> &'static [&'static str] {
    VALUES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map_or(&[], |(_, values)| values)
}
