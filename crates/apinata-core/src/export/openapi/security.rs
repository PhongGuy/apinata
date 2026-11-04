//! Auth as OpenAPI security schemes: the scheme describes how a client authenticates and
//! never carries a credential value.

use crate::export::literal_credential_query;
use crate::{ApiKeyPlacement, AuthSpec, OAuth2Flow, OAuth2Spec};
use ::url::Url;
use serde_json::{Map, Value, json};

/// What an auth becomes in OpenAPI.
pub(super) enum Mapped {
    /// No authentication (`AuthSpec::None`; `Inherit` never reaches here).
    None,
    /// A scheme object, its kind's base name (`basicAuth`, `bearerAuth`, `digestAuth`,
    /// `apiKeyAuth`, `oauth2`) and the scopes a requirement lists.
    Scheme {
        scheme: Value,
        base: &'static str,
        scopes: Vec<String>,
    },
    /// Not expressible; the note detail.
    Unmappable(String),
}

/// The OpenAPI form of `spec`, read from its mode, key placement and OAuth 2 URLs only.
pub(super) fn map_auth(spec: &AuthSpec) -> Mapped {
    let http = |scheme: &str, base| Mapped::Scheme {
        scheme: json!({"type": "http", "scheme": scheme}),
        base,
        scopes: Vec::new(),
    };
    match spec {
        AuthSpec::None | AuthSpec::Inherit => Mapped::None,
        AuthSpec::Basic { .. } => http("basic", "basicAuth"),
        AuthSpec::Bearer { .. } => http("bearer", "bearerAuth"),
        AuthSpec::Digest { .. } => http("digest", "digestAuth"),
        AuthSpec::ApiKey { key, .. } if key.trim().is_empty() => unmappable("apikey"),
        AuthSpec::ApiKey { key, placement, .. } => {
            let place = match placement {
                ApiKeyPlacement::Header => "header",
                ApiKeyPlacement::Query => "query",
            };
            Mapped::Scheme {
                scheme: json!({"type": "apiKey", "name": key, "in": place}),
                base: "apiKeyAuth",
                scopes: Vec::new(),
            }
        }
        AuthSpec::OAuth2(spec) => oauth2(spec),
        AuthSpec::OAuth1(_) => unmappable("oauth1"),
        AuthSpec::AwsV4 { .. } => unmappable("awsv4"),
        AuthSpec::Wsse { .. } => unmappable("wsse"),
        AuthSpec::Ntlm { .. } => unmappable("ntlm"),
        AuthSpec::EdgeGrid(_) => unmappable("akamai-edgegrid"),
        AuthSpec::Unsupported { mode } => unmappable(mode),
    }
}

fn unmappable(mode: &str) -> Mapped {
    Mapped::Unmappable(format!("the {mode} authentication cannot be exported"))
}

/// The flow's OpenAPI name and the URLs it requires as (key, URL, label).
fn flow_urls(spec: &OAuth2Spec) -> (&'static str, Vec<(&'static str, &String, &'static str)>) {
    let (flow, authorize, token) = match spec.flow {
        OAuth2Flow::ClientCredentials => ("clientCredentials", false, true),
        OAuth2Flow::Password => ("password", false, true),
        OAuth2Flow::AuthorizationCode => ("authorizationCode", true, true),
        OAuth2Flow::Implicit => ("implicit", true, false),
    };
    let all = [
        (
            authorize,
            "authorizationUrl",
            &spec.authorization_url,
            "authorization",
        ),
        (token, "tokenUrl", &spec.access_token_url, "token"),
    ];
    let urls = all
        .into_iter()
        .filter(|(wanted, ..)| *wanted)
        .map(|(_, key, url, label)| (key, url, label))
        .collect();
    (flow, urls)
}

/// A note detail per OAuth 2 URL written into the scheme whose query string carries a
/// literal credential; empty for any other auth or an auth that is not exported.
pub(super) fn credential_url_notes(spec: &AuthSpec) -> Vec<String> {
    let AuthSpec::OAuth2(oauth) = spec else {
        return Vec::new();
    };
    let mut details = Vec::new();
    for (_, url, label) in flow_urls(oauth).1 {
        if !is_absolute_http(url) {
            continue;
        }
        let Ok(parsed) = Url::parse(url) else {
            continue;
        };
        for (name, value) in parsed.query_pairs() {
            if literal_credential_query(&name, &value) {
                details.push(format!(
                    "oauth2 {label} URL query parameter {name} holds a literal credential, written into the document as a flow URL"
                ));
            }
        }
    }
    details
}

/// One flow holding only the URLs that flow requires, each an absolute http(s) URL, and
/// the whitespace-split scope with empty descriptions.
fn oauth2(spec: &OAuth2Spec) -> Mapped {
    let (flow, urls) = flow_urls(spec);
    let mut obj = Map::new();
    for (key, url, label) in urls {
        if !is_absolute_http(url) {
            return Mapped::Unmappable(format!(
                "the oauth2 authentication cannot be exported: its {label} URL is not an absolute http or https URL"
            ));
        }
        obj.insert(key.into(), url.as_str().into());
    }
    let mut scopes: Vec<String> = Vec::new();
    for scope in spec.scope.split_whitespace() {
        if !scopes.iter().any(|s| s == scope) {
            scopes.push(scope.to_owned());
        }
    }
    let described: Map<String, Value> = scopes.iter().map(|s| (s.clone(), "".into())).collect();
    obj.insert("scopes".into(), described.into());
    Mapped::Scheme {
        scheme: json!({"type": "oauth2", "flows": {flow: obj}}),
        base: "oauth2",
        scopes,
    }
}

/// Whether `url` is written out (no `{{` template, no surrounding whitespace) and parses
/// with an http or https scheme and no user name or password, which must never be exported.
fn is_absolute_http(url: &str) -> bool {
    !url.is_empty()
        && !url.contains("{{")
        && url.trim() == url
        && Url::parse(url).is_ok_and(|u| {
            matches!(u.scheme(), "http" | "https")
                && u.username().is_empty()
                && u.password().is_none()
        })
}

/// Named schemes in first-seen order; equal scheme values share a name.
#[derive(Default)]
pub(super) struct Schemes(Vec<(String, Value)>);

impl Schemes {
    /// The name of `scheme`: an equal one's, else the first of `base`, `base2`, `base3`
    /// not yet taken.
    pub(super) fn name(&mut self, scheme: Value, base: &str) -> String {
        if let Some((name, _)) = self.0.iter().find(|(_, s)| *s == scheme) {
            return name.clone();
        }
        let taken = |n: &str| self.0.iter().any(|(name, _)| name == n);
        let mut name = base.to_owned();
        let mut n = 2;
        while taken(&name) {
            name = format!("{base}{n}");
            n += 1;
        }
        self.0.push((name.clone(), scheme));
        name
    }

    /// The `components.securitySchemes` object, or `None` when no scheme was named.
    pub(super) fn into_components(self) -> Option<Value> {
        if self.0.is_empty() {
            return None;
        }
        let schemes: Map<String, Value> = self.0.into_iter().collect();
        Some(json!({"securitySchemes": schemes}))
    }
}
