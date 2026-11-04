//! The browser half of the interactive grants: PKCE, state, the authorization URL and reading
//! the redirect that comes back, from the loopback listener or pasted by the user.

use super::OAuth2Error;
use super::client::scrub;
use super::config::TokenConfig;
use crate::edit::{OAuth2Flow, OAuth2ParamPlacement, OAuth2Stage};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::{TryRngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::fmt;
use url::Url;

/// An RFC 7636 code verifier and its S256 challenge.
pub(crate) struct Pkce {
    pub(crate) verifier: String,
    pub(crate) challenge: String,
}

impl Pkce {
    /// 32 OS-random bytes as unpadded base64url (43 characters) and its S256 challenge.
    pub(crate) fn generate() -> Result<Pkce, OAuth2Error> {
        let verifier = URL_SAFE_NO_PAD.encode(random::<32>()?);
        let challenge = Pkce::challenge_for(&verifier);
        Ok(Pkce {
            verifier,
            challenge,
        })
    }

    /// The S256 challenge: base64url of the verifier's SHA-256, unpadded.
    pub(crate) fn challenge_for(verifier: &str) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    }
}

fn random<const N: usize>() -> Result<[u8; N], OAuth2Error> {
    let mut bytes = [0u8; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| OAuth2Error::NoRandomness)?;
    Ok(bytes)
}

/// The user's state trimmed when non-blank, else 16 OS-random bytes as lowercase hex.
pub(crate) fn state_for(user: &str) -> Result<String, OAuth2Error> {
    match user.trim() {
        "" => Ok(random::<16>()?.iter().map(|b| format!("{b:02x}")).collect()),
        mine => Ok(mine.to_owned()),
    }
}

/// `MissingField` for an empty `authorizationUrl` or `clientId`; response_type `code`, or
/// `token` for Implicit (which never sends a challenge).
pub(crate) fn authorization_url(
    cfg: &TokenConfig,
    redirect_uri: &str,
    state: &str,
    challenge: Option<&str>,
) -> Result<String, OAuth2Error> {
    if cfg.authorization_url.is_empty() {
        return Err(OAuth2Error::MissingField {
            field: "authorizationUrl",
        });
    }
    if cfg.client_id.is_empty() {
        return Err(OAuth2Error::MissingField { field: "clientId" });
    }
    let implicit = cfg.flow == OAuth2Flow::Implicit;
    let mut url =
        Url::parse(&crate::resolve::with_scheme(cfg.authorization_url.clone())).map_err(|_| {
            OAuth2Error::Resolve(crate::ResolveError::InvalidUrl {
                url: "the authorization URL".to_owned(),
            })
        })?;
    // Parameter order follows the reference; only query-placed extras apply, since there is
    // no embedded window to carry headers or a body.
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", if implicit { "token" } else { "code" });
        query.append_pair("client_id", &cfg.client_id);
        if !redirect_uri.is_empty() {
            query.append_pair("redirect_uri", redirect_uri);
        }
        if !cfg.scope.is_empty() {
            query.append_pair("scope", &cfg.scope);
        }
        if let Some(challenge) = challenge.filter(|_| !implicit) {
            query.append_pair("code_challenge", challenge);
            query.append_pair("code_challenge_method", "S256");
        }
        if !state.is_empty() {
            query.append_pair("state", state);
        }
        for param in cfg.params.iter().filter(|p| {
            p.stage == OAuth2Stage::Authorization && p.placement == OAuth2ParamPlacement::Query
        }) {
            query.append_pair(&param.name, &param.value);
        }
    }
    Ok(url.into())
}

/// What a redirect handed back: an authorization code, or an implicit token's fields.
pub(crate) enum Redirect {
    Code(String),
    Token(serde_json::Map<String, serde_json::Value>),
}

impl fmt::Debug for Redirect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Redirect::Code(_) => f.write_str("Code([redacted])"),
            Redirect::Token(map) => f.debug_tuple("Token").field(&map.keys()).finish(),
        }
    }
}

fn first<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// A provider `error` becomes `AuthorizationDenied` with its text scrubbed of secrets.
fn denied(pairs: &[(String, String)], error: &str, secrets: &[String]) -> OAuth2Error {
    OAuth2Error::AuthorizationDenied {
        error: scrub(error, secrets),
        description: first(pairs, "error_description").map(|d| scrub(d, secrets)),
    }
}

fn check_state(pairs: &[(String, String)], state: &str) -> Result<(), OAuth2Error> {
    match first(pairs, "state") {
        Some(got) if got == state => Ok(()),
        _ => Err(OAuth2Error::StateMismatch),
    }
}

/// Loopback query pairs to the authorization code. State is checked before a provider
/// `error`, so a forged hit cannot put its text in front of the user.
pub(crate) fn read_callback(
    pairs: &[(String, String)],
    state: &str,
    secrets: &[String],
) -> Result<String, OAuth2Error> {
    let (code, error) = (first(pairs, "code"), first(pairs, "error"));
    if code.is_none() && error.is_none() {
        return Err(OAuth2Error::InvalidRedirect {
            reason: "no authorization code or error",
        });
    }
    check_state(pairs, state)?;
    if let Some(error) = error {
        return Err(denied(pairs, error, secrets));
    }
    let code = code.unwrap_or_default();
    if code.is_empty() {
        return Err(OAuth2Error::InvalidRedirect {
            reason: "empty authorization code",
        });
    }
    Ok(code.to_owned())
}

/// A pasted URL: the query for the code flow, the fragment for Implicit.
pub(crate) fn read_pasted(
    text: &str,
    state: &str,
    implicit: bool,
    secrets: &[String],
) -> Result<Redirect, OAuth2Error> {
    let url = parse_pasted(text)?;
    if !implicit {
        return read_callback(&query_pairs(&url), state, secrets).map(Redirect::Code);
    }
    let pairs = fragment_pairs(&url);
    let error = first(&pairs, "error");
    if error.is_none() && first(&pairs, "access_token").is_none_or(str::is_empty) {
        return Err(OAuth2Error::InvalidRedirect {
            reason: "no access token or error",
        });
    }
    check_state(&pairs, state)?;
    if let Some(error) = error {
        return Err(denied(&pairs, error, secrets));
    }
    let mut token = serde_json::Map::new();
    for (key, value) in pairs.into_iter().filter(|(k, _)| k != "state") {
        token.entry(key).or_insert(serde_json::Value::String(value));
    }
    Ok(Redirect::Token(token))
}

/// `Ok` when `pasted` is a URL whose query has a non-empty `code` or an `error`, or whose
/// fragment has a non-empty `access_token` or an `error`; else `InvalidRedirect`. For
/// shells that validate a paste.
pub fn check_redirect(pasted: &str) -> Result<(), OAuth2Error> {
    let url = parse_pasted(pasted)?;
    let has = |pairs: &[(String, String)], key: &str| {
        first(pairs, key).is_some_and(|v| key == "error" || !v.is_empty())
    };
    let (query, fragment) = (query_pairs(&url), fragment_pairs(&url));
    if has(&query, "code")
        || has(&query, "error")
        || has(&fragment, "access_token")
        || has(&fragment, "error")
    {
        Ok(())
    } else {
        Err(OAuth2Error::InvalidRedirect {
            reason: "no authorization code or error",
        })
    }
}

fn parse_pasted(text: &str) -> Result<Url, OAuth2Error> {
    Url::parse(text.trim()).map_err(|_| OAuth2Error::InvalidRedirect {
        reason: "not a URL",
    })
}

fn query_pairs(url: &Url) -> Vec<(String, String)> {
    url.query_pairs().into_owned().collect()
}

fn fragment_pairs(url: &Url) -> Vec<(String, String)> {
    url::form_urlencoded::parse(url.fragment().unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::effective_oauth2;
    use crate::{Collection, ContextOptions, ItemPath, VarContext};
    use tempfile::TempDir;

    const AC: &str = "    type: oauth2\n    flow: authorization_code\n    authorizationUrl: \"https://id.test/authorize?tenant=t1\"\n    accessTokenUrl: https://id.test/token\n    callbackUrl: https://app.test/cb\n    credentials:\n      clientId: app\n      clientSecret: cs\n    additionalParameters:\n      authorizationRequest:\n        - name: audience\n          value: https://api.test\n          placement: query\n        - name: X-Skip\n          value: \"1\"\n          placement: header\n    scope: read write\n    state: \" user-state \"\n    pkce: {}\n";

    fn config(auth: &str) -> TokenConfig {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("opencollection.yml"),
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("r.yml"),
            format!("info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://api.test/r\n  auth:\n{auth}"),
        )
        .unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
        let req = c.request(&ItemPath::new("r.yml").unwrap()).unwrap();
        TokenConfig::new(&effective_oauth2(req, &ctx).unwrap(), req, &ctx).unwrap()
    }
    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn pkce_uses_43_base64url_characters_and_s256() {
        let p = Pkce::generate().unwrap();
        assert_eq!(p.verifier.len(), 43);
        assert!(
            p.verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(p.challenge, Pkce::challenge_for(&p.verifier));
        assert_ne!(Pkce::generate().unwrap().verifier, p.verifier);
        // RFC 7636 appendix B.
        assert_eq!(
            Pkce::challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn state_is_the_users_trimmed_or_random_hex() {
        assert_eq!(state_for("  mine ").unwrap(), "mine");
        let random = state_for("   ").unwrap();
        assert_eq!(random.len(), 32);
        assert!(
            random
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(state_for("").unwrap(), random);
    }

    #[test]
    fn the_code_flow_url_follows_the_reference_order() {
        let cfg = config(AC);
        assert_eq!(cfg.state, " user-state ");
        assert!(cfg.pkce);
        let url = authorization_url(&cfg, "http://127.0.0.1:5/callback", "st", Some("CH")).unwrap();
        assert_eq!(
            url,
            "https://id.test/authorize?tenant=t1&response_type=code&client_id=app&redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcallback&scope=read+write&code_challenge=CH&code_challenge_method=S256&state=st&audience=https%3A%2F%2Fapi.test"
        );
        let plain = authorization_url(&cfg, "https://app.test/cb", "st", None).unwrap();
        assert!(!plain.contains("code_challenge"), "{plain}");
    }

    #[test]
    fn the_implicit_url_asks_for_a_token_and_never_a_challenge() {
        let cfg = config(
            "    type: oauth2\n    flow: implicit\n    authorizationUrl: https://id.test/authorize\n    callbackUrl: https://app.test/cb\n    credentials:\n      clientId: app\n    scope: read\n    pkce: {}\n",
        );
        assert!(!cfg.pkce);
        assert_eq!(cfg.callback_url, "https://app.test/cb");
        assert_eq!(
            authorization_url(&cfg, "https://app.test/cb", "st", None).unwrap(),
            "https://id.test/authorize?response_type=token&client_id=app&redirect_uri=https%3A%2F%2Fapp.test%2Fcb&scope=read&state=st"
        );
    }

    #[test]
    fn an_empty_authorization_url_is_missing_field() {
        let cfg = config(
            "    type: oauth2\n    flow: authorization_code\n    accessTokenUrl: https://id.test/token\n    credentials:\n      clientId: app\n",
        );
        assert!(matches!(
            authorization_url(&cfg, "http://127.0.0.1/callback", "s", None),
            Err(OAuth2Error::MissingField {
                field: "authorizationUrl"
            })
        ));
    }

    #[test]
    fn callbacks_check_presence_then_state_then_error() {
        let none: [String; 0] = [];
        assert_eq!(
            read_callback(&pairs(&[("code", "c0de"), ("state", "s")]), "s", &none).unwrap(),
            "c0de"
        );
        assert!(matches!(
            read_callback(&pairs(&[("code", "c0de"), ("state", "other")]), "s", &none),
            Err(OAuth2Error::StateMismatch)
        ));
        assert!(matches!(
            read_callback(&pairs(&[("code", "c0de")]), "s", &none),
            Err(OAuth2Error::StateMismatch)
        ));
        assert!(matches!(
            read_callback(&pairs(&[("state", "s")]), "s", &none),
            Err(OAuth2Error::InvalidRedirect {
                reason: "no authorization code or error"
            })
        ));
        assert!(matches!(
            read_callback(&pairs(&[("code", ""), ("state", "s")]), "s", &none),
            Err(OAuth2Error::InvalidRedirect {
                reason: "empty authorization code"
            })
        ));
        let secrets = vec!["cs-SECRET".to_owned()];
        let denied = read_callback(
            &pairs(&[
                ("error", "access_denied"),
                ("error_description", "client cs-SECRET refused"),
                ("state", "s"),
            ]),
            "s",
            &secrets,
        )
        .unwrap_err();
        match denied {
            OAuth2Error::AuthorizationDenied { error, description } => {
                assert_eq!(error, "access_denied");
                assert_eq!(description.as_deref(), Some("client [redacted] refused"));
            }
            other => panic!("expected AuthorizationDenied, got {other:?}"),
        }
        // A forged hit must pass the state check before its provider text is shown.
        for forged in [
            pairs(&[("error", "access_denied")]),
            pairs(&[("error", "access_denied"), ("state", "other")]),
        ] {
            assert!(matches!(
                read_callback(&forged, "s", &none),
                Err(OAuth2Error::StateMismatch)
            ));
        }
    }

    #[test]
    fn pasted_urls_carry_a_code_or_a_fragment_token() {
        let none: [String; 0] = [];
        assert!(matches!(
            read_pasted("https://app.test/cb?code=c0de&state=s", "s", false, &none),
            Ok(Redirect::Code(c)) if c == "c0de"
        ));
        match read_pasted(
            "https://app.test/cb#access_token=t0k&token_type=bearer&expires_in=3600&state=s",
            "s",
            true,
            &none,
        ) {
            Ok(Redirect::Token(map)) => {
                assert_eq!(
                    map.get("access_token").and_then(|v| v.as_str()),
                    Some("t0k")
                );
                assert_eq!(map.get("expires_in").and_then(|v| v.as_str()), Some("3600"));
                assert!(!map.contains_key("state"));
            }
            other => panic!("expected a token, got {other:?}"),
        }
        assert!(matches!(
            read_pasted(
                "https://app.test/cb#access_token=t0k&state=x",
                "s",
                true,
                &none
            ),
            Err(OAuth2Error::StateMismatch)
        ));
        assert!(matches!(
            read_pasted("https://app.test/cb?code=c0de&state=s", "s", true, &none),
            Err(OAuth2Error::InvalidRedirect {
                reason: "no access token or error"
            })
        ));
        for forged in [
            "https://app.test/cb#error=access_denied",
            "https://app.test/cb#error=access_denied&state=x",
            "https://app.test/cb?error=access_denied&state=x",
        ] {
            assert!(matches!(
                read_pasted(forged, "s", forged.contains('#'), &none),
                Err(OAuth2Error::StateMismatch)
            ));
        }
        assert!(matches!(
            read_pasted("nonsense", "s", false, &none),
            Err(OAuth2Error::InvalidRedirect {
                reason: "not a URL"
            })
        ));
    }

    #[test]
    fn check_redirect_accepts_what_the_readers_can_use() {
        assert!(check_redirect("https://app.test/cb?code=c&state=s").is_ok());
        assert!(check_redirect("https://app.test/cb?error=access_denied").is_ok());
        assert!(check_redirect("https://app.test/cb#access_token=t").is_ok());
        assert!(check_redirect(" https://app.test/cb#error=x ").is_ok());
        assert!(matches!(
            check_redirect("https://app.test/cb"),
            Err(OAuth2Error::InvalidRedirect { .. })
        ));
        for empty in [
            "https://app.test/cb?code=&state=s",
            "https://app.test/cb#access_token=&state=s",
        ] {
            assert!(matches!(
                check_redirect(empty),
                Err(OAuth2Error::InvalidRedirect { .. })
            ));
        }
        assert!(matches!(
            check_redirect("code=c"),
            Err(OAuth2Error::InvalidRedirect {
                reason: "not a URL"
            })
        ));
    }
}
