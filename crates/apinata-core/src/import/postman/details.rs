//! Auth, collection variables and `protocolProfileBehavior`, as the reference converter's
//! `processAuth`, `importCollectionLevelVariables` and request settings map them.

use super::notes::members;
use super::{ImportNote, NoteKind, ensure_string, js_string, truthy};
use crate::edit::{
    ApiKeyPlacement, AuthSpec, ClientAuth, EdgeGridSpec, OAuth1KeyKind, OAuth1Placement,
    OAuth1PrivateKey, OAuth1SignatureMethod, OAuth1Spec, OAuth2Flow, OAuth2Param,
    OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage, RequestSettings, TokenPlacement, VarValue,
    Variable,
};
use crate::js;
use serde_json::Value;

/// The collection variable members the reference reads, besides metadata.
const VARIABLE_KEYS: [&str; 4] = ["key", "value", "disabled", "type"];

/// The `protocolProfileBehavior` keys that have a request setting.
const SETTING_KEYS: [&str; 4] = [
    "disableUrlEncoding",
    "followAuthorizationHeader",
    "followRedirects",
    "maxRedirects",
];

/// An `auth` member: falsy gives `absent` (none on the collection, inherit below it). A type
/// core cannot hold gives no auth and a `Converted` note at `source`.
pub(super) fn auth(
    value: Option<&Value>,
    absent: AuthSpec,
    source: String,
    notes: &mut Vec<ImportNote>,
) -> AuthSpec {
    let Some(auth) = value.filter(|v| truthy(Some(v))) else {
        return absent;
    };
    let kind = auth.get("type").and_then(Value::as_str);
    let values = kind.and_then(|k| auth.get(k));
    let get = |key: &str| ensure_string(auth_value(values, key));
    match kind {
        Some("noauth") => AuthSpec::None,
        Some("basic") => AuthSpec::Basic {
            username: get("username"),
            password: get("password"),
        },
        Some("bearer") => AuthSpec::Bearer {
            token: get("token"),
        },
        Some("apikey") => AuthSpec::ApiKey {
            key: get("key"),
            value: get("value"),
            placement: match auth_value(values, "in") {
                Some(Value::String(place)) if place == "query" => ApiKeyPlacement::Query,
                _ => ApiKeyPlacement::Header,
            },
        },
        Some("oauth1") => AuthSpec::OAuth1(oauth1(values)),
        Some("edgegrid") => AuthSpec::EdgeGrid(edgegrid(values)),
        Some("ntlm") => {
            // Apinata derives the workstation itself, so a stated one is noted and dropped.
            if !get("workstation").is_empty() {
                notes.push(ImportNote::new(
                    source,
                    NoteKind::Dropped,
                    "the NTLM workstation has no counterpart".to_owned(),
                ));
            }
            AuthSpec::Ntlm {
                username: get("username"),
                password: get("password"),
                domain: get("domain"),
            }
        }
        Some("digest") => {
            // Apinata negotiates these from the server's challenge, so a stated one is dropped.
            let blank = |key: &str| get(key).trim().is_empty();
            let md5 = get("algorithm").trim().eq_ignore_ascii_case("md5");
            let mut dropped = Vec::new();
            if !blank("algorithm") && !md5 {
                dropped.push("algorithm");
            }
            for key in [
                "realm",
                "nonce",
                "qop",
                "opaque",
                "nonceCount",
                "clientNonce",
            ] {
                if !blank(key) {
                    dropped.push(key);
                }
            }
            if truthy(auth_value(values, "disableRetryRequest")) {
                dropped.push("disableRetryRequest");
            }
            drop_notes(
                notes,
                &source,
                &dropped,
                "negotiated from the server challenge",
            );
            AuthSpec::Digest {
                username: get("username"),
                password: get("password"),
            }
        }
        Some("awsv4") => {
            if truthy(auth_value(values, "addAuthDataToQuery")) {
                drop_notes(
                    notes,
                    &source,
                    &["addAuthDataToQuery"],
                    "the signature always goes in the header",
                );
            }
            AuthSpec::AwsV4 {
                access_key_id: get("accessKey"),
                secret_access_key: get("secretKey"),
                session_token: get("sessionToken"),
                service: get("service"),
                region: get("region"),
                profile_name: String::new(),
            }
        }
        Some("oauth2") => AuthSpec::OAuth2(oauth2(values, &source, notes)),
        other => {
            let detail = match other {
                Some(kind) => format!("`{kind}` authentication is not supported; set to none"),
                None => "the authentication has no type; set to none".to_owned(),
            };
            notes.push(ImportNote::new(source, NoteKind::Converted, detail));
            AuthSpec::None
        }
    }
}

/// One `Dropped` note per field name, never carrying a value.
fn drop_notes(notes: &mut Vec<ImportNote>, source: &str, fields: &[&str], why: &str) {
    for field in fields {
        notes.push(ImportNote::new(
            source,
            NoteKind::Dropped,
            format!("`{field}` is not carried over: {why}"),
        ));
    }
}

/// The reference's oauth2 mapping, keeping only the chosen grant's own fields. Stored tokens
/// are never copied, and every Postman option core cannot hold is noted by name.
fn oauth2(values: Option<&Value>, source: &str, notes: &mut Vec<ImportNote>) -> OAuth2Spec {
    let get = |key: &str| ensure_string(auth_value(values, key));
    let blank = |key: &str| get(key).trim().is_empty();
    let grant = get("grant_type");
    let (flow, pkce) = match grant.as_str() {
        "password_credentials" => (OAuth2Flow::Password, false),
        "authorization_code" => (OAuth2Flow::AuthorizationCode, false),
        "authorization_code_with_pkce" => (OAuth2Flow::AuthorizationCode, true),
        "implicit" => (OAuth2Flow::Implicit, false),
        other => {
            if !other.trim().is_empty() && other != "client_credentials" {
                notes.push(ImportNote::new(
                    source,
                    NoteKind::Converted,
                    format!("the grant type `{other}` is not supported; set to client_credentials"),
                ));
            }
            (OAuth2Flow::ClientCredentials, false)
        }
    };
    let mut o = OAuth2Spec::new(flow);
    o.pkce = pkce;
    o.client_id = get("clientId");
    o.scope = get("scope");
    if flow != OAuth2Flow::Implicit {
        o.access_token_url = get("accessTokenUrl");
        o.refresh_token_url = get("refreshTokenUrl");
        o.client_secret = get("clientSecret");
        o.client_auth = Some(if get("client_authentication") == "body" {
            ClientAuth::Body
        } else {
            ClientAuth::BasicHeader
        });
    }
    if matches!(flow, OAuth2Flow::AuthorizationCode | OAuth2Flow::Implicit) {
        o.authorization_url = get("authUrl");
        o.callback_url = get("redirect_uri");
        o.state = get("state");
    }
    if flow == OAuth2Flow::Password {
        o.username = get("username");
        o.password = get("password");
    }
    if !blank("tokenName") {
        o.token_id = get("tokenName");
    }
    // Postman defaults to the header; only an explicit query choice moves the token.
    o.token_placement = match get("addTokenTo").as_str() {
        "queryParams" | "url" => TokenPlacement::Query {
            key: "access_token".to_owned(),
        },
        _ => TokenPlacement::Header {
            prefix: if blank("headerPrefix") {
                "Bearer".to_owned()
            } else {
                get("headerPrefix")
            },
        },
    };
    o.params = oauth2_params(values, flow, source, notes);

    // Each group is noted with the reason that is true for it, never with a value.
    let mut unused = Vec::new();
    if !matches!(flow, OAuth2Flow::AuthorizationCode | OAuth2Flow::Implicit) && !blank("state") {
        unused.push("state");
    }
    if flow == OAuth2Flow::Implicit {
        for key in ["clientSecret", "accessTokenUrl", "refreshTokenUrl"] {
            if !blank(key) {
                unused.push(key);
            }
        }
    }
    drop_notes(notes, source, &unused, "this grant does not use it");

    let mut stored = Vec::new();
    for key in ["accessToken", "refresh_token"] {
        if !blank(key) {
            stored.push(key);
        }
    }
    // A zero lifetime is Postman's empty default, not a stated expiry.
    if !blank("expires_in") && get("expires_in").trim() != "0" {
        stored.push("expires_in");
    }
    drop_notes(notes, source, &stored, "a stored token is never imported");

    let mut other = Vec::new();
    // Apinata always uses S256 for PKCE.
    if !blank("challengeAlgorithm") && get("challengeAlgorithm") != "S256" {
        other.push("challengeAlgorithm");
    }
    if !blank("tokenType") && !get("tokenType").trim().eq_ignore_ascii_case("bearer") {
        other.push("tokenType");
    }
    for key in ["code_verifier", "audience", "resource"] {
        if !blank(key) {
            other.push(key);
        }
    }
    for key in ["useBrowser", "isAuthInSecondaryWindow"] {
        if truthy(auth_value(values, key)) {
            other.push(key);
        }
    }
    drop_notes(notes, source, &other, "Apinata has no setting for it");
    o
}

/// The additional request parameters of the flow's stages. A disabled one is noted, since core
/// keeps no enabled flag.
fn oauth2_params(
    values: Option<&Value>,
    flow: OAuth2Flow,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Vec<OAuth2Param> {
    let mut params = Vec::new();
    for (member, stage, label) in [
        (
            "authRequestParams",
            OAuth2Stage::Authorization,
            "authorization",
        ),
        ("tokenRequestParams", OAuth2Stage::Token, "token"),
        ("refreshRequestParams", OAuth2Stage::Refresh, "refresh"),
    ] {
        let Some(Value::Array(entries)) = auth_value(values, member) else {
            continue;
        };
        let used = flow.stages().contains(&stage);
        for entry in entries {
            let text = |key: &str| ensure_string(entry.get(key));
            let disabled = matches!(entry.get("enabled"), Some(Value::Bool(false)));
            if !used {
                if !disabled {
                    notes.push(ImportNote::new(
                        source,
                        NoteKind::Dropped,
                        format!(
                            "the {label} parameter `{}` is not carried over: this grant has no {label} request",
                            text("key")
                        ),
                    ));
                }
                continue;
            }
            if disabled {
                notes.push(ImportNote::new(
                    source,
                    NoteKind::Dropped,
                    format!(
                        "the disabled {label} parameter `{}` is not carried over",
                        text("key")
                    ),
                ));
                continue;
            }
            let placement = match text("send_as").as_str() {
                "request_url" => OAuth2ParamPlacement::Query,
                "request_body" => OAuth2ParamPlacement::Body,
                _ => OAuth2ParamPlacement::Header,
            };
            params.push(OAuth2Param::new(
                stage,
                text("key"),
                text("value"),
                placement,
            ));
        }
    }
    params
}

/// The reference's oauth1 mapping: optional fields are absent when blank, and only a JSON
/// `false` for `addParamsToHeader` moves the parameters to the query.
fn oauth1(values: Option<&Value>) -> OAuth1Spec {
    let get = |key: &str| ensure_string(auth_value(values, key));
    let filled = |key: &str| Some(get(key)).filter(|v| !v.is_empty());
    let method = filled("signatureMethod").unwrap_or_else(|| "HMAC-SHA1".to_owned());
    OAuth1Spec {
        consumer_key: Some(get("consumerKey")),
        consumer_secret: Some(get("consumerSecret")),
        access_token: Some(get("token")),
        access_token_secret: Some(get("tokenSecret")),
        callback_url: filled("callback"),
        verifier: filled("verifier"),
        signature_method: OAuth1SignatureMethod::parse(&method),
        private_key: filled("privateKey").map(|k| OAuth1PrivateKey::new(OAuth1KeyKind::Text, k)),
        timestamp: filled("timestamp"),
        nonce: filled("nonce"),
        version: filled("version").unwrap_or_else(|| "1.0".to_owned()),
        realm: filled("realm"),
        placement: match auth_value(values, "addParamsToHeader") {
            Some(Value::Bool(false)) => OAuth1Placement::Query,
            _ => OAuth1Placement::Header,
        },
        include_body_hash: truthy(auth_value(values, "includeBodyHash")),
    }
}

/// `Number(value)` for a `maxBodySize` value. An array converts through its comma-joined text,
/// so `[]` is 0, `[7]` is 7 and `[1, 2]` is NaN; null, `""` and objects are absent.
fn max_body_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) if !s.is_empty() => js::parse_number(s),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Array(items) => js::parse_number(&array_text(items)),
        _ => None,
    }
}

/// JavaScript `String(array)`: elements joined by commas, null as nothing.
fn array_text(items: &[Value]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|item| match item {
            Value::Null => String::new(),
            Value::Array(inner) => array_text(inner),
            other => js::to_text(other),
        })
        .collect();
    parts.join(",")
}

/// The reference's edgegrid mapping: every field as text, and `maxBodySize` as a number when
/// `Number(value)` gives one, absent for null, `""` or NaN.
fn edgegrid(values: Option<&Value>) -> EdgeGridSpec {
    let get = |key: &str| ensure_string(auth_value(values, key));
    let max_body_size = auth_value(values, "maxBodySize").and_then(max_body_number);
    EdgeGridSpec {
        access_token: get("accessToken"),
        client_token: get("clientToken"),
        client_secret: get("clientSecret"),
        nonce: get("nonce"),
        timestamp: get("timestamp"),
        base_url: get("baseURL"),
        headers_to_sign: get("headersToSign"),
        max_body_size,
    }
}

/// One auth parameter, from a v2.0 object or a v2.1 `[{key, value}]` list where the last
/// entry with the key wins, as the reference's `reduce` does.
fn auth_value<'a>(values: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match values? {
        Value::Array(entries) => entries
            .iter()
            .rev()
            .find(|e| e.get("key").is_some_and(|k| js_string(k) == key))
            .and_then(|e| e.get("value")),
        other => other.get(key),
    }
}

/// Collection variables. Names have every UTF-16 unit outside `[A-Za-z0-9_.-]` replaced by
/// `_`. Input the reference throws on (not a list, a `null` entry, a non-text key) drops
/// them all, as its caught error does.
pub(super) fn variables(value: Option<&Value>, notes: &mut Vec<ImportNote>) -> Vec<Variable> {
    let Some(value) = value.filter(|v| truthy(Some(v))) else {
        return Vec::new();
    };
    let drop_all = |notes: &mut Vec<ImportNote>, detail: &str| {
        notes.push(ImportNote::new(
            "collection / variable",
            NoteKind::Dropped,
            detail,
        ));
        Vec::new()
    };
    let Value::Array(entries) = value else {
        return drop_all(notes, "the collection variables are not a list");
    };
    // Held back until every entry is read, since a later entry can still drop them all.
    let mut pending = Vec::new();
    let mut vars = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let source = format!("collection / variable[{index}]");
        match entry {
            Value::Null => return drop_all(notes, "a collection variable is null"),
            Value::Object(map) => {
                let unread = |key: &str| !VARIABLE_KEYS.contains(&key);
                members(map, &unread, &format!("{source}."), &mut pending);
            }
            _ => pending.push(ImportNote::new(
                source.clone(),
                NoteKind::Dropped,
                "the variable is not an object",
            )),
        }
        let key = entry.get("key").filter(|k| !k.is_null());
        let value = entry.get("value").filter(|v| !v.is_null());
        if key.is_none() && value.is_none() {
            continue;
        }
        let key = match key {
            None => "",
            Some(Value::String(key)) => key.as_str(),
            Some(_) => return drop_all(notes, "a collection variable name is not text"),
        };
        let name = clean_name(key);
        if name != key {
            pending.push(ImportNote::new(
                format!("{source}.key"),
                NoteKind::Converted,
                format!("the variable `{key}` was renamed to `{name}`"),
            ));
        }
        let text = match value {
            None => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(other) => js::stringify(other),
        };
        let mut var = Variable::new(name, VarValue::Text(text));
        var.enabled = !truthy(entry.get("disabled"));
        vars.push(var);
    }
    notes.append(&mut pending);
    vars
}

pub(super) fn clean_name(key: &str) -> String {
    let mut name = String::with_capacity(key.len());
    for c in key.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
            name.push(c);
        } else {
            name.extend(std::iter::repeat_n('_', c.len_utf16()));
        }
    }
    name
}

/// A request's settings from its item's `protocolProfileBehavior`; anything without a setting
/// gets a `Dropped` note at `<crumb> / protocolProfileBehavior.<key>`.
pub(super) fn settings(
    behavior: Option<&Value>,
    crumb: &str,
    notes: &mut Vec<ImportNote>,
) -> RequestSettings {
    // Postman has no per-request timeout, so the new-file default (inherit) applies.
    let mut settings = RequestSettings::for_new_file();
    settings.forward_authorization_header = false;
    let behavior = match behavior {
        Some(Value::Object(behavior)) => behavior,
        None | Some(Value::Null) => return settings,
        Some(_) => {
            notes.push(ImportNote::new(
                format!("{crumb} / protocolProfileBehavior"),
                NoteKind::Dropped,
                "the request behavior is not an object",
            ));
            return settings;
        }
    };
    let mut drop = |key: &str, detail: &str| {
        notes.push(ImportNote::new(
            format!("{crumb} / protocolProfileBehavior.{key}"),
            NoteKind::Dropped,
            detail,
        ));
    };
    settings.encode_url = behavior.get("disableUrlEncoding") != Some(&Value::Bool(true));
    match behavior.get("followAuthorizationHeader") {
        Some(Value::Bool(forward)) => settings.forward_authorization_header = *forward,
        None | Some(Value::Null) => {}
        Some(_) => drop(
            "followAuthorizationHeader",
            "the value is not true or false",
        ),
    }
    match behavior.get("followRedirects") {
        Some(Value::Bool(follow)) => settings.follow_redirects = *follow,
        None | Some(Value::Null) => {}
        Some(_) => drop("followRedirects", "the value is not true or false"),
    }
    // Pushed once `drop`, which borrows `notes`, is done.
    let mut converted = None;
    match behavior.get("maxRedirects") {
        // Core holds a whole u32, so a fraction is truncated and a huge count saturates;
        // either change is reported rather than made silently.
        Some(Value::Number(n)) if n.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0) => {
            let limit = n.as_f64().unwrap_or(5.0);
            settings.max_redirects = limit.trunc() as u32;
            let kept = settings.max_redirects;
            let detail = if limit > f64::from(u32::MAX) {
                Some(format!(
                    "the redirect limit {n} is above the largest supported; {kept} is used"
                ))
            } else if limit.fract() != 0.0 {
                Some(format!(
                    "the redirect limit {n} is not a whole number; {kept} is used"
                ))
            } else {
                None
            };
            converted = detail.map(|detail| {
                ImportNote::new(
                    format!("{crumb} / protocolProfileBehavior.maxRedirects"),
                    NoteKind::Converted,
                    detail,
                )
            });
        }
        None => {}
        Some(_) => drop(
            "maxRedirects",
            "the redirect limit is not a number of 0 or more; the default of 5 is used",
        ),
    }
    for key in behavior.keys() {
        if !SETTING_KEYS.contains(&key.as_str()) {
            drop(key, "this request behavior has no counterpart");
        }
    }
    notes.extend(converted);
    settings
}

/// A `protocolProfileBehavior` on a folder or the collection, which has no settings to hold
/// it, is dropped whole.
pub(super) fn folder_behavior(
    behavior: Option<&Value>,
    source: String,
    notes: &mut Vec<ImportNote>,
) {
    if behavior.is_some_and(|b| !b.is_null()) {
        notes.push(ImportNote::new(
            source,
            NoteKind::Dropped,
            "request behavior is only kept on requests",
        ));
    }
}
