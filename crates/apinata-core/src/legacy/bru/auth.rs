//! Auth blocks and OAuth2 additional-parameter blocks, shared by the request and collection
//! grammars. Auth dictionaries keep `~` in names and the first pair with a name wins.

use super::lex::{Block, BlockBody, BlockKind, Pair, PairValue};
use super::{BruError, is_truthy, js_number, pair_value, pairs_to_rows};
use crate::js::parse_number;
use serde_json::{Map, Value, json};

/// The auth keywords in the reference grammar's ordered-choice order; `auth:oauth2` precedes
/// its `additional_params` blocks and relies on the dictionary failing to backtrack.
pub(super) const AUTH_BLOCKS: &[(&str, BlockKind)] = &[
    ("auth:awsv4", BlockKind::Dictionary),
    ("auth:basic", BlockKind::Dictionary),
    ("auth:bearer", BlockKind::Dictionary),
    ("auth:digest", BlockKind::Dictionary),
    ("auth:ntlm", BlockKind::Dictionary),
    ("auth:oauth1", BlockKind::Dictionary),
    ("auth:oauth2", BlockKind::Dictionary),
    ("auth:wsse", BlockKind::Dictionary),
    ("auth:apikey", BlockKind::Dictionary),
    ("auth:akamai-edgegrid", BlockKind::Dictionary),
    (
        "auth:oauth2:additional_params:auth_req:headers",
        BlockKind::Dictionary,
    ),
    (
        "auth:oauth2:additional_params:auth_req:queryparams",
        BlockKind::Dictionary,
    ),
    (
        "auth:oauth2:additional_params:access_token_req:headers",
        BlockKind::Dictionary,
    ),
    (
        "auth:oauth2:additional_params:access_token_req:queryparams",
        BlockKind::Dictionary,
    ),
    (
        "auth:oauth2:additional_params:access_token_req:body",
        BlockKind::Dictionary,
    ),
    (
        "auth:oauth2:additional_params:refresh_token_req:headers",
        BlockKind::Dictionary,
    ),
    (
        "auth:oauth2:additional_params:refresh_token_req:queryparams",
        BlockKind::Dictionary,
    ),
    (
        "auth:oauth2:additional_params:refresh_token_req:body",
        BlockKind::Dictionary,
    ),
];

/// The reference's oauth1 action calls `startsWith` on `private_key`, so a list value throws
/// after a successful parse; this reports the block's line for such a block.
pub(super) fn auth_block_error(block: &Block) -> Option<BruError> {
    let BlockBody::Pairs(pairs) = &block.body else {
        return None;
    };
    let list_key = block.keyword == "auth:oauth1"
        && matches!(
            find(pairs, "private_key").map(|p| &p.value),
            Some(PairValue::List(_))
        );
    list_key.then(|| BruError {
        line: block.line,
        message: "a list is not a valid private key".to_owned(),
    })
}

/// The JSON for an auth or additional-params block, or `None` for any other keyword.
pub(super) fn auth_block(keyword: &str, pairs: &[Pair]) -> Option<Value> {
    let text = |name: &str| find(pairs, name).map_or_else(|| json!(""), |p| pair_value(&p.value));
    let value = match keyword {
        "auth:awsv4" => json!({ "auth": { "awsv4": {
            "accessKeyId": text("accessKeyId"),
            "secretAccessKey": text("secretAccessKey"),
            "sessionToken": text("sessionToken"),
            "service": text("service"),
            "region": text("region"),
            "profileName": text("profileName"),
        } } }),
        "auth:basic" | "auth:digest" | "auth:wsse" => {
            let mode = keyword.trim_start_matches("auth:");
            json!({ "auth": { (mode): {
                "username": text("username"),
                "password": text("password"),
            } } })
        }
        "auth:bearer" => json!({ "auth": { "bearer": { "token": text("token") } } }),
        "auth:ntlm" => json!({ "auth": { "ntlm": {
            "username": text("username"),
            "password": text("password"),
            "domain": text("domain"),
        } } }),
        "auth:oauth1" => json!({ "auth": { "oauth1": oauth1(pairs) } }),
        "auth:oauth2" => json!({ "auth": { "oauth2": oauth2(pairs) } }),
        "auth:apikey" => json!({ "auth": { "apikey": {
            "key": text("key"),
            "value": text("value"),
            "placement": text("placement"),
        } } }),
        "auth:akamai-edgegrid" => json!({ "auth": { "akamaiEdgegrid": {
            "accessToken": text("accessToken"),
            "clientToken": text("clientToken"),
            "clientSecret": text("clientSecret"),
            "nonce": text("nonce"),
            "timestamp": text("timestamp"),
            "baseURL": text("baseURL"),
            "headersToSign": text("headersToSign"),
            "maxBodySize": max_body_size(find(pairs, "maxBodySize")),
        } } }),
        other => {
            let rest = other.strip_prefix("auth:oauth2:additional_params:")?;
            // `access_token_req:body` becomes `..._access_token_req_bodyvalues`.
            let suffix = rest.replace(':', "_");
            let suffix = if suffix.ends_with("_body") {
                format!("{suffix}values")
            } else {
                suffix
            };
            json!({ (format!("oauth2_additional_parameters_{suffix}")): pairs_to_rows(pairs) })
        }
    };
    Some(value)
}

/// `_.find(auth, { name })`: the first pair whose raw name matches, `~` included.
fn find<'a>(pairs: &'a [Pair], name: &str) -> Option<&'a Pair> {
    pairs.iter().find(|p| p.key == name)
}

/// JavaScript `String(value)`: a list joins its items with commas.
fn js_string(value: &PairValue) -> String {
    match value {
        PairValue::Text(text) => text.clone(),
        PairValue::List(items) => items.join(","),
    }
}

fn oauth1(pairs: &[Pair]) -> Value {
    // `findValue` returns the raw value, so a list stays a list; `private_key` is a string here
    // because `auth_block_error` rejects a list there.
    let text = |name: &str| find(pairs, name).map_or_else(|| json!(""), |p| pair_value(&p.value));
    let key = find(pairs, "private_key").map_or_else(String::new, |p| js_string(&p.value));
    let (private_key, key_type) = match key
        .strip_prefix("@file(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(path) => (path.to_owned(), "file"),
        None => (key, "text"),
    };
    json!({
        "consumerKey": text("consumer_key"),
        "consumerSecret": text("consumer_secret"),
        "accessToken": text("access_token"),
        "accessTokenSecret": text("token_secret"),
        "callbackUrl": text("callback_url"),
        "verifier": text("verifier"),
        "signatureMethod": text("signature_method"),
        "privateKey": private_key,
        "privateKeyType": key_type,
        "timestamp": text("timestamp"),
        "nonce": text("nonce"),
        "version": text("version"),
        "realm": text("realm"),
        "placement": text("placement"),
        "includeBodyHash": matches!(
            find(pairs, "include_body_hash").map(|p| &p.value),
            Some(PairValue::Text(t)) if t == "true"
        ),
    })
}

/// The oauth2 shape for the grant type: four grants have their own key sets, and any other
/// grant gives an empty object.
fn oauth2(pairs: &[Pair]) -> Value {
    let value = |name: &str| find(pairs, name).map(|p| pair_value(&p.value));
    let text = |name: &str| value(name).unwrap_or_else(|| json!(""));
    // `key?.value ? key.value : default`
    let or = |name: &str, default: &str| match value(name) {
        Some(v) if is_truthy(Some(&v)) => v,
        _ => json!(default),
    };
    // `key ? safeParseJson(key.value) ?? default : default`
    let flag = |name: &str, default: bool| {
        find(pairs, name)
            .and_then(|p| super::json::parse(&js_string(&p.value)).ok())
            .filter(|v| !v.is_null())
            .unwrap_or(Value::Bool(default))
    };
    let grant = find(pairs, "grant_type").map(|p| js_string(&p.value));
    let mut out = Map::new();
    let mut put = |key: &str, v: Value| {
        out.insert(key.to_owned(), v);
    };
    let grant = grant.as_deref().unwrap_or_default();
    if !matches!(
        grant,
        "password" | "authorization_code" | "client_credentials" | "implicit"
    ) {
        return Value::Object(Map::new());
    }
    put("grantType", text("grant_type"));
    if matches!(grant, "authorization_code" | "implicit") {
        put("callbackUrl", text("callback_url"));
        put("authorizationUrl", text("authorization_url"));
    }
    if grant != "implicit" {
        put("accessTokenUrl", text("access_token_url"));
        put("refreshTokenUrl", text("refresh_token_url"));
    }
    if grant == "password" {
        put("username", text("username"));
        put("password", text("password"));
    }
    put("clientId", text("client_id"));
    if grant != "implicit" {
        put("clientSecret", text("client_secret"));
    }
    put("scope", text("scope"));
    if matches!(grant, "authorization_code" | "implicit") {
        put("state", text("state"));
    }
    if grant == "authorization_code" {
        put("pkce", flag("pkce", false));
    }
    if grant != "implicit" {
        put("credentialsPlacement", or("credentials_placement", "body"));
    }
    put("credentialsId", or("credentials_id", "credentials"));
    put("tokenSource", or("token_source", "access_token"));
    put("tokenPlacement", or("token_placement", "header"));
    put("tokenHeaderPrefix", or("token_header_prefix", ""));
    put("tokenQueryKey", or("token_query_key", "access_token"));
    put("autoFetchToken", flag("auto_fetch_token", true));
    if grant != "implicit" {
        put("autoRefreshToken", flag("auto_refresh_token", false));
    }
    Value::Object(out)
}

/// `raw === '' || isNaN(Number(raw)) ? null : Number(raw)`, with a missing key read as `''`.
fn max_body_size(pair: Option<&Pair>) -> Value {
    match pair.map(|p| &p.value) {
        None => Value::Null,
        Some(PairValue::Text(raw)) if raw.is_empty() => Value::Null,
        Some(raw) => parse_number(&js_string(raw)).map_or(Value::Null, js_number),
    }
}
