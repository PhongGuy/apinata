//! `AuthSpec` to a Postman v2.1 auth object.

use serde_json::{Value, json};

use crate::edit::edgegrid::MAX_EXACT;
use crate::edit::{
    ApiKeyPlacement, AuthSpec, ClientAuth, EdgeGridSpec, OAuth1KeyKind, OAuth1Placement,
    OAuth1Spec, OAuth2Flow, OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage, TokenPlacement,
};

/// The Postman auth object for a mode it can carry; `None` for `None`, `Inherit` and
/// `Unsupported`, which each level places or omits itself.
pub(super) fn auth(spec: &AuthSpec) -> Option<Value> {
    let (kind, entries) = match spec {
        AuthSpec::Basic { username, password } => (
            "basic",
            strings(&[("password", password), ("username", username)]),
        ),
        AuthSpec::Bearer { token } => ("bearer", strings(&[("token", token)])),
        AuthSpec::ApiKey {
            key,
            value,
            placement,
        } => {
            let at = match placement {
                ApiKeyPlacement::Query => "query",
                _ => "header",
            };
            let entries = [("key", key.as_str()), ("value", value), ("in", at)];
            ("apikey", strings(&entries))
        }
        AuthSpec::Digest { username, password } => (
            "digest",
            strings(&[("password", password), ("username", username)]),
        ),
        AuthSpec::Ntlm {
            username,
            password,
            domain,
        } => (
            "ntlm",
            strings(&[
                ("domain", domain),
                ("password", password),
                ("username", username),
            ]),
        ),
        AuthSpec::AwsV4 {
            access_key_id,
            secret_access_key,
            session_token,
            service,
            region,
            ..
        } => {
            let entries = [
                ("sessionToken", session_token.as_str()),
                ("service", service),
                ("region", region),
                ("secretKey", secret_access_key),
                ("accessKey", access_key_id),
            ];
            ("awsv4", strings(&entries))
        }
        AuthSpec::OAuth1(o) => ("oauth1", oauth1(o)),
        AuthSpec::OAuth2(o) => ("oauth2", oauth2(o)),
        AuthSpec::EdgeGrid(e) => ("edgegrid", edgegrid(e)),
        _ => return None,
    };
    Some(json!({"type": kind, kind: entries}))
}

fn strings(entries: &[(&str, &str)]) -> Vec<Value> {
    entries
        .iter()
        .map(|(key, value)| json!({"key": key, "value": value, "type": "string"}))
        .collect()
}

fn booleans(entries: &[(&str, bool)]) -> Vec<Value> {
    entries
        .iter()
        .map(|(key, value)| json!({"key": key, "value": value, "type": "boolean"}))
        .collect()
}

/// The reference's edgegrid list: seven strings, then `maxBodySize` as the number (an integer
/// when exactly one) or `""` when absent, still typed `string` as the reference writes it.
fn edgegrid(e: &EdgeGridSpec) -> Vec<Value> {
    let mut out = strings(&[
        ("accessToken", &e.access_token),
        ("clientToken", &e.client_token),
        ("clientSecret", &e.client_secret),
        ("baseURL", &e.base_url),
        ("nonce", &e.nonce),
        ("timestamp", &e.timestamp),
        ("headersToSign", &e.headers_to_sign),
    ]);
    let max = match e.max_body_size {
        Some(n) if n.fract() == 0.0 && n.abs() <= MAX_EXACT => json!(n as i64),
        Some(n) => json!(n),
        None => json!(""),
    };
    out.push(json!({"key": "maxBodySize", "value": max, "type": "string"}));
    out
}

/// The reference's oauth1 list: every string field with absent ones as `""`, the private key
/// only for an `RSA-` method name (blank for a file key), then the two flags.
fn oauth1<'a>(o: &'a OAuth1Spec) -> Vec<Value> {
    let text = |v: &Option<String>| v.clone().unwrap_or_default();
    let or = |v: &'a str, default: &'a str| if v.is_empty() { default } else { v };
    let method = or(o.signature_method.as_str(), "HMAC-SHA1");
    let mut out = strings(&[
        ("consumerKey", &text(&o.consumer_key)),
        ("consumerSecret", &text(&o.consumer_secret)),
        ("token", &text(&o.access_token)),
        ("tokenSecret", &text(&o.access_token_secret)),
        ("signatureMethod", method),
    ]);
    if method.starts_with("RSA-") {
        let key = match &o.private_key {
            Some(k) if k.kind != OAuth1KeyKind::File => k.value.as_str(),
            _ => "",
        };
        out.extend(strings(&[("privateKey", key)]));
    }
    out.extend(strings(&[
        ("callback", &text(&o.callback_url)),
        ("verifier", &text(&o.verifier)),
        ("timestamp", &text(&o.timestamp)),
        ("nonce", &text(&o.nonce)),
        ("version", or(&o.version, "1.0")),
        ("realm", &text(&o.realm)),
    ]));
    out.extend(booleans(&[
        ("addParamsToHeader", o.placement != OAuth1Placement::Query),
        ("includeBodyHash", o.include_body_hash),
    ]));
    out
}

/// The reference's oauth2 list: every field in its order with empty values dropped, then one
/// `*RequestParams` entry per stage that has extra parameters.
fn oauth2(o: &OAuth2Spec) -> Vec<Value> {
    let grant = match o.flow {
        OAuth2Flow::AuthorizationCode if o.pkce => "authorization_code_with_pkce",
        OAuth2Flow::AuthorizationCode => "authorization_code",
        OAuth2Flow::Password => "password_credentials",
        OAuth2Flow::Implicit => "implicit",
        _ => "client_credentials",
    };
    let (add_to, prefix) = match &o.token_placement {
        TokenPlacement::Query { .. } => ("queryParams", ""),
        TokenPlacement::Header { prefix } => ("header", prefix.as_str()),
        _ => ("header", ""),
    };
    let client_auth = match o.client_auth {
        Some(ClientAuth::Body) => "body",
        _ => "header",
    };
    let fields = [
        ("grant_type", grant),
        ("accessTokenUrl", &o.access_token_url),
        ("refreshTokenUrl", &o.refresh_token_url),
        ("clientId", &o.client_id),
        ("clientSecret", &o.client_secret),
        ("scope", &o.scope),
        ("state", &o.state),
        ("tokenName", &o.token_id),
        ("addTokenTo", add_to),
        ("headerPrefix", prefix),
        ("client_authentication", client_auth),
        ("authUrl", &o.authorization_url),
        ("redirect_uri", &o.callback_url),
        ("username", &o.username),
        ("password", &o.password),
    ];
    let kept: Vec<(&str, &str)> = fields.into_iter().filter(|(_, v)| !v.is_empty()).collect();
    let mut out = strings(&kept);

    let stages = [
        (OAuth2Stage::Authorization, "authRequestParams"),
        (OAuth2Stage::Token, "tokenRequestParams"),
        (OAuth2Stage::Refresh, "refreshRequestParams"),
    ];
    for (stage, key) in stages {
        let params: Vec<Value> = o
            .params
            .iter()
            .filter(|p| p.stage == stage)
            .map(|p| {
                let send_as = match p.placement {
                    OAuth2ParamPlacement::Query => "request_url",
                    OAuth2ParamPlacement::Body => "request_body",
                    _ => "request_header",
                };
                json!({"key": p.name, "value": p.value, "enabled": true, "send_as": send_as})
            })
            .collect();
        if !params.is_empty() {
            out.push(json!({"key": key, "value": params, "type": "any"}));
        }
    }
    out
}
