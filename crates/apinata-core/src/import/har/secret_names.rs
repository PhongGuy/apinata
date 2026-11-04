//! Names whose captured values are credentials. A name matches a whole list entry, ignoring
//! ASCII case and treating `-`, ` `, `.` and `_` as equal, so lookalikes such as `x-api-key-id`
//! do not. Field and query names match by their `leaf`, as nested-parameter parsers store them.

/// Vendor API-key and token headers; `Authorization`, `Proxy-Authorization` and `Cookie` have
/// their own pass.
pub(super) const HEADERS: [&str; 11] = [
    "x-api-key",
    "api-key",
    "x-auth-token",
    "x-access-token",
    "private-token",
    "x-csrf-token",
    "x-xsrf-token",
    "ocp-apim-subscription-key",
    "x-amz-security-token",
    "x-goog-api-key",
    "x-vault-token",
];

/// Whether `name` is a credential header from `HEADERS`.
pub(super) fn is_header(name: &str) -> bool {
    matches_any(name, &HEADERS)
}

/// Query, form, multipart and JSON field names whose values are credentials. Taken from
/// Cloudflare har-sanitizer (Apache-2.0) commit `bce43fac`, `defaultWordList`, without its
/// identifiers and vendor-specific fields, plus `api_key`, `apikey`, `secret` and `private_token`.
pub(super) const FIELDS: [&str; 14] = [
    "access_token",
    "api_key",
    "apikey",
    "assertion",
    "authenticity_token",
    "client_secret",
    "code_verifier",
    "id_token",
    "password",
    "private_token",
    "refresh_token",
    "samlresponse",
    "secret",
    "token",
];

/// Whether `name` is a credential form, multipart or JSON field from `FIELDS`.
pub(super) fn is_field(name: &str) -> bool {
    matches_any(leaf(name), &FIELDS)
}

/// Generic names taken as credentials in a query only, since they are common body keys, plus
/// the signed-URL parameters of botocore (`SigV4QueryAuth`, `HmacV1QueryAuth`, commit
/// `c6eb7bdf`) and Google Cloud Storage. Credential and key ids tie a capture to an account.
pub(super) const QUERY_ONLY: [&str; 10] = [
    "auth",
    "key",
    "sig",
    "signature",
    "x-amz-signature",
    "x-amz-security-token",
    "x-amz-credential",
    "awsaccesskeyid",
    "x-goog-signature",
    "x-goog-credential",
];

/// Whether `name` is a credential query parameter, from `FIELDS` or `QUERY_ONLY`.
pub(super) fn is_query(name: &str) -> bool {
    let name = leaf(name);
    matches_any(name, &FIELDS) || matches_any(name, &QUERY_ONLY)
}

/// The part of `name` a server's nested-parameter parser stores the value under: trailing `[]`
/// and all-digit `[0]` segments are list positions and dropped, then the last `[...]` segment is
/// the leaf. A name whose final bracket is not closed is its own leaf.
pub(super) fn leaf(name: &str) -> &str {
    let mut rest = name;
    while let Some(open) = rest.strip_suffix(']').and_then(|r| r.rfind('[')) {
        let inner = &rest[open + 1..rest.len() - 1];
        if !inner.bytes().all(|b| b.is_ascii_digit()) {
            return inner;
        }
        rest = &rest[..open];
    }
    rest
}

fn matches_any(name: &str, list: &[&str]) -> bool {
    let fold = |c: u8| {
        if matches!(c, b'-' | b' ' | b'.') {
            b'_'
        } else {
            c.to_ascii_lowercase()
        }
    };
    list.iter().any(|entry| {
        entry.len() == name.len()
            && entry
                .bytes()
                .zip(name.bytes())
                .all(|(a, b)| fold(a) == fold(b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_whole_ignoring_ascii_case_and_dash_versus_underscore() {
        assert!(is_header("X-API-KEY"));
        assert!(is_header("x_api_key"));
        assert!(!is_header("x-api-key-id"));
        assert!(!is_header("Authorization"));
        for name in ["x-api-key-secret", "api", "token", "csrf"] {
            assert!(!is_header(name), "{name}");
        }
    }

    #[test]
    fn query_names_take_fields_and_the_query_only_names() {
        assert!(is_query("key"));
        assert!(is_query("ACCESS_TOKEN"));
        assert!(is_query("Refresh-Token"));
        assert!(is_query("SAMLResponse"));
        for name in [
            "code",
            "state",
            "client_id",
            "email",
            "token_type",
            "username",
            "keyword",
        ] {
            assert!(!is_query(name), "{name}");
        }
    }

    #[test]
    fn fields_exclude_the_query_only_names() {
        assert!(is_field("Refresh-Token"));
        assert!(is_field("SAMLResponse"));
        for name in [
            "key",
            "auth",
            "sig",
            "signature",
            "code",
            "state",
            "client_id",
            "email",
        ] {
            assert!(!is_field(name), "{name}");
        }
    }

    #[test]
    fn a_bracketed_name_matches_by_its_last_segment() {
        assert_eq!(leaf("user[password]"), "password");
        assert_eq!(leaf("a[b][token]"), "token");
        assert_eq!(leaf("token[]"), "token");
        assert_eq!(leaf("token[0]"), "token");
        assert_eq!(leaf("user[password][]"), "password");
        assert_eq!(leaf("password"), "password");
        assert_eq!(leaf("user[password"), "user[password");
        assert!(is_field("client[secret]"));
        assert!(is_query("auth[token]"));
        assert!(!is_field("password[hint]"));
        assert!(!is_header("x[x-api-key]"));
    }

    #[test]
    fn spaces_and_dots_fold_like_underscores() {
        assert!(is_query("access token"));
        assert!(is_query("access.token"));
        assert!(is_field("client secret"));
        assert!(!is_query("access  token"));
    }

    #[test]
    fn signed_url_names_are_query_only() {
        for name in [
            "X-Amz-Signature",
            "X-Amz-Security-Token",
            "X-Amz-Credential",
            "AWSAccessKeyId",
            "X-Goog-Signature",
            "X-Goog-Credential",
        ] {
            assert!(is_query(name), "{name}");
            assert!(!is_field(name), "{name}");
        }
        for name in [
            "X-Amz-Date",
            "X-Amz-Expires",
            "X-Amz-Algorithm",
            "X-Amz-SignedHeaders",
            "X-Goog-Date",
            "Expires",
        ] {
            assert!(!is_query(name), "{name}");
        }
    }
}
