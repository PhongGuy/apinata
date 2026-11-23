//! Header names and common values offered while a request header is typed.
//!
//! The names are the request-relevant subset of the IANA message header registry, in
//! canonical casing; the values are the ones people type most for the headers that have a
//! small vocabulary. The descriptions are our own one-line wording, for request names and
//! the response headers people most often look up.

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

/// One-line descriptions, keyed by lowercase name: the offered request names, then the
/// response headers people most often look up.
const DESCRIPTIONS: &[(&str, &str)] = &[
    (
        "accept",
        "The response formats the client can read, best first",
    ),
    (
        "authorization",
        "Credentials that prove who is making the request",
    ),
    (
        "content-type",
        "The format of the body, such as JSON or a form",
    ),
    ("content-length", "The size of the body in bytes"),
    (
        "cache-control",
        "Rules for whether and how long the message may be cached",
    ),
    ("cookie", "Cookies the client sends back to the server"),
    (
        "user-agent",
        "The name and version of the program making the request",
    ),
    (
        "accept-encoding",
        "The compression schemes the client can unpack",
    ),
    (
        "accept-language",
        "The human languages the client prefers, best first",
    ),
    ("accept-charset", "The text encodings the client can read"),
    (
        "connection",
        "Whether to keep the connection open after this exchange",
    ),
    ("host", "The server name and port the request is meant for"),
    ("origin", "The site a browser request comes from"),
    ("referer", "The page that linked to or started this request"),
    (
        "if-match",
        "Act only if the resource still has one of these ETags",
    ),
    (
        "if-none-match",
        "Act only if the resource has none of these ETags; used to reuse a cached copy",
    ),
    (
        "if-modified-since",
        "Send the resource only if it changed after this date",
    ),
    (
        "if-unmodified-since",
        "Act only if the resource has not changed since this date",
    ),
    (
        "if-range",
        "Send only the asked range if the resource is unchanged, else all of it",
    ),
    ("range", "Ask for only part of the resource, by byte range"),
    (
        "content-encoding",
        "The compression applied to the body, such as gzip",
    ),
    (
        "content-language",
        "The human language the body is written for",
    ),
    (
        "content-location",
        "Where the body's content can be fetched on its own",
    ),
    (
        "content-md5",
        "An MD5 checksum of the body; outdated and rarely checked",
    ),
    (
        "content-disposition",
        "Whether the body shows inline or downloads, and its file name",
    ),
    ("date", "When the message was created"),
    (
        "etag",
        "A version tag for the resource, changed whenever it changes",
    ),
    (
        "expect",
        "Asks the server to confirm before the body is sent",
    ),
    (
        "forwarded",
        "The client and proxies a request passed through",
    ),
    ("from", "An email address for the person behind the request"),
    (
        "max-forwards",
        "How many more proxies may pass the request on",
    ),
    (
        "pragma",
        "An old way to say no-cache; Cache-Control replaces it",
    ),
    (
        "prefer",
        "Optional server behaviour the client would like, such as a short reply",
    ),
    (
        "proxy-authorization",
        "Credentials for the proxy between client and server",
    ),
    (
        "te",
        "The transfer codings the client accepts, and whether it takes trailers",
    ),
    (
        "trailer",
        "Headers that follow after the body instead of before it",
    ),
    (
        "transfer-encoding",
        "How the body is framed in transit, such as in chunks",
    ),
    (
        "upgrade",
        "Asks to switch to another protocol, such as WebSocket",
    ),
    ("via", "The proxies the message passed through"),
    (
        "warning",
        "Extra notes about the message's state; now outdated",
    ),
    (
        "access-control-request-method",
        "In a preflight, the method the real request will use",
    ),
    (
        "access-control-request-headers",
        "In a preflight, the headers the real request will send",
    ),
    ("dnt", "Asks not to be tracked; widely ignored"),
    (
        "sec-fetch-dest",
        "What a browser will use the response for, such as an image",
    ),
    (
        "sec-fetch-mode",
        "How a browser makes the request, such as cors or navigate",
    ),
    (
        "sec-fetch-site",
        "How the requesting site relates to the target site",
    ),
    (
        "upgrade-insecure-requests",
        "Says the client prefers to be sent to HTTPS",
    ),
    (
        "x-api-key",
        "An API key that identifies the caller; a common custom header",
    ),
    (
        "x-forwarded-for",
        "The client address a proxy received the request from",
    ),
    (
        "x-forwarded-host",
        "The host the client first asked a proxy for",
    ),
    (
        "x-forwarded-proto",
        "Whether the client reached the proxy over HTTP or HTTPS",
    ),
    (
        "x-requested-with",
        "Marks a request made by script, often XMLHttpRequest",
    ),
    (
        "x-request-id",
        "An id for this one request, to find it in logs",
    ),
    (
        "x-correlation-id",
        "An id shared by all requests in one operation, to trace it across services",
    ),
    (
        "x-http-method-override",
        "The method the server should use instead of the one sent",
    ),
    ("age", "How many seconds the response has sat in a cache"),
    ("allow", "The methods the resource supports"),
    ("expires", "When the response stops being fresh in a cache"),
    ("last-modified", "When the resource last changed"),
    (
        "location",
        "Where to go next: the redirect target or the new resource",
    ),
    ("retry-after", "How long to wait before trying again"),
    ("server", "The software that handled the request"),
    (
        "set-cookie",
        "A cookie for the client to store and send back later",
    ),
    (
        "vary",
        "The request headers that change which response comes back",
    ),
    (
        "www-authenticate",
        "How to authenticate, sent with a 401 response",
    ),
    (
        "proxy-authenticate",
        "How to authenticate with the proxy, sent with a 407 response",
    ),
    (
        "accept-ranges",
        "Whether the server can send part of the resource, and in what unit",
    ),
    (
        "content-range",
        "Which part of the resource this body holds",
    ),
    (
        "content-security-policy",
        "Which sources the page may load scripts, styles and more from",
    ),
    (
        "strict-transport-security",
        "Tells browsers to use only HTTPS for this site for a while",
    ),
    (
        "x-content-type-options",
        "With nosniff, browsers must trust Content-Type and not guess",
    ),
    (
        "x-frame-options",
        "Whether other sites may show this page in a frame",
    ),
    (
        "referrer-policy",
        "How much of the page address to send as Referer",
    ),
    (
        "link",
        "Links to related resources, such as the next page of results",
    ),
    (
        "keep-alive",
        "How long an open connection may idle, and how many requests it serves",
    ),
    (
        "alt-svc",
        "Other places or protocols where the same service can be reached",
    ),
    (
        "access-control-allow-origin",
        "Which sites a browser lets read this response",
    ),
    (
        "access-control-allow-methods",
        "In a preflight reply, the methods the real request may use",
    ),
    (
        "access-control-allow-headers",
        "In a preflight reply, the headers the real request may send",
    ),
    (
        "access-control-allow-credentials",
        "Whether a browser may share the response when cookies were sent",
    ),
    (
        "access-control-expose-headers",
        "Response headers a browser lets scripts read",
    ),
    (
        "access-control-max-age",
        "How many seconds a browser may reuse a preflight reply",
    ),
    (
        "x-ratelimit-limit",
        "How many requests are allowed in the current window",
    ),
    (
        "x-ratelimit-remaining",
        "How many requests are left in the current window",
    ),
    (
        "x-ratelimit-reset",
        "When the request limit window starts over",
    ),
    (
        "x-powered-by",
        "The framework or platform behind the server",
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

/// A one-line, plain description of the header `name` (compared without case), for a hint
/// beside the name; `None` for a header this list does not describe.
pub fn header_description(name: &str) -> Option<&'static str> {
    DESCRIPTIONS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, text)| *text)
}
