//! The reverse of `dto`: sections an agent sends, applied through core's setters so every write
//! keeps Bruno's bytes. A section equal to what the file holds is skipped, and list entries reuse
//! the current entry of the same name, so keys MCP does not model survive an edit.

use apinata_core::{
    Assertion, AuthSpec, BodySpec, EditError, FileEntry, FolderPatch, KeyValue, MultipartField,
    MultipartValue, Param, ParamKind, RequestFile, RequestSettings, Timeout, VarValue, Variable,
    normalize_method,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::dto::{
    self, AssertionDoc, AuthDoc, BodyDoc, ConfigDoc, FileEntryDoc, KeyValueDoc, MultipartFieldDoc,
    MultipartValueDoc, ParamDoc, ParamKindDoc, SettingsDoc, TimeoutDoc, VarDoc,
};

/// `update_request`'s arguments: the request and the sections to replace.
#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestPatch {
    /// Collection-relative path of the request, such as `users/get.yml`.
    pub path: String,
    /// The display name in `info.name`; the file keeps its name.
    pub name: Option<String>,
    /// The HTTP method, such as `GET`.
    pub method: Option<String>,
    /// The URL; setting it re-derives the params from it, as the app does.
    pub url: Option<String>,
    /// Query and path params; setting them rebuilds the URL's query from the enabled query params.
    pub params: Option<Vec<ParamDoc>>,
    /// Headers in send order, disabled ones included.
    pub headers: Option<Vec<KeyValueDoc>>,
    /// The body; an `unsupported` mode is refused unless unchanged.
    pub body: Option<BodyDoc>,
    /// The authentication; an `unsupported` mode is refused unless unchanged.
    pub auth: Option<AuthDoc>,
    /// Request variables, set before the request is sent.
    pub vars: Option<Vec<VarDoc>>,
    /// Assertions, matched by expression; a description cannot be added or changed.
    pub assertions: Option<Vec<AssertionDoc>>,
    /// Request settings; a timeout of 0 ms is refused, use `unlimited`.
    pub settings: Option<SettingsDoc>,
    /// Markdown docs; blank text removes them.
    pub docs: Option<String>,
    /// Tags; none removes the key.
    pub tags: Option<Vec<String>>,
    /// Position among the folder's items.
    pub seq: Option<u64>,
    /// read_request's report, accepted so an echoed read applies, and never written.
    #[schemars(skip)]
    #[serde(default)]
    #[expect(dead_code, reason = "accepted only to be dropped")]
    pub query_mismatch: Option<IgnoredAny>,
    /// read_request's examples, accepted so an echoed read applies, and never written.
    #[schemars(skip)]
    #[serde(default)]
    #[expect(dead_code, reason = "accepted only to be dropped")]
    pub examples: Option<IgnoredAny>,
    /// Settles a `query_mismatch`, applied after every other section: `url` keeps the URL's
    /// query and rewrites the query params to match (the app's Keep URL), `params` rebuilds the
    /// URL's query from the enabled query params (the app's Keep table). An agreeing request is
    /// left as it is.
    pub sync_query: Option<SyncQueryDoc>,
}

/// Which side wins when `update_request` settles a URL and params query mismatch.
#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SyncQueryDoc {
    Url,
    Params,
}

/// `update_folder`'s arguments: the folder (`""` for the collection) and the sections to replace.
#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigPatch {
    /// Collection-relative folder path; empty for the collection itself.
    pub path: String,
    /// Headers every request below inherits, disabled ones included.
    pub headers: Option<Vec<KeyValueDoc>>,
    /// The authentication requests set to `inherit` use; `unsupported` is refused unless unchanged.
    pub auth: Option<AuthDoc>,
    /// Variables every request below sees.
    pub vars: Option<Vec<VarDoc>>,
    /// Markdown docs; blank text removes them.
    pub docs: Option<String>,
}

fn edit_error(e: EditError) -> String {
    e.to_string()
}

/// Apply every present section of `p` to `r`, in file order: the URL before the params, so params
/// that equal what the URL derived are left alone. Nothing is saved.
pub(crate) fn apply_request(r: &mut RequestFile, p: &RequestPatch) -> Result<(), String> {
    if let Some(name) = &p.name {
        r.set_name(name.clone());
    }
    if let Some(method) = &p.method {
        r.set_method(normalize_method(method).map_err(|e| e.to_string())?);
    }
    if let Some(url) = &p.url {
        r.set_url(url.clone());
    }
    if let Some(params) = &p.params {
        let current = r.shown_params();
        if !same(&current, params, dto::param)? {
            r.set_params(merge(
                current,
                params,
                |c, d| c.name == d.name && c.kind == kind(&d.kind),
                |d| Param::new(d.name.clone(), d.value.clone(), kind(&d.kind)),
                |c, d| {
                    c.value = d.value.clone();
                    c.enabled = d.enabled;
                },
            ));
        }
    }
    if let Some(headers) = &p.headers {
        let current = r.headers();
        if !same(&current, headers, |kv| Ok(dto::key_value(kv)))? {
            r.set_headers(key_values(current, headers))
                .map_err(edit_error)?;
        }
    }
    if let Some(body) = &p.body {
        let current = r.body();
        if dto::body(current.clone())? != *body {
            r.set_body(body_spec(current, body)?);
        }
    }
    if let Some(auth) = &p.auth {
        let current = r.auth();
        if dto::auth(current)? != *auth {
            r.set_auth(auth_spec(auth)?);
        }
    }
    if let Some(vars) = &p.vars {
        let current = r.vars();
        if !same(&current, vars, dto::var)? {
            r.set_vars(variables(current, vars));
        }
    }
    if let Some(assertions) = &p.assertions {
        let current = r.assertions();
        if !same(&current, assertions, |a| Ok(dto::assertion(a)))? {
            r.set_assertions(assertion_list(current, assertions)?);
        }
    }
    if let Some(settings) = &p.settings {
        let current = r.settings();
        if dto::settings(current)? != *settings {
            r.set_settings(request_settings(current, settings)?);
        }
    }
    if let Some(docs) = &p.docs {
        r.set_docs(docs.clone());
    }
    if let Some(tags) = &p.tags {
        r.set_tags(tags.clone());
    }
    if let Some(seq) = p.seq
        && r.seq() != Some(seq)
    {
        r.set_seq(seq);
    }
    // Last, so an echoed read (url and params unchanged) plus `sync_query` settles in one call.
    match p.sync_query {
        Some(SyncQueryDoc::Url) => r.use_url_query(),
        Some(SyncQueryDoc::Params) => r.use_params_query(),
        None => {}
    }
    Ok(())
}

/// Turn `p` into core's patch. Auth goes to core only when it differs from `current` as a
/// document reads, since a document can omit fields core's auth holds.
pub(crate) fn folder_patch(current: &ConfigDoc, p: &ConfigPatch) -> Result<FolderPatch, String> {
    let mut patch = FolderPatch::new();
    if let Some(headers) = &p.headers {
        patch = patch.headers(headers.iter().map(key_value).collect());
    }
    if let Some(auth) = &p.auth
        && current.auth != *auth
    {
        patch = patch.auth(auth_spec(auth)?);
    }
    if let Some(vars) = &p.vars {
        patch = patch.vars(
            vars.iter()
                .map(|d| {
                    let mut v = Variable::new(d.name.clone(), var_value(d));
                    v.enabled = d.enabled;
                    v
                })
                .collect(),
        );
    }
    if let Some(docs) = &p.docs {
        patch = patch.docs(docs.clone());
    }
    Ok(patch)
}

fn key_value(d: &KeyValueDoc) -> KeyValue {
    let mut kv = KeyValue::new(d.name.clone(), d.value.clone());
    kv.enabled = d.enabled;
    kv
}

/// Whether `current` already reads as `docs`.
fn same<T, D: PartialEq>(
    current: &[T],
    docs: &[D],
    read: impl Fn(&T) -> Result<D, String>,
) -> Result<bool, String> {
    let read: Vec<D> = current.iter().map(read).collect::<Result<_, _>>()?;
    Ok(read == docs)
}

/// Build the new list in `docs` order. Each entry starts from the first unused current entry it
/// matches, keeping what core remembers about that entry, or else from `fresh`; `update` then
/// sets the fields the document carries.
fn merge<T: Clone, D>(
    current: Vec<T>,
    docs: &[D],
    matches: impl Fn(&T, &D) -> bool,
    fresh: impl Fn(&D) -> T,
    update: impl Fn(&mut T, &D),
) -> Vec<T> {
    let mut used = vec![false; current.len()];
    docs.iter()
        .map(|d| {
            let found = (0..current.len()).find(|&i| !used[i] && matches(&current[i], d));
            let mut entry = match found {
                Some(i) => {
                    used[i] = true;
                    current[i].clone()
                }
                None => fresh(d),
            };
            update(&mut entry, d);
            entry
        })
        .collect()
}

fn key_values(current: Vec<KeyValue>, docs: &[KeyValueDoc]) -> Vec<KeyValue> {
    merge(
        current,
        docs,
        |c, d| c.name == d.name,
        |d| KeyValue::new(d.name.clone(), ""),
        |c, d| {
            c.value = d.value.clone();
            c.enabled = d.enabled;
        },
    )
}

fn variables(current: Vec<Variable>, docs: &[VarDoc]) -> Vec<Variable> {
    merge(
        current,
        docs,
        |c, d| c.name == d.name,
        |d| Variable::new(d.name.clone(), VarValue::Text(String::new())),
        |c, d| {
            c.value = var_value(d);
            c.enabled = d.enabled;
        },
    )
}

pub(crate) fn var_value(d: &VarDoc) -> VarValue {
    match &d.value_type {
        Some(kind) => VarValue::Typed {
            kind: kind.clone(),
            data: d.value.clone(),
        },
        None => VarValue::Text(d.value.clone()),
    }
}

fn kind(k: &ParamKindDoc) -> ParamKind {
    match k {
        ParamKindDoc::Query => ParamKind::Query,
        ParamKindDoc::Path => ParamKind::Path,
    }
}

/// Assertions match by expression. Core cannot set a description, so a document that changes
/// one is refused rather than silently dropped.
fn assertion_list(
    current: Vec<Assertion>,
    docs: &[AssertionDoc],
) -> Result<Vec<Assertion>, String> {
    let list = merge(
        current,
        docs,
        |c, d| c.expression() == d.expression,
        |d| Assertion::new(d.expression.clone(), d.operator.clone(), d.value.clone()),
        |c, d| {
            if c.operator() != d.operator || c.value() != d.value.as_deref() {
                c.set_operator(d.operator.clone(), d.value.clone());
            }
            c.set_enabled(d.enabled);
        },
    );
    for (a, d) in list.iter().zip(docs) {
        if a.description() != d.description.as_deref() {
            return Err(format!(
                "assertion `{}`: descriptions cannot be set through MCP",
                d.expression
            ));
        }
    }
    Ok(list)
}

/// The body for `doc`. A list body of the current mode starts from the current entries. The
/// current body's variant tells the request's kind, and a body of the other kind is refused.
fn body_spec(current: BodySpec, doc: &BodyDoc) -> Result<BodySpec, String> {
    match (&current, doc) {
        (BodySpec::Graphql { .. }, BodyDoc::Graphql { .. }) => {}
        (BodySpec::Graphql { .. }, _) => {
            return Err("a graphql request takes only a graphql body".to_owned());
        }
        (_, BodyDoc::Graphql { .. }) => {
            return Err("an http request cannot take a graphql body".to_owned());
        }
        _ => {}
    }
    Ok(match doc {
        BodyDoc::None => BodySpec::None,
        BodyDoc::Json { text } => BodySpec::Json(text.clone()),
        BodyDoc::Xml { text } => BodySpec::Xml(text.clone()),
        BodyDoc::Text { text } => BodySpec::Text(text.clone()),
        BodyDoc::Sparql { text } => BodySpec::Sparql(text.clone()),
        BodyDoc::FormUrlencoded { fields } => {
            let current = match current {
                BodySpec::FormUrlEncoded(f) => f,
                _ => Vec::new(),
            };
            BodySpec::FormUrlEncoded(key_values(current, fields))
        }
        BodyDoc::Multipart { fields } => {
            let current = match current {
                BodySpec::Multipart(f) => f,
                _ => Vec::new(),
            };
            BodySpec::Multipart(multipart(current, fields))
        }
        BodyDoc::File { files } => {
            let current = match current {
                BodySpec::File(f) => f,
                _ => Vec::new(),
            };
            BodySpec::File(file_entries(current, files))
        }
        BodyDoc::Graphql { query, variables } => BodySpec::Graphql {
            query: query.clone(),
            variables: variables.clone(),
        },
        BodyDoc::Unsupported { mode } => {
            return Err(format!("body mode `{mode}` cannot be written through MCP"));
        }
    })
}

fn multipart(current: Vec<MultipartField>, docs: &[MultipartFieldDoc]) -> Vec<MultipartField> {
    merge(
        current,
        docs,
        |c, d| c.name == d.name,
        |d| MultipartField::text(d.name.clone(), ""),
        |c, d| {
            c.value = match &d.value {
                MultipartValueDoc::Text { text } => MultipartValue::Text(text.clone()),
                MultipartValueDoc::Files { paths } => MultipartValue::Files(paths.clone()),
            };
            c.content_type = d.content_type.clone();
            c.enabled = d.enabled;
        },
    )
}

fn file_entries(current: Vec<FileEntry>, docs: &[FileEntryDoc]) -> Vec<FileEntry> {
    merge(
        current,
        docs,
        |c, d| c.path == d.path,
        |d| FileEntry::new(d.path.clone()),
        |c, d| {
            c.content_type = d.content_type.clone();
            c.selected = d.selected;
        },
    )
}

/// Core's spec for `doc`, through core's JSON parser so the CLI and MCP share one reader.
fn auth_spec(doc: &AuthDoc) -> Result<AuthSpec, String> {
    if let AuthDoc::Unsupported { mode } = doc {
        return Err(format!("auth mode `{mode}` cannot be written through MCP"));
    }
    let text = serde_json::to_string(doc).map_err(|e| e.to_string())?;
    apinata_core::parse_auth_json(&text).map_err(|e| e.to_string())
}

fn request_settings(mut s: RequestSettings, doc: &SettingsDoc) -> Result<RequestSettings, String> {
    s.encode_url = doc.encode_url;
    s.timeout = match doc.timeout {
        TimeoutDoc::Inherit => Timeout::Inherit,
        TimeoutDoc::Unlimited => Timeout::Unlimited,
        TimeoutDoc::Millis { ms: 0 } => {
            return Err("a timeout of 0 ms is `unlimited`; use that type".to_owned());
        }
        TimeoutDoc::Millis { ms } => Timeout::Millis(ms),
    };
    s.follow_redirects = doc.follow_redirects;
    s.max_redirects = doc.max_redirects;
    s.forward_authorization_header = doc.forward_authorization_header;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apinata_core::{
        ApiKeyPlacement, OAuth1KeyKind, OAuth1Placement, OAuth1PrivateKey, OAuth1SignatureMethod,
        OAuth1Spec,
    };
    use serde_json::{Value, json};
    use std::fs;

    /// One spec of every mode core's JSON parser takes, each with non-default values.
    fn every_mode() -> Vec<AuthSpec> {
        use apinata_core::{
            ClientAuth, EdgeGridSpec, OAuth2Flow, OAuth2Param, OAuth2ParamPlacement, OAuth2Spec,
            OAuth2Stage, TokenPlacement, TokenSource,
        };
        let mut eg = EdgeGridSpec::default();
        eg.access_token = "at".into();
        eg.max_body_size = Some(f64::NEG_INFINITY);
        let mut o1 = OAuth1Spec::default();
        o1.consumer_key = Some("ck".into());
        o1.signature_method = OAuth1SignatureMethod::Other("HMAC-MD5".into());
        o1.private_key = Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "pem"));
        o1.placement = OAuth1Placement::Query;
        let mut o2 = OAuth2Spec::new(OAuth2Flow::Password);
        o2.access_token_url = "https://id/token".into();
        o2.authorization_url = "kept although the flow does not write it".into();
        o2.client_auth = Some(ClientAuth::Other("jwt".into()));
        o2.username = "ann".into();
        o2.password = "{{pw}}".into();
        o2.token_placement = TokenPlacement::Unset;
        o2.token_source = TokenSource::Other(String::new());
        o2.params = vec![
            OAuth2Param::new(OAuth2Stage::Refresh, "a", "1", OAuth2ParamPlacement::Body),
            OAuth2Param::new(OAuth2Stage::Token, "b", "", OAuth2ParamPlacement::Header),
        ];
        let mut code = OAuth2Spec::new(OAuth2Flow::AuthorizationCode);
        code.client_auth = None;
        code.pkce = true;
        code.token_source = TokenSource::IdToken;
        vec![
            AuthSpec::None,
            AuthSpec::Inherit,
            AuthSpec::Basic {
                username: "u".into(),
                password: "p".into(),
            },
            AuthSpec::Bearer { token: "t".into() },
            AuthSpec::ApiKey {
                key: "k".into(),
                value: "v".into(),
                placement: ApiKeyPlacement::Query,
            },
            AuthSpec::Digest {
                username: "u".into(),
                password: "p".into(),
            },
            AuthSpec::AwsV4 {
                access_key_id: "a".into(),
                secret_access_key: "s".into(),
                session_token: "".into(),
                service: "execute-api".into(),
                region: "".into(),
                profile_name: "dev".into(),
            },
            AuthSpec::OAuth1(o1),
            AuthSpec::OAuth2(o2),
            AuthSpec::OAuth2(code),
            AuthSpec::OAuth2(OAuth2Spec::new(OAuth2Flow::Implicit)),
            AuthSpec::Wsse {
                username: "u".into(),
                password: "p".into(),
            },
            AuthSpec::Ntlm {
                username: "u".into(),
                password: "p".into(),
                domain: "d".into(),
            },
            AuthSpec::EdgeGrid(eg),
        ]
    }

    #[test]
    fn every_mode_read_as_a_document_parses_back_through_core() {
        for spec in every_mode() {
            let text = serde_json::to_string(&dto::auth(spec.clone()).unwrap()).unwrap();
            let back = apinata_core::parse_auth_json(&text).expect(&text);
            assert!(back == spec, "{text}");
            let doc: AuthDoc = serde_json::from_str(&text).unwrap();
            assert!(auth_spec(&doc).unwrap() == spec, "{text}");
        }
    }

    #[test]
    fn wsse_and_edgegrid_round_trip_through_the_document() {
        use apinata_core::EdgeGridSpec;
        let mut eg = EdgeGridSpec::default();
        eg.access_token = "at".into();
        eg.client_token = "ct".into();
        eg.client_secret = "{{cs}}".into();
        eg.nonce = "n".into();
        eg.timestamp = "t".into();
        eg.base_url = "https://b.test".into();
        eg.headers_to_sign = "X-A".into();
        eg.max_body_size = Some(1.5);
        let wsse = AuthSpec::Wsse {
            username: "u".into(),
            password: "{{pw}}".into(),
        };
        for spec in [
            wsse.clone(),
            AuthSpec::EdgeGrid(eg.clone()),
            AuthSpec::EdgeGrid(EdgeGridSpec::default()),
        ] {
            let json = serde_json::to_value(dto::auth(spec.clone()).unwrap()).unwrap();
            let back: AuthDoc = serde_json::from_value(json).unwrap();
            assert_eq!(auth_spec(&back).unwrap(), spec);
        }
        assert_eq!(
            serde_json::to_value(dto::auth(wsse).unwrap()).unwrap(),
            json!({"type": "wsse", "username": "u", "password": "{{pw}}"})
        );
        let json = serde_json::to_value(dto::auth(AuthSpec::EdgeGrid(eg)).unwrap()).unwrap();
        assert_eq!(
            (json["type"].clone(), json["access_token"].clone()),
            (json!("edgegrid"), json!("at"))
        );
        assert_eq!(
            (json["base_url"].clone(), json["max_body_size"].clone()),
            (json!("https://b.test"), json!(1.5))
        );
        let json =
            serde_json::to_value(dto::auth(AuthSpec::EdgeGrid(EdgeGridSpec::default())).unwrap())
                .unwrap();
        assert_eq!(json["max_body_size"], Value::Null);
    }

    #[test]
    fn a_non_finite_max_body_size_round_trips_as_its_javascript_spelling() {
        use apinata_core::EdgeGridSpec;
        for (value, spelling) in [
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (f64::NAN, "NaN"),
        ] {
            let mut eg = EdgeGridSpec::default();
            eg.max_body_size = Some(value);
            let spec = AuthSpec::EdgeGrid(eg);
            let json = serde_json::to_value(dto::auth(spec.clone()).unwrap()).unwrap();
            assert_eq!(json["max_body_size"], json!(spelling));
            let back: AuthDoc = serde_json::from_value(json).unwrap();
            assert!(
                dto::auth(spec.clone()).unwrap() == back,
                "{spelling} reads as a change"
            );
            assert_eq!(auth_spec(&back).unwrap(), spec);
        }
        let other = json!({"type": "edgegrid", "access_token": "", "client_token": "",
            "client_secret": "", "nonce": "", "timestamp": "", "base_url": "",
            "headers_to_sign": "", "max_body_size": "inf"});
        assert!(serde_json::from_value::<AuthDoc>(other).is_err());
    }

    #[test]
    fn ntlm_round_trips_through_the_document() {
        let spec = AuthSpec::Ntlm {
            username: "u".into(),
            password: "{{pw}}".into(),
            domain: "CORP".into(),
        };
        let doc = dto::auth(spec.clone()).unwrap();
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(
            json,
            json!({"type": "ntlm", "username": "u", "password": "{{pw}}", "domain": "CORP"})
        );
        let back: AuthDoc = serde_json::from_value(json).unwrap();
        assert_eq!(auth_spec(&back).unwrap(), spec);
        assert!(doc == back, "reading the document back is not a change");
    }

    #[test]
    fn ntlm_with_an_empty_domain_round_trips_and_a_missing_one_is_refused() {
        let spec = AuthSpec::Ntlm {
            username: String::new(),
            password: String::new(),
            domain: String::new(),
        };
        let json = serde_json::to_value(dto::auth(spec.clone()).unwrap()).unwrap();
        assert_eq!(json["domain"], json!(""));
        let back: AuthDoc = serde_json::from_value(json).unwrap();
        assert_eq!(auth_spec(&back).unwrap(), spec);
        let missing = json!({"type": "ntlm", "username": "u", "password": "p"});
        assert!(serde_json::from_value::<AuthDoc>(missing).is_err());
    }

    #[test]
    fn digest_and_awsv4_round_trip_through_the_document() {
        let specs = [
            AuthSpec::Digest {
                username: "u".into(),
                password: "{{pw}}".into(),
            },
            AuthSpec::AwsV4 {
                access_key_id: "AK".into(),
                secret_access_key: "{{sk}}".into(),
                session_token: String::new(),
                service: "execute-api".into(),
                region: "eu-west-1".into(),
                profile_name: "dev".into(),
            },
        ];
        for spec in specs {
            let doc = dto::auth(spec.clone()).unwrap();
            let json = serde_json::to_value(&doc).unwrap();
            let back: AuthDoc = serde_json::from_value(json).unwrap();
            assert_eq!(auth_spec(&back).unwrap(), spec);
        }
        let json = serde_json::to_value(
            dto::auth(AuthSpec::AwsV4 {
                access_key_id: "AK".into(),
                secret_access_key: "S".into(),
                session_token: "T".into(),
                service: "s".into(),
                region: "r".into(),
                profile_name: "p".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(json["type"], "awsv4");
        assert_eq!(json["profile_name"], "p");
    }

    #[test]
    fn oauth1_round_trips_through_the_document() {
        use apinata_core::{
            OAuth1KeyKind, OAuth1Placement, OAuth1PrivateKey, OAuth1SignatureMethod, OAuth1Spec,
        };
        let mut o = OAuth1Spec::default();
        o.consumer_key = Some("ck".into());
        o.verifier = None;
        o.signature_method = OAuth1SignatureMethod::Other("HMAC-MD5".into());
        o.private_key = Some(OAuth1PrivateKey::new(OAuth1KeyKind::File, "keys/p.pem"));
        o.placement = OAuth1Placement::Query;
        o.include_body_hash = true;
        let spec = AuthSpec::OAuth1(o);
        let json = serde_json::to_value(dto::auth(spec.clone()).unwrap()).unwrap();
        assert_eq!(json["type"], "oauth1");
        assert_eq!(json["signature_method"], "HMAC-MD5");
        assert_eq!(
            json["private_key"],
            json!({"type": "file", "value": "keys/p.pem"})
        );
        assert_eq!(
            (json["verifier"].clone(), json["callback_url"].clone()),
            (Value::Null, json!(""))
        );
        let back: AuthDoc = serde_json::from_value(json).unwrap();
        assert_eq!(auth_spec(&back).unwrap(), spec);
    }

    fn load(text: &str) -> (tempfile::TempDir, RequestFile) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("r.yml");
        fs::write(&path, text).expect("write");
        let r = RequestFile::load(&path).expect("loads");
        (dir, r)
    }

    fn request(http_tail: &str, rest: &str) -> String {
        format!(
            "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: http://localhost/x\n{http_tail}{rest}"
        )
    }

    /// Every section as read, sent back through JSON as an agent would.
    fn echo(r: &RequestFile) -> RequestPatch {
        let mut doc = serde_json::to_value(dto::request_doc(r).expect("reads")).expect("json");
        doc["path"] = json!("r.yml");
        serde_json::from_value(doc).expect("a read document is a valid patch")
    }

    const BASIC_DESCRIBED_UNLIMITED: (&str, &str) = (
        "  auth:\n    type: basic\n    username: u\n    password: \"{{pw}}\"\n",
        "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n      description: must be ok\nsettings:\n  timeout: 0\n",
    );

    #[test]
    fn every_section_sent_back_as_read_leaves_the_file_byte_identical() {
        let files = [
            request(BASIC_DESCRIBED_UNLIMITED.0, BASIC_DESCRIBED_UNLIMITED.1),
            request("  body:\n    type: xml\n    data: <a/>\n", ""),
            request("  body:\n    type: text\n    data: hello\n", ""),
            request(
                "  body:\n    type: file\n    data:\n      - filePath: a.bin\n        contentType: application/octet-stream\n        selected: true\n",
                "",
            ),
            request("  body:\n    type: sparql\n    data: ASK {}\n", ""),
            request(
                "  body:\n    type: msgpack\n    data: gqFhAQ==\n  auth:\n    type: oauth2\n",
                "",
            ),
        ];
        let tags = ["none", "xml", "text", "file", "sparql", "unsupported"];
        for (text, tag) in files.iter().zip(tags) {
            let (dir, mut r) = load(text);
            let patch = echo(&r);
            assert_eq!(
                serde_json::to_value(&patch.body).expect("json")["type"],
                tag
            );
            apply_request(&mut r, &patch).expect("applies");
            assert!(!r.is_dirty(), "{tag}: an echo must change nothing");
            r.save().expect("saves");
            assert_eq!(
                &fs::read_to_string(dir.path().join("r.yml")).expect("read"),
                text
            );
        }
    }

    #[test]
    fn basic_auth_unlimited_timeout_and_a_description_read_as_their_tags() {
        let (_dir, r) = load(&request(
            BASIC_DESCRIBED_UNLIMITED.0,
            BASIC_DESCRIBED_UNLIMITED.1,
        ));
        let patch = echo(&r);
        let auth = serde_json::to_value(patch.auth).expect("json");
        assert_eq!(
            auth,
            json!({"type": "basic", "username": "u", "password": "{{pw}}"})
        );
        let settings = serde_json::to_value(patch.settings).expect("json");
        assert_eq!(settings["timeout"], json!({"type": "unlimited"}));
        let assertions = serde_json::to_value(patch.assertions).expect("json");
        assert_eq!(assertions[0]["description"], "must be ok");
    }

    #[test]
    fn a_matched_header_keeps_keys_mcp_does_not_model() {
        let (dir, mut r) = load(&request(
            "  headers:\n    - name: A\n      value: \"1\"\n    - name: B\n      value: \"2\"\n      description: keep me\n",
            "",
        ));
        let patch = RequestPatch {
            headers: Some(vec![
                KeyValueDoc {
                    name: "B".into(),
                    value: "3".into(),
                    enabled: true,
                },
                KeyValueDoc {
                    name: "A".into(),
                    value: "1".into(),
                    enabled: true,
                },
            ]),
            ..RequestPatch::default()
        };
        apply_request(&mut r, &patch).expect("applies");
        r.save().expect("saves");
        let text = fs::read_to_string(dir.path().join("r.yml")).expect("read");
        assert!(
            text.contains(
                "    - name: B\n      value: \"3\"\n      description: keep me\n    - name: A\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn what_core_cannot_write_is_refused_by_name() {
        let (_dir, mut r) = load(&request("", ""));
        let cases = [
            (
                json!({"path": "r.yml", "body": {"type": "unsupported", "mode": "msgpack"}}),
                "msgpack",
            ),
            (
                json!({"path": "r.yml", "auth": {"type": "unsupported", "mode": "hawk"}}),
                "hawk",
            ),
            (
                json!({"path": "r.yml", "settings": {"encode_url": true, "timeout": {"type": "millis", "ms": 0},
                    "follow_redirects": true, "max_redirects": 5, "forward_authorization_header": true}}),
                "unlimited",
            ),
            (
                json!({"path": "r.yml", "assertions": [{"expression": "res.status", "operator": "eq",
                    "value": "200", "enabled": true, "description": "new"}]}),
                "description",
            ),
        ];
        for (json, needle) in cases {
            let patch: RequestPatch = serde_json::from_value(json).expect("valid patch");
            let err = apply_request(&mut r, &patch).expect_err("refused");
            assert!(err.contains(needle), "{err}");
        }
    }

    #[test]
    fn an_unknown_section_is_a_deserialisation_error() {
        let err = serde_json::from_value::<RequestPatch>(json!({"path": "r.yml", "header": []}))
            .err()
            .expect("refused");
        assert!(err.to_string().contains("header"), "{err}");
    }

    #[test]
    fn an_unknown_key_inside_a_section_is_a_deserialisation_error() {
        let cases = [
            (
                json!({"path": "r.yml", "headers": [{"name": "A", "value": "1", "enabled": true, "description": "d"}]}),
                "description",
            ),
            (
                json!({"path": "r.yml", "body": {"type": "json", "text": "{}", "data": "{}"}}),
                "data",
            ),
            (
                json!({"path": "r.yml", "settings": {"encode_url": true, "timeout": {"type": "inherit"}, "follow_redirects": true,
                "max_redirects": 5, "forward_authorization_header": true, "timeout_ms": 5}}),
                "timeout_ms",
            ),
        ];
        for (json, key) in cases {
            let err = serde_json::from_value::<RequestPatch>(json)
                .err()
                .expect("refused");
            assert!(err.to_string().contains(key), "{key}: {err}");
        }
    }
}
