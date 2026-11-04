//! `parse_postman`: auth, collection variables, docs and protocolProfileBehavior.

use apinata_core::import::{
    ImportNote, ImportedCollection, ImportedItem, ImportedRequestItem, NoteKind, parse_postman,
};
use apinata_core::{ApiKeyPlacement, AuthSpec, RequestSettings, Timeout, VarValue, Variable};

const V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// A v2.1 collection named `c`; `extra` adds top-level members (starting with a comma).
fn parse(extra: &str, items: &str) -> ImportedCollection {
    let json = format!(r#"{{"info": {{"name": "c", "schema": "{V21}"}}{extra}, "item": {items}}}"#);
    parse_postman(json.as_bytes()).expect("parses")
}

fn only_request(c: &ImportedCollection) -> &ImportedRequestItem {
    match c.items.as_slice() {
        [ImportedItem::Request(r)] => r,
        other => panic!("one request expected: {other:?}"),
    }
}

/// One request named `r` with `inside` added to its `request` and `beside` to the item.
fn request(inside: &str, beside: &str) -> ImportedCollection {
    parse(
        "",
        &format!(
            r#"[{{"name": "r", "request": {{"method": "GET", "url": "u"{inside}}}{beside}}}]"#
        ),
    )
}

fn has(notes: &[ImportNote], kind: NoteKind, source: &str) -> bool {
    notes.iter().any(|n| n.kind == kind && n.source == source)
}

#[test]
fn auth_maps_supported_types_and_sets_the_rest_to_none() {
    assert_eq!(
        parse("", "[]").auth,
        AuthSpec::None,
        "a collection without auth has none"
    );
    assert_eq!(
        only_request(&request("", "")).request.auth,
        AuthSpec::Inherit
    );
    let c = parse("", r#"[{"name": "f", "item": []}]"#);
    let ImportedItem::Folder(f) = &c.items[0] else {
        panic!("folder")
    };
    assert_eq!(f.auth, AuthSpec::Inherit);
    let auth = |json: &str| {
        only_request(&request(&format!(r#", "auth": {json}"#), ""))
            .request
            .auth
            .clone()
    };
    assert_eq!(auth(r#"{"type": "noauth"}"#), AuthSpec::None);
    assert_eq!(
        auth(r#"{"type": "basic", "basic": {"username": "u", "password": 7}}"#),
        AuthSpec::Basic {
            username: "u".into(),
            password: "7".into()
        },
        "v2.0 object form"
    );
    assert_eq!(
        auth(
            r#"{"type": "basic", "basic": [{"key": "username", "value": "u"}, {"key": "password", "value": "p"}]}"#
        ),
        AuthSpec::Basic {
            username: "u".into(),
            password: "p".into()
        },
        "v2.1 list form"
    );
    assert_eq!(
        auth(r#"{"type": "bearer", "bearer": [{"key": "token", "value": "{{t}}"}]}"#),
        AuthSpec::Bearer {
            token: "{{t}}".into()
        }
    );
    assert_eq!(
        auth(
            r#"{"type": "apikey", "apikey": [{"key": "key", "value": "k"}, {"key": "value", "value": "v"}]}"#
        ),
        AuthSpec::ApiKey {
            key: "k".into(),
            value: "v".into(),
            placement: ApiKeyPlacement::Header
        }
    );
    for unsupported in ["hawk", "jwt"] {
        let c = request(
            &format!(r#", "auth": {{"type": "{unsupported}", "{unsupported}": []}}"#),
            "",
        );
        let r = only_request(&c);
        assert_eq!(r.request.auth, AuthSpec::None, "{unsupported}");
        assert!(
            has(&r.notes, NoteKind::Converted, "r / request.auth"),
            "{unsupported}: {:?}",
            r.notes
        );
    }
    let c = parse(
        r#", "auth": {"type": "bearer", "bearer": {"token": "t"}}"#,
        "[]",
    );
    assert_eq!(c.auth, AuthSpec::Bearer { token: "t".into() });
}

#[test]
fn oauth1_maps_each_postman_key() {
    use apinata_core::{
        OAuth1KeyKind, OAuth1Placement, OAuth1PrivateKey, OAuth1SignatureMethod, OAuth1Spec,
    };
    let full = r#", "auth": {"type": "oauth1", "oauth1": [{"key": "consumerKey", "value": "ck"},
        {"key": "consumerSecret", "value": "cs"}, {"key": "token", "value": "tk"}, {"key": "tokenSecret", "value": "ts"},
        {"key": "callback", "value": "oob"}, {"key": "verifier", "value": ""}, {"key": "signatureMethod", "value": "RSA-SHA256"},
        {"key": "privateKey", "value": "PEM"}, {"key": "timestamp", "value": "1"}, {"key": "nonce", "value": "n"},
        {"key": "version", "value": ""}, {"key": "realm", "value": "r"}, {"key": "includeBodyHash", "value": true},
        {"key": "addParamsToHeader", "value": false}]}"#;
    let c = request(full, "");
    let r = only_request(&c);
    let s = |v: &str| Some(v.to_owned());
    let mut want = OAuth1Spec::default();
    (
        want.consumer_key,
        want.consumer_secret,
        want.access_token,
        want.access_token_secret,
    ) = (s("ck"), s("cs"), s("tk"), s("ts"));
    (
        want.callback_url,
        want.verifier,
        want.timestamp,
        want.nonce,
        want.realm,
    ) = (s("oob"), None, s("1"), s("n"), s("r"));
    want.signature_method = OAuth1SignatureMethod::RsaSha256;
    want.private_key = Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "PEM"));
    (want.placement, want.include_body_hash) = (OAuth1Placement::Query, true);
    assert_eq!(r.request.auth, AuthSpec::OAuth1(want));
    assert!(r.notes.is_empty(), "{:?}", r.notes);

    let c = request(r#", "auth": {"type": "oauth1", "oauth1": []}"#, "");
    let mut bare = OAuth1Spec::default();
    (
        bare.callback_url,
        bare.verifier,
        bare.timestamp,
        bare.nonce,
        bare.realm,
        bare.private_key,
    ) = (None, None, None, None, None, None);
    assert_eq!(only_request(&c).request.auth, AuthSpec::OAuth1(bare));
}

#[test]
fn collection_variables_are_cleaned_and_stringified() {
    let c = parse(
        r#", "variable": [
        {"key": "base url", "value": "https://x.test"}, {"key": "n", "value": 1.0},
        {"key": "o", "value": {"a": [1, true]}}, {"key": "nil", "value": null},
        {"key": null, "value": null}, {"key": "off", "value": "x", "disabled": true}, {"key": "é😀", "value": "1"}]"#,
        "[]",
    );
    let mut off = Variable::new("off", VarValue::Text("x".into()));
    off.enabled = false;
    assert_eq!(
        c.vars,
        [
            Variable::new("base_url", VarValue::Text("https://x.test".into())),
            Variable::new("n", VarValue::Text("1".into())),
            Variable::new("o", VarValue::Text(r#"{"a":[1,true]}"#.into())),
            Variable::new("nil", VarValue::Text(String::new())),
            off,
            // One `_` per UTF-16 unit, as the JavaScript regex replaces them.
            Variable::new("___", VarValue::Text("1".into())),
        ]
    );
    assert!(
        has(
            &c.notes,
            NoteKind::Converted,
            "collection / variable[0].key"
        ),
        "{:?}",
        c.notes
    );
    assert!(
        has(
            &c.notes,
            NoteKind::Converted,
            "collection / variable[6].key"
        ),
        "{:?}",
        c.notes
    );
    assert_eq!(
        c.notes.len(),
        2,
        "only renamed keys are reported: {:?}",
        c.notes
    );
}

#[test]
fn descriptions_become_docs_and_item_level_ones_are_reported() {
    assert_eq!(parse("", "[]").docs, "");
    let json = format!(
        r#"{{"info": {{"name": "c", "schema": "{V21}", "description": {{"content": "Root", "type": "text/markdown"}}}},
        "item": [{{"name": "f", "description": "Folder", "item": []}},
                 {{"name": "r", "description": "Item level", "request": {{"method": "GET", "url": "u", "description": "Req"}}}}]}}"#
    );
    let c = parse_postman(json.as_bytes()).expect("parses");
    assert_eq!(c.docs, "Root");
    let ImportedItem::Folder(f) = &c.items[0] else {
        panic!("folder")
    };
    assert_eq!(f.docs, "Folder");
    let ImportedItem::Request(r) = &c.items[1] else {
        panic!("request")
    };
    assert_eq!(r.request.docs, "Req");
    assert!(
        has(&r.notes, NoteKind::Dropped, "r / description"),
        "{:?}",
        r.notes
    );
}

#[test]
fn a_fractional_or_oversized_redirect_limit_is_converted_with_a_note() {
    for (limit, kept, detail) in [
        (
            "3.7",
            3,
            "the redirect limit 3.7 is not a whole number; 3 is used",
        ),
        (
            "1e+31",
            u32::MAX,
            "the redirect limit 1e+31 is above the largest supported; 4294967295 is used",
        ),
    ] {
        let c = request(
            "",
            &format!(r#", "protocolProfileBehavior": {{"maxRedirects": {limit}}}"#),
        );
        let r = only_request(&c);
        assert_eq!(r.request.settings.expect("set").max_redirects, kept);
        let details: Vec<_> = r
            .notes
            .iter()
            .filter(|n| {
                n.kind == NoteKind::Converted
                    && n.source == "r / protocolProfileBehavior.maxRedirects"
            })
            .map(|n| n.detail.as_str())
            .collect();
        assert_eq!(details, [detail], "{:?}", r.notes);
    }
}

#[test]
fn protocol_profile_behavior_becomes_settings_where_core_has_the_field() {
    let defaults = only_request(&request("", ""))
        .request
        .settings
        .expect("always set");
    let mut expected = RequestSettings::new();
    expected.forward_authorization_header = false;
    expected.timeout = Timeout::Inherit;
    assert_eq!(defaults, expected);
    // Postman has no per-request timeout, so the request takes the default one.
    assert_eq!(defaults.timeout, Timeout::Inherit);
    let c = request(
        "",
        r#", "protocolProfileBehavior": {"followRedirects": false, "maxRedirects": 3, "disableUrlEncoding": true, "followAuthorizationHeader": true}"#,
    );
    let r = only_request(&c);
    let s = r.request.settings.expect("set");
    assert_eq!(
        (
            s.encode_url,
            s.follow_redirects,
            s.max_redirects,
            s.forward_authorization_header
        ),
        (false, false, 3, true)
    );
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let c = request(
        "",
        r#", "protocolProfileBehavior": {"maxRedirects": "3", "disabledSystemHeaders": {"user-agent": true}, "disableBodyPruning": true}"#,
    );
    let r = only_request(&c);
    assert_eq!(r.request.settings.expect("set").max_redirects, 5);
    for key in [
        "maxRedirects",
        "disabledSystemHeaders",
        "disableBodyPruning",
    ] {
        assert!(
            has(
                &r.notes,
                NoteKind::Dropped,
                &format!("r / protocolProfileBehavior.{key}")
            ),
            "{key}: {:?}",
            r.notes
        );
    }
}

#[test]
fn edgegrid_maps_each_postman_key() {
    use apinata_core::EdgeGridSpec;
    let full = r#", "auth": {"type": "edgegrid", "edgegrid": [{"key": "accessToken", "value": "at"},
        {"key": "clientToken", "value": "ct"}, {"key": "clientSecret", "value": "cs"}, {"key": "nonce", "value": "n"},
        {"key": "timestamp", "value": "t"}, {"key": "baseURL", "value": "https://b.test"},
        {"key": "headersToSign", "value": "X-A"}, {"key": "maxBodySize", "value": 70, "type": "number"}]}"#;
    let c = request(full, "");
    let r = only_request(&c);
    let mut want = EdgeGridSpec::default();
    (want.access_token, want.client_token, want.client_secret) =
        ("at".into(), "ct".into(), "cs".into());
    (want.nonce, want.timestamp, want.base_url) = ("n".into(), "t".into(), "https://b.test".into());
    (want.headers_to_sign, want.max_body_size) = ("X-A".into(), Some(70.0));
    assert_eq!(r.request.auth, AuthSpec::EdgeGrid(want));
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    for (value, want) in [
        (r#""16""#, Some(16.0)),
        (r#""abc""#, None),
        (r#""""#, None),
        ("null", None),
        ("1.5", Some(1.5)),
        ("[7]", Some(7.0)),
        ("[]", Some(0.0)),
        (r#"["8"]"#, Some(8.0)),
        ("[1, 2]", None),
    ] {
        let c = request(
            &format!(
                r#", "auth": {{"type": "edgegrid", "edgegrid": [{{"key": "maxBodySize", "value": {value}}}]}}"#
            ),
            "",
        );
        match &only_request(&c).request.auth {
            AuthSpec::EdgeGrid(s) => assert_eq!(s.max_body_size, want, "{value}"),
            other => panic!("{value}: {other:?}"),
        }
    }
}

#[test]
fn ntlm_maps_username_password_and_domain() {
    let full = r#", "auth": {"type": "ntlm", "ntlm": [{"key": "password", "value": "{{pw}}"},
        {"key": "username", "value": "u"}, {"key": "domain", "value": "CORP"}]}"#;
    let c = request(full, "");
    let r = only_request(&c);
    assert_eq!(
        r.request.auth,
        AuthSpec::Ntlm {
            username: "u".into(),
            password: "{{pw}}".into(),
            domain: "CORP".into()
        }
    );
    assert!(r.notes.is_empty(), "{:?}", r.notes);
}

#[test]
fn ntlm_without_a_domain_has_an_empty_one() {
    let c = request(
        r#", "auth": {"type": "ntlm", "ntlm": [{"key": "username", "value": "u"}]}"#,
        "",
    );
    assert_eq!(
        only_request(&c).request.auth,
        AuthSpec::Ntlm {
            username: "u".into(),
            password: String::new(),
            domain: String::new()
        }
    );
}

#[test]
fn ntlm_workstation_is_dropped_with_a_note() {
    let ws = r#", "auth": {"type": "ntlm", "ntlm": [{"key": "workstation", "value": "my-pc"},
        {"key": "username", "value": "u"}]}"#;
    let c = request(ws, "");
    let r = only_request(&c);
    assert!(matches!(r.request.auth, AuthSpec::Ntlm { .. }));
    let dropped: Vec<&ImportNote> = r
        .notes
        .iter()
        .filter(|n| n.kind == NoteKind::Dropped && n.detail.contains("workstation"))
        .collect();
    assert_eq!(dropped.len(), 1, "{:?}", r.notes);
    assert!(!dropped[0].detail.to_lowercase().contains("bruno"));
    // A blank workstation carries nothing, so it is not worth a note.
    let blank = request(
        r#", "auth": {"type": "ntlm", "ntlm": [{"key": "workstation", "value": ""}]}"#,
        "",
    );
    assert!(only_request(&blank).notes.is_empty());
}

#[test]
fn collection_level_ntlm_is_kept() {
    let c = parse(
        r#", "auth": {"type": "ntlm", "ntlm": [{"key": "domain", "value": "d"}]}"#,
        "[]",
    );
    assert_eq!(
        c.auth,
        AuthSpec::Ntlm {
            username: String::new(),
            password: String::new(),
            domain: "d".into()
        }
    );
}

/// The `auth` of one request whose Postman auth is `kind` with the v2.1 list `entries`, and the
/// request's notes.
fn auth_of(kind: &str, entries: &str) -> (AuthSpec, Vec<ImportNote>) {
    let c = request(
        &format!(r#", "auth": {{"type": "{kind}", "{kind}": {entries}}}"#),
        "",
    );
    let r = only_request(&c);
    (r.request.auth.clone(), r.notes.clone())
}

/// A v2.1 list from `(key, json value)` pairs.
fn list(pairs: &[(&str, &str)]) -> String {
    let items: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!(r#"{{"key": "{k}", "value": {v}}}"#))
        .collect();
    format!("[{}]", items.join(", "))
}

fn oauth2_of(pairs: &[(&str, &str)]) -> (apinata_core::OAuth2Spec, Vec<ImportNote>) {
    match auth_of("oauth2", &list(pairs)) {
        (AuthSpec::OAuth2(spec), notes) => (spec, notes),
        other => panic!("oauth2 expected: {other:?}"),
    }
}

fn noted(notes: &[ImportNote], kind: NoteKind, word: &str) -> bool {
    notes
        .iter()
        .any(|n| n.kind == kind && n.source == "r / request.auth" && n.detail.contains(word))
}

#[test]
fn digest_maps_the_credentials() {
    let (auth, notes) = auth_of(
        "digest",
        &list(&[
            ("username", r#""u""#),
            ("password", r#""{{pw}}""#),
            ("algorithm", r#""md5""#),
            ("realm", r#""""#),
            ("disableRetryRequest", "false"),
        ]),
    );
    assert_eq!(
        auth,
        AuthSpec::Digest {
            username: "u".into(),
            password: "{{pw}}".into()
        }
    );
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn digest_notes_the_server_negotiated_fields_it_drops() {
    let (_, notes) = auth_of(
        "digest",
        &list(&[
            ("username", r#""u""#),
            ("algorithm", r#""SHA-256""#),
            ("realm", r#""r""#),
            ("nonce", r#""n""#),
            ("qop", r#""auth""#),
            ("opaque", r#""o""#),
            ("nonceCount", r#""00000001""#),
            ("clientNonce", r#""c""#),
            ("disableRetryRequest", "true"),
        ]),
    );
    for field in [
        "algorithm",
        "realm",
        "nonce",
        "qop",
        "opaque",
        "nonceCount",
        "clientNonce",
        "disableRetryRequest",
    ] {
        assert!(
            noted(&notes, NoteKind::Dropped, field),
            "{field}: {notes:?}"
        );
    }
    assert_eq!(notes.len(), 8, "{notes:?}");
}

#[test]
fn awsv4_maps_each_postman_key() {
    let (auth, notes) = auth_of(
        "awsv4",
        &list(&[
            ("accessKey", r#""AK""#),
            ("secretKey", r#""SK""#),
            ("sessionToken", r#""ST""#),
            ("service", r#""s3""#),
            ("region", r#""eu-west-1""#),
            ("addAuthDataToQuery", "false"),
        ]),
    );
    assert_eq!(
        auth,
        AuthSpec::AwsV4 {
            access_key_id: "AK".into(),
            secret_access_key: "SK".into(),
            session_token: "ST".into(),
            service: "s3".into(),
            region: "eu-west-1".into(),
            profile_name: String::new(),
        }
    );
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn awsv4_notes_the_query_placement_it_drops() {
    let (_, notes) = auth_of("awsv4", &list(&[("addAuthDataToQuery", "true")]));
    assert!(
        noted(&notes, NoteKind::Dropped, "addAuthDataToQuery"),
        "{notes:?}"
    );
    assert_eq!(notes.len(), 1);
}

#[test]
fn oauth2_client_credentials_maps_each_postman_key() {
    use apinata_core::{ClientAuth, OAuth2Flow};
    let (o, notes) = oauth2_of(&[
        ("grant_type", r#""client_credentials""#),
        ("accessTokenUrl", r#""https://a/token""#),
        ("refreshTokenUrl", r#""https://a/refresh""#),
        ("clientId", r#""id""#),
        ("clientSecret", r#""{{secret}}""#),
        ("scope", r#""read""#),
        ("tokenName", r#""tok""#),
        ("client_authentication", r#""body""#),
        ("addTokenTo", r#""header""#),
        ("headerPrefix", r#""Token""#),
    ]);
    assert_eq!(o.flow, OAuth2Flow::ClientCredentials);
    assert_eq!(
        (o.access_token_url.as_str(), o.refresh_token_url.as_str()),
        ("https://a/token", "https://a/refresh")
    );
    assert_eq!(
        (o.client_id.as_str(), o.client_secret.as_str()),
        ("id", "{{secret}}")
    );
    assert_eq!((o.scope.as_str(), o.token_id.as_str()), ("read", "tok"));
    assert_eq!(o.client_auth, Some(ClientAuth::Body));
    assert_eq!(
        o.token_placement,
        apinata_core::TokenPlacement::Header {
            prefix: "Token".into()
        }
    );
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn oauth2_grants_map_to_flows() {
    use apinata_core::OAuth2Flow::*;
    let flow = |grant: &str| {
        let (o, _) = oauth2_of(&[("grant_type", &format!("\"{grant}\""))]);
        (o.flow, o.pkce)
    };
    assert_eq!(flow("client_credentials"), (ClientCredentials, false));
    assert_eq!(flow("password_credentials"), (Password, false));
    assert_eq!(flow("authorization_code"), (AuthorizationCode, false));
    assert_eq!(
        flow("authorization_code_with_pkce"),
        (AuthorizationCode, true)
    );
    assert_eq!(flow("implicit"), (Implicit, false));
    let (o, notes) = oauth2_of(&[]);
    assert_eq!(o.flow, ClientCredentials, "an absent grant defaults");
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn an_unknown_grant_is_converted_to_client_credentials() {
    let (o, notes) = oauth2_of(&[("grant_type", r#""device_code""#)]);
    assert_eq!(o.flow, apinata_core::OAuth2Flow::ClientCredentials);
    assert!(
        noted(&notes, NoteKind::Converted, "device_code"),
        "{notes:?}"
    );
}

#[test]
fn oauth2_keeps_only_the_grants_own_fields() {
    let (o, notes) = oauth2_of(&[
        ("grant_type", r#""password_credentials""#),
        ("username", r#""alice""#),
        ("password", r#""pw""#),
        ("authUrl", r#""https://a/authorize""#),
        ("redirect_uri", r#""https://cb""#),
    ]);
    assert_eq!((o.username.as_str(), o.password.as_str()), ("alice", "pw"));
    assert!(o.authorization_url.is_empty() && o.callback_url.is_empty());
    assert!(notes.is_empty(), "{notes:?}");

    let (o, _) = oauth2_of(&[
        ("grant_type", r#""authorization_code""#),
        ("authUrl", r#""https://a/authorize""#),
        ("redirect_uri", r#""https://cb""#),
        ("state", r#""xyz""#),
        ("username", r#""alice""#),
    ]);
    assert_eq!(o.authorization_url, "https://a/authorize");
    assert_eq!(
        (o.callback_url.as_str(), o.state.as_str()),
        ("https://cb", "xyz")
    );
    assert!(o.username.is_empty());
}

#[test]
fn a_state_on_a_grant_that_has_none_is_noted() {
    for grant in ["client_credentials", "password_credentials"] {
        let (o, notes) = oauth2_of(&[("grant_type", &format!("\"{grant}\"")), ("state", r#""s""#)]);
        assert!(o.state.is_empty());
        assert!(
            noted(&notes, NoteKind::Dropped, "state"),
            "{grant}: {notes:?}"
        );
        assert!(
            noted(&notes, NoteKind::Dropped, "does not use"),
            "{grant}: {notes:?}"
        );
    }
    let (_, notes) = oauth2_of(&[("grant_type", r#""implicit""#), ("state", r#""s""#)]);
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn the_token_goes_in_the_header_unless_postman_says_query() {
    use apinata_core::TokenPlacement::{Header, Query};
    let place = |pairs: &[(&str, &str)]| oauth2_of(pairs).0.token_placement;
    let bearer = Header {
        prefix: "Bearer".into(),
    };
    assert_eq!(place(&[]), bearer);
    assert_eq!(place(&[("addTokenTo", r#""header""#)]), bearer);
    assert_eq!(place(&[("addTokenTo", r#""bogus""#)]), bearer);
    assert_eq!(
        place(&[("addTokenTo", r#""header""#), ("headerPrefix", r#"" ""#)]),
        bearer,
        "a blank prefix keeps Bearer"
    );
    let query = Query {
        key: "access_token".into(),
    };
    assert_eq!(place(&[("addTokenTo", r#""queryParams""#)]), query);
    assert_eq!(place(&[("addTokenTo", r#""url""#)]), query);
}

#[test]
fn client_authentication_and_token_name_defaults() {
    use apinata_core::ClientAuth;
    let (o, _) = oauth2_of(&[("client_authentication", r#""header""#)]);
    assert_eq!(o.client_auth, Some(ClientAuth::BasicHeader));
    let (o, _) = oauth2_of(&[]);
    assert_eq!(o.client_auth, Some(ClientAuth::BasicHeader));
    assert_eq!(o.token_id, "credentials", "a blank name keeps the default");
}

#[test]
fn additional_params_map_by_stage_and_placement() {
    use apinata_core::{OAuth2Param, OAuth2ParamPlacement as P, OAuth2Stage as S};
    let (o, notes) = oauth2_of(&[
        ("grant_type", r#""authorization_code""#),
        (
            "authRequestParams",
            r#"[{"key": "prompt", "value": "login", "send_as": "request_url", "enabled": true}]"#,
        ),
        (
            "tokenRequestParams",
            r#"[{"key": "audience", "value": "api", "send_as": "request_body"}, {"key": "X-A", "value": "1", "send_as": "request_header"}, {"key": "bare", "value": "2"}]"#,
        ),
        (
            "refreshRequestParams",
            r#"[{"key": "old", "value": "x", "send_as": "request_body", "enabled": false}]"#,
        ),
    ]);
    assert_eq!(
        o.params,
        vec![
            OAuth2Param::new(S::Authorization, "prompt", "login", P::Query),
            OAuth2Param::new(S::Token, "audience", "api", P::Body),
            OAuth2Param::new(S::Token, "X-A", "1", P::Header),
            OAuth2Param::new(S::Token, "bare", "2", P::Header),
        ]
    );
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(noted(&notes, NoteKind::Dropped, "old"), "{notes:?}");
    assert!(noted(&notes, NoteKind::Dropped, "refresh"), "{notes:?}");
}

#[test]
fn oauth2_notes_the_fields_it_drops() {
    let (_, notes) = oauth2_of(&[
        ("challengeAlgorithm", r#""plain""#),
        ("code_verifier", r#""v""#),
        ("useBrowser", "true"),
        ("tokenType", r#""MAC""#),
        ("accessToken", r#""stored""#),
        ("refresh_token", r#""stored-refresh""#),
        ("expires_in", "3600"),
        ("audience", r#""a""#),
        ("resource", r#""r""#),
        ("isAuthInSecondaryWindow", "true"),
    ]);
    for field in [
        "challengeAlgorithm",
        "code_verifier",
        "useBrowser",
        "tokenType",
        "accessToken",
        "refresh_token",
        "expires_in",
        "audience",
        "resource",
        "isAuthInSecondaryWindow",
    ] {
        assert!(
            noted(&notes, NoteKind::Dropped, field),
            "{field}: {notes:?}"
        );
    }
    assert_eq!(notes.len(), 10, "{notes:?}");
}

#[test]
fn oauth2_ordinary_values_give_no_dropped_note() {
    let (_, notes) = oauth2_of(&[
        ("challengeAlgorithm", r#""S256""#),
        ("tokenType", r#""bearer""#),
        ("accessToken", r#""""#),
        ("useBrowser", "false"),
        ("audience", r#""""#),
    ]);
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn no_secret_value_appears_in_any_note() {
    let secrets = [
        "S3CRET-client",
        "S3CRET-pw",
        "S3CRET-aws",
        "S3CRET-tok",
        "S3CRET-stored",
        "S3CRET-refresh",
    ];
    let (_, mut notes) = oauth2_of(&[
        ("grant_type", r#""password_credentials""#),
        ("clientSecret", r#""S3CRET-client""#),
        ("password", r#""S3CRET-pw""#),
        ("state", r#""S3CRET-state""#),
        ("accessToken", r#""S3CRET-stored""#),
        ("refresh_token", r#""S3CRET-refresh""#),
        (
            "refreshRequestParams",
            r#"[{"key": "k", "value": "S3CRET-tok", "send_as": "request_body", "enabled": false}]"#,
        ),
    ]);
    notes.extend(
        auth_of(
            "awsv4",
            &list(&[
                ("secretKey", r#""S3CRET-aws""#),
                ("sessionToken", r#""S3CRET-tok""#),
                ("addAuthDataToQuery", "true"),
            ]),
        )
        .1,
    );
    notes.extend(
        auth_of(
            "digest",
            &list(&[("password", r#""S3CRET-pw""#), ("nonce", r#""S3CRET-tok""#)]),
        )
        .1,
    );
    assert!(!notes.is_empty());
    for n in &notes {
        for s in secrets.iter().chain(&["S3CRET-state"]) {
            assert!(!n.detail.contains(s) && !n.source.contains(s), "{n:?}");
        }
    }
}

#[test]
fn an_implicit_grant_notes_the_token_endpoint_fields_it_does_not_use() {
    let (o, notes) = oauth2_of(&[
        ("grant_type", r#""implicit""#),
        ("clientSecret", r#""S3CRET""#),
        ("accessTokenUrl", r#""https://a/token""#),
        ("refreshTokenUrl", r#""https://a/refresh""#),
    ]);
    assert!(o.access_token_url.is_empty() && o.client_secret.is_empty());
    for field in ["clientSecret", "accessTokenUrl", "refreshTokenUrl"] {
        assert!(
            noted(&notes, NoteKind::Dropped, field),
            "{field}: {notes:?}"
        );
    }
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes.iter().all(|n| !n.detail.contains("S3CRET")));
}

#[test]
fn enabled_params_of_a_stage_the_grant_lacks_are_noted() {
    let (o, notes) = oauth2_of(&[
        ("grant_type", r#""client_credentials""#),
        (
            "authRequestParams",
            r#"[{"key": "prompt", "value": "v", "send_as": "request_url"}, {"key": "off", "value": "v", "enabled": false}]"#,
        ),
    ]);
    assert!(o.params.is_empty());
    assert!(
        noted(&notes, NoteKind::Dropped, "authorization"),
        "{notes:?}"
    );
    assert!(noted(&notes, NoteKind::Dropped, "prompt"), "{notes:?}");
    assert_eq!(
        notes.len(),
        1,
        "a disabled one is not noted twice: {notes:?}"
    );
}

#[test]
fn a_zero_or_blank_expires_in_is_not_noted() {
    for value in [r#""""#, r#""0""#, "0"] {
        let (_, notes) = oauth2_of(&[("expires_in", value)]);
        assert!(notes.is_empty(), "{value}: {notes:?}");
    }
    let (_, notes) = oauth2_of(&[("expires_in", r#""3600""#)]);
    assert!(noted(&notes, NoteKind::Dropped, "expires_in"), "{notes:?}");
}
