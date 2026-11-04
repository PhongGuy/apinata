//! HTTP digest authentication (RFC 7616): reading a server's challenge and answering it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DigestAlgorithm {
    Md5,
    Sha256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub(crate) realm: String,
    pub(crate) nonce: String,
    pub(crate) opaque: Option<String>,
    /// The server offered `qop=auth`.
    pub(crate) qop_auth: bool,
    pub(crate) algorithm: DigestAlgorithm,
}

/// The first `Digest` challenge among `WWW-Authenticate` values. `None` when there is none or
/// that first one is unusable: it lacks realm or nonce, its algorithm is not MD5 or SHA-256 (case-insensitive,
/// absent means MD5), or it offers qop without `auth`.
pub(crate) fn parse_challenge<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<Challenge> {
    let params = values.into_iter().find_map(digest_params)?;
    let param = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let algorithm = match param("algorithm") {
        None => DigestAlgorithm::Md5,
        Some(a) if a.eq_ignore_ascii_case("MD5") => DigestAlgorithm::Md5,
        Some(a) if a.eq_ignore_ascii_case("SHA-256") => DigestAlgorithm::Sha256,
        Some(_) => return None,
    };
    // An offered qop must include plain `auth`; `auth-int` alone is unsupported.
    let qop_auth = match param("qop") {
        None => false,
        Some(q) if q.split(',').any(|o| o.trim().eq_ignore_ascii_case("auth")) => true,
        Some(_) => return None,
    };
    Some(Challenge {
        realm: param("realm")?.to_owned(),
        nonce: param("nonce")?.to_owned(),
        opaque: param("opaque").map(str::to_owned),
        qop_auth,
        algorithm,
    })
}

/// The auth-params of one header value when its scheme is `Digest`, as RFC 7235 reads them:
/// `token=token` or `token="quoted string"` with `\` escapes, separated by commas.
fn digest_params(value: &str) -> Option<Vec<(String, String)>> {
    let value = value.trim_start();
    let scheme_end = value
        .find(|c: char| c.is_ascii_whitespace())
        .unwrap_or(value.len());
    if !value[..scheme_end].eq_ignore_ascii_case("digest") {
        return None;
    }
    let mut chars = value[scheme_end..].chars().peekable();
    let mut params = Vec::new();
    loop {
        while chars
            .next_if(|c| c.is_ascii_whitespace() || *c == ',')
            .is_some()
        {}
        let name: String = std::iter::from_fn(|| {
            chars.next_if(|c| *c != '=' && !c.is_ascii_whitespace() && *c != ',')
        })
        .collect();
        while chars.next_if(char::is_ascii_whitespace).is_some() {}
        if name.is_empty() || chars.next() != Some('=') {
            return Some(params);
        }
        while chars.next_if(char::is_ascii_whitespace).is_some() {}
        let mut val = String::new();
        if chars.next_if_eq(&'"').is_some() {
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => val.extend(chars.next()),
                    c => val.push(c),
                }
            }
        } else {
            val.extend(std::iter::from_fn(|| {
                chars.next_if(|c| *c != ',' && !c.is_ascii_whitespace())
            }));
        }
        params.push((name, val));
    }
}

/// The `Authorization` value, fields in the reference's order.
pub(crate) fn authorization(
    c: &Challenge,
    username: &str,
    password: &str,
    method: &str,
    uri: &str,
    cnonce: &str,
) -> String {
    let h = |input: String| hash(c.algorithm, &input);
    let ha1 = h(format!("{username}:{}:{password}", c.realm));
    let ha2 = h(format!("{method}:{uri}"));
    let name = match c.algorithm {
        DigestAlgorithm::Md5 => "MD5",
        DigestAlgorithm::Sha256 => "SHA-256",
    };
    let mut header;
    if c.qop_auth {
        let response = h(format!("{ha1}:{}:{NC}:{cnonce}:auth:{ha2}", c.nonce));
        header = format!(
            r#"Digest username="{username}", realm="{}", nonce="{}", uri="{uri}", response="{response}", qop="auth", algorithm="{name}", nc="{NC}", cnonce="{cnonce}""#,
            c.realm, c.nonce
        );
    } else {
        let response = h(format!("{ha1}:{}:{ha2}", c.nonce));
        header = format!(
            r#"Digest username="{username}", realm="{}", nonce="{}", uri="{uri}", response="{response}""#,
            c.realm, c.nonce
        );
        // The legacy form names the algorithm only when it is not the implied MD5.
        if c.algorithm != DigestAlgorithm::Md5 {
            header.push_str(&format!(r#", algorithm="{name}""#));
        }
    }
    if let Some(opaque) = &c.opaque {
        header.push_str(&format!(r#", opaque="{opaque}""#));
    }
    header
}

/// The nonce count: each challenge is answered exactly once.
const NC: &str = "00000001";

/// Lowercase hex of the algorithm's digest of `input`; the digests have no `LowerHex`.
fn hash(algorithm: DigestAlgorithm, input: &str) -> String {
    use sha2::Digest as _;
    let bytes: Vec<u8> = match algorithm {
        DigestAlgorithm::Md5 => md5::Md5::digest(input).to_vec(),
        DigestAlgorithm::Sha256 => sha2::Sha256::digest(input).to_vec(),
    };
    hex(&bytes)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 24 random bytes as 48 lowercase hex characters; `None` when the OS generator fails.
pub(crate) fn new_cnonce() -> Option<String> {
    use rand::{TryRngCore, rngs::OsRng};
    let mut bytes = [0u8; 24];
    OsRng.try_fill_bytes(&mut bytes).ok()?;
    Some(hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC2617: &str = r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#;
    const RFC7616: &str = r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=SHA-256, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#;
    const CNONCE_7616: &str = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";

    #[test]
    fn rfc2617_qop_auth_md5() {
        let c = parse_challenge([RFC2617]).unwrap();
        assert_eq!(
            c,
            Challenge {
                realm: "testrealm@host.com".into(),
                nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093".into(),
                opaque: Some("5ccc069c403ebaf9f0171e9517f40e41".into()),
                qop_auth: true,
                algorithm: DigestAlgorithm::Md5,
            }
        );
        assert_eq!(
            authorization(
                &c,
                "Mufasa",
                "Circle Of Life",
                "GET",
                "/dir/index.html",
                "0a4f113b"
            ),
            r#"Digest username="Mufasa", realm="testrealm@host.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", uri="/dir/index.html", response="6629fae49393a05397450978507c4ef1", qop="auth", algorithm="MD5", nc="00000001", cnonce="0a4f113b", opaque="5ccc069c403ebaf9f0171e9517f40e41""#
        );
    }

    #[test]
    fn rfc7616_sha256_and_md5() {
        let c = parse_challenge([RFC7616]).unwrap();
        assert_eq!(c.algorithm, DigestAlgorithm::Sha256);
        let h = authorization(
            &c,
            "Mufasa",
            "Circle of Life",
            "GET",
            "/dir/index.html",
            CNONCE_7616,
        );
        assert!(
            h.contains(
                r#"response="753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1""#
            ),
            "{h}"
        );
        assert!(h.contains(r#"algorithm="SHA-256""#), "{h}");
        let md5 = Challenge {
            algorithm: DigestAlgorithm::Md5,
            ..c
        };
        let h = authorization(
            &md5,
            "Mufasa",
            "Circle of Life",
            "GET",
            "/dir/index.html",
            CNONCE_7616,
        );
        assert!(
            h.contains(r#"response="8ca523f5e9506fed4657c9700eebdbec""#),
            "{h}"
        );
    }

    #[test]
    fn without_qop_the_legacy_form_is_sent() {
        let c = parse_challenge([
            r#"Digest realm="testrealm@host.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093""#,
        ])
        .unwrap();
        assert!(!c.qop_auth);
        assert_eq!(
            authorization(
                &c,
                "Mufasa",
                "Circle Of Life",
                "GET",
                "/dir/index.html",
                "unused"
            ),
            r#"Digest username="Mufasa", realm="testrealm@host.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", uri="/dir/index.html", response="670fd8c2df070c60b045671b8b24ff02""#
        );
        let c = parse_challenge([r#"digest realm="testrealm@host.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", algorithm="sha-256""#]).unwrap();
        assert_eq!(
            authorization(
                &c,
                "Mufasa",
                "Circle Of Life",
                "GET",
                "/dir/index.html",
                "unused"
            ),
            r#"Digest username="Mufasa", realm="testrealm@host.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", uri="/dir/index.html", response="e71f89d8267982ee1cd4dfb3637698eaf2f55848fe056aee7be175262aab5d2a", algorithm="SHA-256""#
        );
    }

    #[test]
    fn unusable_challenges_are_refused() {
        for value in [
            r#"Basic realm="x""#,
            r#"Digest realm="r""#,
            r#"Digest nonce="n""#,
            r#"Digest realm="r", nonce="n", algorithm=MD5-sess"#,
            r#"Digest realm="r", nonce="n", algorithm=SHA-512-256"#,
            r#"Digest realm="r", nonce="n", qop="auth-int""#,
        ] {
            assert_eq!(parse_challenge([value]), None, "{value}");
        }
        assert_eq!(parse_challenge([]), None);
    }

    #[test]
    fn the_first_digest_value_is_used_and_quoted_commas_survive() {
        let c = parse_challenge([
            r#"Basic realm="b""#,
            r#"Digest realm="a, b", nonce="n1""#,
            r#"Digest realm="z", nonce="n2""#,
        ])
        .unwrap();
        assert_eq!((c.realm.as_str(), c.nonce.as_str()), ("a, b", "n1"));
    }

    #[test]
    fn cnonces_are_48_hex_and_fresh() {
        let (a, b) = (new_cnonce().unwrap(), new_cnonce().unwrap());
        assert_eq!(a.len(), 48);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(a, b);
    }
}
