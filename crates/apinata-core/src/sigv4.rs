//! AWS Signature Version 4 for one request hop, matching the `aws4` package's choices for
//! scope inference and s3 path canonicalisation, with the signing itself left to `aws-sigv4`.

use std::fmt;
use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    PayloadChecksumKind, PercentEncodingMode, SignableBody, SignableRequest, SigningSettings,
    UriPathNormalizationMode,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, percent_encode};

/// Resolved SigV4 settings; `session_token` empty means none.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SigV4 {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: String,
    pub(crate) session_token: String,
    pub(crate) service: String,
    pub(crate) region: String,
}

// The keys never reach logs or error text, so Debug names only the scope.
impl fmt::Debug for SigV4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigV4")
            .field("service", &self.service)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

/// RFC 3986 unreserved characters stay; everything else is percent-encoded.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// `(service, region)`: the given values, with blanks filled from `host` by aws4 1.13.2's
/// `matchHost` rule; region falls back to `us-east-1`; service `email` becomes `ses`.
pub(crate) fn infer_scope(host: &str, service: &str, region: &str) -> (String, String) {
    let (host_service, host_region) = match_host(host);
    let service = if service.is_empty() {
        host_service.unwrap_or_default()
    } else {
        service.to_owned()
    };
    let region = match region {
        "" => host_region
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "us-east-1".to_owned()),
        given => given.to_owned(),
    };
    let service = if service == "email" {
        "ses".to_owned()
    } else {
        service
    };
    (service, region)
}

/// aws4's `([^.]{1,63})\.(?:([^.]{0,63})\.)?amazonaws\.com(\.cn)?$` without a regex: the
/// leftmost match takes the two labels before the suffix when both fit, else the last one.
/// Then es/aoss hosts swap their parts and s3 hosts name the bucket's region.
fn match_host(host: &str) -> (Option<String>, Option<String>) {
    let Some(rest) = host
        .strip_suffix(".amazonaws.com.cn")
        .or_else(|| host.strip_suffix(".amazonaws.com"))
    else {
        return (None, None);
    };
    let labels: Vec<&str> = rest.split('.').collect();
    let last = labels[labels.len() - 1];
    // A group-one label longer than 63 matches from its last 63 characters.
    let tail = |label: &str| {
        let skip = label.chars().count().saturating_sub(63);
        label.chars().skip(skip).collect::<String>()
    };
    let pair = match labels.len().checked_sub(2).map(|i| labels[i]) {
        Some(first) if !first.is_empty() && last.chars().count() <= 63 => {
            Some((tail(first), Some(last.to_owned())))
        }
        _ if !last.is_empty() => Some((tail(last), None)),
        _ => None,
    };
    let Some((mut first, mut second)) = pair else {
        return (None, None);
    };
    if matches!(second.as_deref(), Some("es" | "aoss")) {
        let swapped = second.take().unwrap_or_default();
        second = Some(std::mem::replace(&mut first, swapped));
    }
    if second.as_deref() == Some("s3") {
        return (Some("s3".to_owned()), Some("us-east-1".to_owned()));
    }
    for part in [Some(first.as_str()), second.as_deref()]
        .into_iter()
        .flatten()
    {
        if let Some(bucket_region) = part.strip_prefix("s3-") {
            return (Some("s3".to_owned()), Some(bucket_region.to_owned()));
        }
    }
    (Some(first), second)
}

/// The s3 canonical path: each segment percent-decoded (`+` as space), re-encoded keeping only
/// RFC 3986 unreserved characters, `%2F` restored to `/`.
pub(crate) fn s3_canonical_path(path: &str) -> String {
    // Decoding works on bytes, so a malformed escape stays literal and its `%` encodes as `%25`.
    path.split('/')
        .map(|segment| {
            let decoded: Vec<u8> = percent_decode_str(&segment.replace('+', " ")).collect();
            percent_encode(&decoded, UNRESERVED)
                .to_string()
                .replace("%2F", "/")
        })
        .collect::<Vec<_>>()
        .join("/")
}

impl SigV4 {
    /// Headers to add to the hop, lowercase names, in the order aws-sigv4 returns them:
    /// `x-amz-date`, `authorization`, then `x-amz-security-token` and, for s3,
    /// `x-amz-content-sha256` when they apply. `headers` are the hop's own headers; `host`
    /// is signed from `url` when absent. Errors carry the crate's message, never a key.
    pub(crate) fn sign(
        &self,
        method: &str,
        url: &url::Url,
        headers: &[(String, String)],
        body: &[u8],
        time: SystemTime,
    ) -> Result<Vec<(String, String)>, String> {
        let token = (!self.session_token.is_empty()).then(|| self.session_token.clone());
        let identity = Credentials::new(
            &self.access_key_id,
            &self.secret_access_key,
            token,
            None,
            "apinata",
        )
        .into();

        // s3 signs its own canonical path once, as aws4 does; the wire URL is left alone.
        let mut signed_url = url.clone();
        signed_url.set_fragment(None);
        let mut settings = SigningSettings::default();
        if self.service == "s3" {
            settings.percent_encoding_mode = PercentEncodingMode::Single;
            settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
            settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
            signed_url.set_path(&s3_canonical_path(url.path()));
        }

        let params = aws_sigv4::sign::v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name(&self.service)
            .time(time)
            .settings(settings)
            .build()
            .map_err(|e| e.to_string())?
            .into();
        let lowered: Vec<(String, &str)> = headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.as_str()))
            .collect();
        let request = SignableRequest::new(
            method,
            signed_url.as_str(),
            lowered.iter().map(|(name, value)| (name.as_str(), *value)),
            SignableBody::Bytes(body),
        )
        .map_err(|e| e.to_string())?;
        let (instructions, _signature) = aws_sigv4::http_request::sign(request, &params)
            .map_err(|e| e.to_string())?
            .into_parts();
        Ok(instructions
            .headers()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    const T: u64 = 1_440_938_160; // 20150830T123600Z

    fn keys(service: &str, region: &str, token: &str) -> SigV4 {
        SigV4 {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: token.into(),
            service: service.into(),
            region: region.into(),
        }
    }

    fn signed(
        s: &SigV4,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Vec<(String, String)> {
        let url = url::Url::parse(url).unwrap();
        let headers: Vec<(String, String)> = headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect();
        s.sign(
            method,
            &url,
            &headers,
            body,
            UNIX_EPOCH + Duration::from_secs(T),
        )
        .unwrap()
    }

    fn get<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
        h.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn get_vanilla_matches_the_aws_suite() {
        let h = signed(
            &keys("service", "us-east-1", ""),
            "GET",
            "https://example.amazonaws.com/",
            &[],
            b"",
        );
        assert_eq!(get(&h, "x-amz-date"), Some("20150830T123600Z"));
        assert_eq!(
            get(&h, "authorization"),
            Some(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
            )
        );
        assert_eq!(get(&h, "x-amz-security-token"), None);
        assert_eq!(get(&h, "x-amz-content-sha256"), None);
    }

    #[test]
    fn a_session_token_is_sent_and_signed() {
        let h = signed(
            &keys("execute-api", "eu-west-1", "TOKEN123"),
            "GET",
            "https://example.amazonaws.com/items?b=2&a=1",
            &[],
            b"",
        );
        assert_eq!(get(&h, "x-amz-security-token"), Some("TOKEN123"));
        assert_eq!(
            get(&h, "authorization"),
            Some(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/eu-west-1/execute-api/aws4_request, SignedHeaders=host;x-amz-date;x-amz-security-token, Signature=f3b7cd0fdc35e90e2cde08d1596d0a3fc2c71a10c13a90994c86efa7e1cfc843"
            )
        );
    }

    #[test]
    fn own_headers_and_the_body_are_signed_with_a_port_in_the_host() {
        let h = signed(
            &keys("execute-api", "us-east-1", ""),
            "POST",
            "http://127.0.0.1:8080/v1/things",
            &[("Content-Type", "application/json")],
            br#"{"a":1}"#,
        );
        assert_eq!(
            get(&h, "authorization"),
            Some(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/execute-api/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=b0f058709886e91739cf9f0df5d39decb4960e72a7db9d1d4ae25a812623fbe7"
            )
        );
    }

    #[test]
    fn s3_sends_the_payload_hash() {
        let h = signed(
            &keys("s3", "us-east-1", ""),
            "PUT",
            "https://bucket.s3.amazonaws.com/key.txt",
            &[],
            b"hello",
        );
        assert_eq!(
            get(&h, "x-amz-content-sha256"),
            Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
        );
        assert_eq!(
            get(&h, "authorization"),
            Some(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=459a1c67894345261389d48e1d28640ada6d4947b96e34bbadc24a5689e17535"
            )
        );
    }

    #[test]
    fn s3_paths_are_canonicalised_as_aws4_does() {
        assert_eq!(
            s3_canonical_path("/photos/file%20(1)!.pdf"),
            "/photos/file%20%281%29%21.pdf"
        );
        assert_eq!(s3_canonical_path("/a+b/c%2Fd"), "/a%20b/c/d");
        assert_eq!(s3_canonical_path("/"), "/");
        // A malformed escape cannot be decoded, so its `%` is encoded literally.
        assert_eq!(s3_canonical_path("/a%zz"), "/a%25zz");
        let h = signed(
            &keys("s3", "us-east-1", ""),
            "GET",
            "https://bucket.s3.amazonaws.com/photos/file (1)!.pdf",
            &[],
            b"",
        );
        assert!(get(&h, "authorization").unwrap().ends_with(
            "Signature=24f4d51cdf3492f2a8617ddfe69831d0fde34c7f3b565cec2f7fa49a951187e5"
        ));
    }

    #[test]
    fn scope_is_inferred_from_aws_hosts() {
        let s = |host: &str, service: &str, region: &str| {
            let (a, b) = infer_scope(host, service, region);
            (a, b)
        };
        let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
        assert_eq!(
            s("dynamodb.us-west-2.amazonaws.com", "", ""),
            pair("dynamodb", "us-west-2")
        );
        assert_eq!(s("sts.amazonaws.com", "", ""), pair("sts", "us-east-1"));
        assert_eq!(
            s("search-x.eu-west-1.es.amazonaws.com", "", ""),
            pair("es", "eu-west-1")
        );
        assert_eq!(
            s("bucket.s3.amazonaws.com", "", ""),
            pair("s3", "us-east-1")
        );
        assert_eq!(
            s("bucket.s3-eu-west-1.amazonaws.com", "", ""),
            pair("s3", "eu-west-1")
        );
        assert_eq!(
            s("x.cn-north-1.amazonaws.com.cn", "", ""),
            pair("x", "cn-north-1")
        );
        assert_eq!(
            s("email.us-east-1.amazonaws.com", "", ""),
            pair("ses", "us-east-1")
        );
        assert_eq!(s("api.example.com", "", ""), pair("", "us-east-1"));
        assert_eq!(
            s("dynamodb.us-west-2.amazonaws.com", "execute-api", ""),
            pair("execute-api", "us-west-2")
        );
        assert_eq!(
            s("api.example.com", "execute-api", "eu-north-1"),
            pair("execute-api", "eu-north-1")
        );
    }

    #[test]
    fn debug_hides_the_keys() {
        let text = format!("{:?}", keys("svc", "r1", "TOKEN123"));
        assert!(
            !text.contains("AKIDEXAMPLE")
                && !text.contains("EXAMPLEKEY")
                && !text.contains("TOKEN123"),
            "{text}"
        );
        assert!(text.contains("svc") && text.contains("r1"), "{text}");
    }
}
