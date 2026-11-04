//! Auth mapping: a port of the reference's `toOpenCollectionAuth` and `toOpenCollectionOAuth2`.
//! Keys are inserted in the reference's order, and a key whose JavaScript value would be
//! `undefined` is never inserted.

use super::value::{is_non_empty_string, js_or, truthy};
use crate::js::js_trim;
use serde_json::{Map, Value, json};

type Object = Map<String, Value>;

/// A property of a value; anything but an object has none, as property access on a primitive.
fn field<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    value
        .and_then(Value::as_object)
        .and_then(|map| map.get(key))
}

/// Copies each listed field holding a string, empty or not (`isString`).
fn copy_strings(out: &mut Object, config: Option<&Value>, keys: &[&str]) {
    for key in keys {
        if let Some(text @ Value::String(_)) = field(config, key) {
            out.insert((*key).into(), text.clone());
        }
    }
}

/// A `{ type }` object followed by the listed string fields.
fn with_strings(kind: &str, config: Option<&Value>, keys: &[&str]) -> Object {
    let mut out = Object::new();
    out.insert("type".into(), json!(kind));
    copy_strings(&mut out, config, keys);
    out
}

fn apikey(config: Option<&Value>) -> Object {
    let mut out = with_strings("apikey", config, &["key", "value"]);
    let placement = match field(config, "placement").and_then(Value::as_str) {
        Some("header") => Some("header"),
        Some("queryparams") => Some("query"),
        _ => None,
    };
    if let Some(placement) = placement {
        out.insert("placement".into(), json!(placement));
    }
    out
}

fn edgegrid(config: Option<&Value>) -> Object {
    let keys = [
        "accessToken",
        "clientToken",
        "clientSecret",
        "nonce",
        "timestamp",
        "baseURL",
        "headersToSign",
    ];
    let mut out = with_strings("akamai-edgegrid", config, &keys);
    if let Some(size @ Value::Number(_)) = field(config, "maxBodySize") {
        out.insert("maxBodySize".into(), size.clone());
    }
    out
}

/// OAuth1 splits around the private key, which becomes a `{type, value}` pair.
fn oauth1(config: Option<&Value>) -> Object {
    let head = [
        "consumerKey",
        "consumerSecret",
        "accessToken",
        "accessTokenSecret",
        "callbackUrl",
        "verifier",
        "signatureMethod",
    ];
    let mut out = with_strings("oauth1", config, &head);
    if let Some(key @ Value::String(_)) = field(config, "privateKey") {
        let kind = match field(config, "privateKeyType") {
            Some(Value::String(t)) if t == "file" => "file",
            _ => "text",
        };
        out.insert("privateKey".into(), json!({"type": kind, "value": key}));
    }
    let tail = ["timestamp", "nonce", "version", "realm", "placement"];
    copy_strings(&mut out, config, &tail);
    if let Some(flag @ Value::Bool(_)) = field(config, "includeBodyHash") {
        out.insert("includeBodyHash".into(), flag.clone());
    }
    out
}

/// `toOpenCollectionAuth`; `inherit` maps to the string "inherit", an unknown mode to `None`.
pub(super) fn map_auth(auth: Option<&Value>) -> Option<Value> {
    if !truthy(auth) {
        return None;
    }
    let mode = field(auth, "mode").and_then(Value::as_str);
    let config = |key| field(auth, key);
    let mapped = match mode? {
        "none" => return None,
        "inherit" => return Some(json!("inherit")),
        "awsv4" => with_strings(
            "awsv4",
            config("awsv4"),
            &[
                "accessKeyId",
                "secretAccessKey",
                "sessionToken",
                "service",
                "region",
                "profileName",
            ],
        ),
        "basic" => with_strings("basic", config("basic"), &["username", "password"]),
        "bearer" => with_strings("bearer", config("bearer"), &["token"]),
        "digest" => with_strings("digest", config("digest"), &["username", "password"]),
        "ntlm" => with_strings("ntlm", config("ntlm"), &["username", "password", "domain"]),
        "wsse" => with_strings("wsse", config("wsse"), &["username", "password"]),
        "apikey" => apikey(config("apikey")),
        "oauth1" => oauth1(config("oauth1")),
        "oauth2" => return oauth2(config("oauth2")),
        "akamai-edgegrid" => edgegrid(config("akamaiEdgegrid")),
        _ => return None,
    };
    Some(Value::Object(mapped))
}

/// Copies each listed field holding a non-blank string (`isNonEmptyString`) under its new name.
fn copy_non_empty(out: &mut Object, oauth: Option<&Value>, keys: &[(&str, &str)]) {
    for (from, to) in keys {
        if let Some(value) = field(oauth, from).filter(|v| is_non_empty_string(Some(v))) {
            out.insert((*to).into(), value.clone());
        }
    }
}

/// Inserts an object only when it has keys, as the reference's `Object.keys(x).length > 0`.
fn insert_non_empty(out: &mut Object, key: &str, value: Object) {
    if !value.is_empty() {
        out.insert(key.into(), Value::Object(value));
    }
}

/// `mapAdditionalParameters`: rows with a blank name or an unknown `sendIn` are dropped.
fn additional(oauth: Option<&Value>, group: &str) -> Option<Value> {
    let rows = field(field(oauth, "additionalParameters"), group)?.as_array()?;
    let mapped: Vec<Value> = rows
        .iter()
        .filter_map(|row| {
            let name = field(Some(row), "name");
            if !is_non_empty_string(name) {
                return None;
            }
            let send_in = field(Some(row), "sendIn")?.as_str()?;
            let placement = match js_trim(send_in).to_lowercase().as_str() {
                "headers" => "header",
                "queryparams" => "query",
                "body" => "body",
                _ => return None,
            };
            let mut param = Object::new();
            param.insert("name".into(), json!(js_trim(name?.as_str()?)));
            param.insert("placement".into(), json!(placement));
            copy_non_empty(&mut param, Some(row), &[("value", "value")]);
            Some(Value::Object(param))
        })
        .collect();
    (!mapped.is_empty()).then_some(Value::Array(mapped))
}

/// The `additionalParameters` object for the listed groups, absent when every group is empty.
fn additional_parameters(out: &mut Object, oauth: Option<&Value>, groups: &[(&str, &str)]) {
    let mut params = Object::new();
    for (from, to) in groups {
        if let Some(rows) = additional(oauth, from) {
            params.insert((*to).into(), rows);
        }
    }
    insert_non_empty(out, "additionalParameters", params);
}

fn credentials(out: &mut Object, oauth: Option<&Value>) {
    let mut creds = Object::new();
    let keys = [
        ("clientId", "clientId"),
        ("clientSecret", "clientSecret"),
        ("credentialsPlacement", "placement"),
    ];
    copy_non_empty(&mut creds, oauth, &keys);
    insert_non_empty(out, "credentials", creds);
}

/// `buildTokenConfig`: a blank placement defaults to an empty header prefix, and a header or URL
/// placement copies its prefix or key as is, dropped only when absent.
fn token_config(out: &mut Object, oauth: Option<&Value>) {
    let mut config = Object::new();
    copy_non_empty(&mut config, oauth, &[("credentialsId", "id")]);
    let placement = field(oauth, "tokenPlacement");
    if !is_non_empty_string(placement) {
        config.insert("placement".into(), json!({"header": ""}));
    }
    let target = match placement.and_then(Value::as_str) {
        Some("header") => Some(("header", "tokenHeaderPrefix")),
        Some("url") => Some(("query", "tokenQueryKey")),
        _ => None,
    };
    if let Some((key, source)) = target {
        let mut at = Object::new();
        if let Some(value) = field(oauth, source) {
            at.insert(key.into(), value.clone());
        }
        config.insert("placement".into(), Value::Object(at));
    }
    let source = js_or(field(oauth, "tokenSource"), json!("access_token"));
    config.insert("source".into(), source);
    out.insert("tokenConfig".into(), Value::Object(config));
}

fn settings(out: &mut Object, oauth: Option<&Value>) {
    let mut settings = Object::new();
    for key in ["autoFetchToken", "autoRefreshToken"] {
        if let Some(flag @ Value::Bool(_)) = field(oauth, key) {
            settings.insert(key.into(), flag.clone());
        }
    }
    insert_non_empty(out, "settings", settings);
}

/// `toOpenCollectionOAuth2`: one builder per grant type, `None` for a missing or unknown one.
fn oauth2(oauth: Option<&Value>) -> Option<Value> {
    if !truthy(oauth) {
        return None;
    }
    let flow = match field(oauth, "grantType").and_then(Value::as_str)? {
        "client_credentials" => "client_credentials",
        "password" => "resource_owner_password_credentials",
        "authorization_code" => "authorization_code",
        "implicit" => "implicit",
        _ => return None,
    };
    let mut out = Object::new();
    out.insert("type".into(), json!("oauth2"));
    out.insert("flow".into(), json!(flow));
    let (token, refresh) = (
        ("token", "accessTokenRequest"),
        ("refresh", "refreshTokenRequest"),
    );
    let token_groups = [token, refresh];
    match flow {
        "implicit" => {
            let keys = [
                ("authorizationUrl", "authorizationUrl"),
                ("callbackUrl", "callbackUrl"),
            ];
            copy_non_empty(&mut out, oauth, &keys);
            let mut id = Object::new();
            copy_non_empty(&mut id, oauth, &[("clientId", "clientId")]);
            insert_non_empty(&mut out, "credentials", id);
            copy_non_empty(&mut out, oauth, &[("scope", "scope"), ("state", "state")]);
            let groups = [("authorization", "authorizationRequest")];
            additional_parameters(&mut out, oauth, &groups);
        }
        "authorization_code" => {
            let keys = [
                ("authorizationUrl", "authorizationUrl"),
                ("accessTokenUrl", "accessTokenUrl"),
                ("refreshTokenUrl", "refreshTokenUrl"),
                ("callbackUrl", "callbackUrl"),
            ];
            copy_non_empty(&mut out, oauth, &keys);
            credentials(&mut out, oauth);
            let groups = [("authorization", "authorizationRequest"), token, refresh];
            additional_parameters(&mut out, oauth, &groups);
            copy_non_empty(&mut out, oauth, &[("scope", "scope"), ("state", "state")]);
            match field(oauth, "pkce") {
                None | Some(Value::Null) => {}
                pkce if truthy(pkce) => {
                    out.insert("pkce".into(), json!({}));
                }
                _ => {
                    out.insert("pkce".into(), json!({"disabled": true}));
                }
            }
        }
        _ => {
            let keys = [
                ("accessTokenUrl", "accessTokenUrl"),
                ("refreshTokenUrl", "refreshTokenUrl"),
            ];
            copy_non_empty(&mut out, oauth, &keys);
            credentials(&mut out, oauth);
            if flow == "resource_owner_password_credentials" {
                let mut owner = Object::new();
                let keys = [("username", "username"), ("password", "password")];
                copy_non_empty(&mut owner, oauth, &keys);
                insert_non_empty(&mut out, "resourceOwner", owner);
            }
            copy_non_empty(&mut out, oauth, &[("scope", "scope")]);
            additional_parameters(&mut out, oauth, &token_groups);
        }
    }
    token_config(&mut out, oauth);
    settings(&mut out, oauth);
    Some(Value::Object(out))
}
