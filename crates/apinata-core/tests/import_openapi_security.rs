//! Security schemes become auth with secret placeholders.

use crate::openapi_support;

use apinata_core::import::{ImportBundle, ImportedRequestItem, NoteKind, write_collection};
use apinata_core::{ApiKeyPlacement, AuthSpec, MemoryStore, OAuth2Flow, OAuth2Spec};
use openapi_support::{import, request};
use serde_json::{Value, json};
use tempfile::TempDir;

/// The security fixture with its paths replaced by `/a: ops` and its root security by `root`.
fn doc(root: Option<Value>, ops: Value) -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/openapi/apinata/security.yaml"
    );
    let text = std::fs::read_to_string(path).expect("read");
    let mut doc: Value = yaml_serde::from_str(&text).expect("parses");
    doc["paths"] = json!({ "/a": ops });
    match root {
        Some(security) => doc["security"] = security,
        None => {
            doc.as_object_mut().expect("object").remove("security");
        }
    }
    doc
}

fn auth_of(bundle: &ImportBundle, name: &str) -> AuthSpec {
    request(&bundle.collection, name).request.auth.clone()
}

fn noted(r: &ImportedRequestItem, kind: NoteKind, part: &str) -> bool {
    r.notes
        .iter()
        .any(|n| n.kind == kind && n.detail.contains(part))
}

fn api_key(key: &str, placement: ApiKeyPlacement) -> AuthSpec {
    let value = "{{apiKey}}".into();
    AuthSpec::ApiKey {
        key: key.into(),
        value,
        placement,
    }
}

#[test]
fn http_schemes_match_case_insensitively() {
    let ops = json!({"get": {"summary": "G", "security": [{"basicAuth": []}]}});
    let b = import(&doc(Some(json!([{"bearerAuth": []}])), ops));
    assert_eq!(
        b.collection.auth,
        AuthSpec::Bearer {
            token: "{{token}}".into()
        }
    );
    let basic = AuthSpec::Basic {
        username: "{{username}}".into(),
        password: "{{password}}".into(),
    };
    assert_eq!(auth_of(&b, "G"), basic);
}

#[test]
fn empty_operation_security_is_none() {
    let ops = json!({"get": {"summary": "Open", "security": []}, "put": {"summary": "Inherited"}});
    let b = import(&doc(Some(json!([{"bearerAuth": []}])), ops));
    assert_eq!(auth_of(&b, "Open"), AuthSpec::None);
    assert_eq!(auth_of(&b, "Inherited"), AuthSpec::Inherit);
    assert_eq!(
        import(&doc(None, json!({"get": {"summary": "G"}})))
            .collection
            .auth,
        AuthSpec::None
    );
}

#[test]
fn an_api_key_is_auth_only_and_cookie_keys_are_dropped() {
    let b = import(&doc(
        None,
        json!({
            "get": {"summary": "H", "security": [{"keyHeader": []}]},
            "put": {"summary": "Q", "security": [{"keyQuery": []}]},
            "post": {"summary": "C", "security": [{"keyCookie": []}]}
        }),
    ));
    assert_eq!(
        auth_of(&b, "H"),
        api_key("X-API-Key", ApiKeyPlacement::Header)
    );
    assert_eq!(auth_of(&b, "Q"), api_key("api_key", ApiKeyPlacement::Query));
    let h = request(&b.collection, "H");
    assert!(h.request.headers.is_empty() && h.request.params.is_empty());
    let c = request(&b.collection, "C");
    assert_eq!(c.request.auth, AuthSpec::None);
    assert!(noted(c, NoteKind::Dropped, "`keyCookie`"), "{:?}", c.notes);
}

#[test]
fn unsupported_schemes_become_none_with_a_note() {
    let b = import(&doc(
        None,
        json!({"get": {"summary": "G", "security": [{"missing": []}]}}),
    ));
    let r = request(&b.collection, "G");
    assert_eq!(r.request.auth, AuthSpec::None);
    assert!(noted(r, NoteKind::Dropped, "`missing`"), "{:?}", r.notes);
}

#[test]
fn only_the_first_requirement_is_used() {
    let b = import(&doc(
        None,
        json!({
            "get": {"summary": "Alt", "security": [{"keyHeader": []}, {"bearerAuth": []}]},
            "put": {"summary": "Optional", "security": [{}, {"bearerAuth": []}]}
        }),
    ));
    let alt = request(&b.collection, "Alt");
    assert!(matches!(alt.request.auth, AuthSpec::ApiKey { .. }));
    let noted = alt
        .notes
        .iter()
        .any(|n| n.kind == NoteKind::Converted && n.source == "paths./a.get.security");
    assert!(noted, "{:?}", alt.notes);
    assert_eq!(auth_of(&b, "Optional"), AuthSpec::None);
}

#[test]
fn auth_placeholders_become_secret_variables() {
    let ops = json!({"get": {"summary": "G", "security": [{"basicAuth": []}]}});
    let mut two_servers = doc(Some(json!([{"bearerAuth": []}])), ops.clone());
    two_servers["servers"] = json!([{"url": "https://a.test", "description": "A"}, {"url": "https://b.test", "description": "B"}]);
    let b = import(&two_servers);
    assert_eq!(b.environments.len(), 2);
    for env in &b.environments {
        let secrets: Vec<&str> = env
            .variables
            .iter()
            .filter(|v| v.secret)
            .map(|v| v.name.as_str())
            .collect();
        assert_eq!(secrets, ["token", "username", "password"], "{}", env.name);
    }
    let mut no_servers = doc(Some(json!([{"bearerAuth": []}])), ops);
    no_servers
        .as_object_mut()
        .expect("object")
        .remove("servers");
    let b = import(&no_servers);
    assert_eq!(b.environments.len(), 1);
    assert_eq!(b.environments[0].name, "Environment");
    let parent = TempDir::new().expect("tempdir");
    let (_, report) = write_collection(
        parent.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .expect("writes");
    let wanted =
        ["token", "username", "password"].map(|v| ("Environment".to_owned(), v.to_owned()));
    assert_eq!(report.secrets_to_enter, wanted);
}

#[test]
fn swagger_2_security_definitions_map_the_same_way() {
    let doc = json!({"swagger": "2.0", "host": "x.test", "security": [{"key": []}],
        "securityDefinitions": {"key": {"type": "apiKey", "in": "header", "name": "X-Key"}, "basic": {"type": "basic"}, "oauth": {"type": "oauth2", "flow": "implicit"}},
        "paths": {"/a": {"get": {"summary": "B", "security": [{"basic": []}]}, "put": {"summary": "O", "security": [{"oauth": []}]}}}});
    let b = import(&doc);
    assert!(matches!(b.collection.auth, AuthSpec::ApiKey { .. }));
    assert!(matches!(auth_of(&b, "B"), AuthSpec::Basic { .. }));
    assert!(matches!(auth_of(&b, "O"), AuthSpec::OAuth2(_)));
}

#[test]
fn malformed_operation_security_is_none_with_one_note() {
    for security in [json!({"bearerAuth": []}), json!(["bearerAuth"])] {
        let ops = json!({"get": {"summary": "G", "security": security}});
        let b = import(&doc(None, ops));
        let r = request(&b.collection, "G");
        assert_eq!(r.request.auth, AuthSpec::None, "{security}");
        let dropped: Vec<_> = r
            .notes
            .iter()
            .filter(|n| n.kind == NoteKind::Dropped)
            .collect();
        assert_eq!(dropped.len(), 1, "{security}: {:?}", r.notes);
        assert_eq!(dropped[0].source, "paths./a.get.security");
    }
}

#[test]
fn scheme_notes_name_the_missing_field() {
    let ops = json!({
        "get": {"summary": "Untyped", "security": [{"untyped": []}]},
        "put": {"summary": "Nameless", "security": [{"nameless": []}]}
    });
    let mut d = doc(None, ops);
    let schemes = &mut d["components"]["securitySchemes"];
    schemes["untyped"] = json!({"in": "header", "name": "K"});
    schemes["nameless"] = json!({"type": "apiKey", "in": "header", "name": ""});
    let b = import(&d);
    let untyped = request(&b.collection, "Untyped");
    assert_eq!(untyped.request.auth, AuthSpec::None);
    assert!(
        noted(untyped, NoteKind::Dropped, "`untyped` has no `type`"),
        "{:?}",
        untyped.notes
    );
    assert!(untyped.notes.iter().all(|n| !n.detail.contains("``")));
    let nameless = request(&b.collection, "Nameless");
    assert_eq!(nameless.request.auth, AuthSpec::None);
    assert!(
        noted(
            nameless,
            NoteKind::Dropped,
            "`nameless` is an `apiKey` scheme with no `name`"
        ),
        "{:?}",
        nameless.notes
    );
    assert!(
        nameless
            .notes
            .iter()
            .all(|n| !n.detail.contains("in `header`"))
    );
}

/// The security fixture's paths replaced by one operation `G` requiring `req`, with the extra
/// scheme `extra` registered as `s`.
fn with_scheme(extra: Value, req: Value) -> ImportBundle {
    let ops = json!({"get": {"summary": "G", "security": [{"s": req}]}});
    let mut d = doc(None, ops);
    d["components"]["securitySchemes"]["s"] = extra;
    import(&d)
}

fn oauth_of(b: &ImportBundle) -> OAuth2Spec {
    match auth_of(b, "G") {
        AuthSpec::OAuth2(o) => o,
        other => panic!("not oauth2: {other:?}"),
    }
}

fn oauth3(flows: Value, req: Value) -> ImportBundle {
    with_scheme(json!({"type": "oauth2", "flows": flows}), req)
}

#[test]
fn digest_maps_to_digest_with_placeholders() {
    // Both spellings match, and the fixture's own lowercase `digest` scheme too.
    for scheme in ["Digest", "DIGEST", "digest"] {
        let b = with_scheme(json!({"type": "http", "scheme": scheme}), json!([]));
        assert_eq!(
            auth_of(&b, "G"),
            AuthSpec::Digest {
                username: "{{username}}".into(),
                password: "{{password}}".into()
            },
            "{scheme}"
        );
        let r = request(&b.collection, "G");
        assert!(
            r.notes.iter().all(|n| n.kind != NoteKind::Converted),
            "{:?}",
            r.notes
        );
    }
    let mut d = doc(
        None,
        json!({"get": {"summary": "G", "security": [{"digestAuth": []}]}}),
    );
    d["servers"] = json!([]);
    let b = import(&d);
    assert!(matches!(auth_of(&b, "G"), AuthSpec::Digest { .. }));
    let secrets: Vec<&str> = b.environments[0]
        .variables
        .iter()
        .filter(|v| v.secret)
        .map(|v| v.name.as_str())
        .collect();
    assert_eq!(secrets, ["username", "password"]);
    let parent = TempDir::new().expect("tempdir");
    let (_, report) = write_collection(
        parent.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .expect("writes");
    let wanted = ["username", "password"].map(|v| ("Environment".to_owned(), v.to_owned()));
    assert_eq!(report.secrets_to_enter, wanted);
}

#[test]
fn client_credentials_flow() {
    let flows = json!({"clientCredentials": {"tokenUrl": "https://a.test/t", "refreshUrl": "https://a.test/r", "scopes": {"read": "r", "write": "w"}}});
    let b = oauth3(flows, json!(["read"]));
    let o = oauth_of(&b);
    assert_eq!(o.flow, OAuth2Flow::ClientCredentials);
    assert_eq!(o.access_token_url, "https://a.test/t");
    assert_eq!(o.refresh_token_url, "https://a.test/r");
    assert_eq!(o.scope, "read");
    assert_eq!(o.client_id, "{{clientId}}");
    assert_eq!(o.client_secret, "{{clientSecret}}");
    assert!(!o.pkce);
}

#[test]
fn password_flow() {
    let flows = json!({"password": {"tokenUrl": "https://a.test/t", "scopes": {}}});
    let o = oauth_of(&oauth3(flows, json!([])));
    assert_eq!(o.flow, OAuth2Flow::Password);
    assert_eq!(o.username, "{{username}}");
    assert_eq!(o.password, "{{password}}");
    assert_eq!(o.client_id, "{{clientId}}");
    assert_eq!(o.client_secret, "{{clientSecret}}");
    assert_eq!(o.scope, "");
}

#[test]
fn authorization_code_flow_uses_pkce() {
    let flows = json!({"authorizationCode": {"authorizationUrl": "https://a.test/a", "tokenUrl": "https://a.test/t", "scopes": {}}});
    let o = oauth_of(&oauth3(flows, json!([])));
    assert_eq!(o.flow, OAuth2Flow::AuthorizationCode);
    assert!(o.pkce);
    assert_eq!(o.authorization_url, "https://a.test/a");
    assert_eq!(o.access_token_url, "https://a.test/t");
    assert_eq!(o.client_secret, "{{clientSecret}}");
}

#[test]
fn implicit_flow_has_no_secret_or_token_url() {
    let flows = json!({"implicit": {"authorizationUrl": "https://a.test/a", "scopes": {}}});
    let o = oauth_of(&oauth3(flows, json!([])));
    assert_eq!(o.flow, OAuth2Flow::Implicit);
    assert_eq!(o.authorization_url, "https://a.test/a");
    assert_eq!(o.client_id, "{{clientId}}");
    assert_eq!(o.client_secret, "");
    assert_eq!(o.access_token_url, "");
}

#[test]
fn open_id_connect_is_authorization_code_with_a_note() {
    let b = with_scheme(
        json!({"type": "openIdConnect", "openIdConnectUrl": "https://a.test/.well-known"}),
        json!(["openid", "bad scope"]),
    );
    let o = oauth_of(&b);
    assert_eq!(o.flow, OAuth2Flow::AuthorizationCode);
    assert!(o.pkce);
    assert_eq!(o.client_id, "{{clientId}}");
    assert_eq!(o.authorization_url, "");
    assert_eq!(o.access_token_url, "");
    assert_eq!(o.scope, "openid");
    let r = request(&b.collection, "G");
    assert!(
        noted(r, NoteKind::Converted, "https://a.test/.well-known"),
        "{:?}",
        r.notes
    );
    assert!(noted(r, NoteKind::Dropped, "bad scope"), "{:?}", r.notes);
}

#[test]
fn several_flows_pick_by_priority_and_note_the_skipped() {
    let flows = json!({
        "implicit": {"authorizationUrl": "https://a.test/a", "scopes": {}},
        "password": {"tokenUrl": "https://a.test/p", "scopes": {}},
        "clientCredentials": {"tokenUrl": "https://a.test/c", "scopes": {}}
    });
    let b = oauth3(flows, json!([]));
    let o = oauth_of(&b);
    assert_eq!(o.flow, OAuth2Flow::ClientCredentials);
    let r = request(&b.collection, "G");
    assert!(noted(r, NoteKind::Converted, "`password`"), "{:?}", r.notes);
    assert!(noted(r, NoteKind::Converted, "`implicit`"), "{:?}", r.notes);
}

#[test]
fn scopes_come_from_the_requirement_else_all_flow_scopes() {
    let flows = json!({"clientCredentials": {"tokenUrl": "https://a.test/t", "scopes": {"read": "r", "write": "w", "bad one": "x"}}});
    let b = oauth3(flows.clone(), json!(["write", "other"]));
    assert_eq!(oauth_of(&b).scope, "write other");
    let b = oauth3(flows, json!([]));
    assert_eq!(oauth_of(&b).scope, "read write");
    let r = request(&b.collection, "G");
    assert!(noted(r, NoteKind::Dropped, "bad one"), "{:?}", r.notes);
}

#[test]
fn swagger_2_flows_map_application_and_access_code() {
    let doc = |scheme: Value| {
        json!({"swagger": "2.0", "host": "x.test", "security": [{"s": ["read"]}],
            "securityDefinitions": {"s": scheme}, "paths": {}})
    };
    let app = json!({"type": "oauth2", "flow": "application", "tokenUrl": "https://a.test/t", "scopes": {"read": "r"}});
    let b = import(&doc(app));
    let AuthSpec::OAuth2(o) = &b.collection.auth else {
        panic!("{:?}", b.collection.auth)
    };
    assert_eq!(o.flow, OAuth2Flow::ClientCredentials);
    assert_eq!(o.access_token_url, "https://a.test/t");
    assert_eq!(o.scope, "read");
    let code = json!({"type": "oauth2", "flow": "accessCode", "authorizationUrl": "https://a.test/a", "tokenUrl": "https://a.test/t", "scopes": {}});
    let b = import(&doc(code));
    let AuthSpec::OAuth2(o) = &b.collection.auth else {
        panic!("{:?}", b.collection.auth)
    };
    assert_eq!(o.flow, OAuth2Flow::AuthorizationCode);
    assert_eq!(o.authorization_url, "https://a.test/a");
}

#[test]
fn oauth_operation_security_overrides_global_and_empty_is_none() {
    let ops = json!({
        "get": {"summary": "G", "security": [{"oauth": ["read"]}]},
        "put": {"summary": "Open", "security": []}
    });
    let b = import(&doc(Some(json!([{"bearerAuth": []}])), ops));
    assert!(matches!(b.collection.auth, AuthSpec::Bearer { .. }));
    assert!(matches!(auth_of(&b, "G"), AuthSpec::OAuth2(_)));
    assert_eq!(auth_of(&b, "Open"), AuthSpec::None);
}

#[test]
fn oauth_placeholders_reach_the_environment() {
    let flows = json!({"clientCredentials": {"tokenUrl": "https://a.test/t", "scopes": {}}});
    let b = oauth3(flows, json!([]));
    let env = &b.environments[0];
    let kinds: Vec<(&str, bool)> = env
        .variables
        .iter()
        .map(|v| (v.name.as_str(), v.secret))
        .filter(|(name, _)| *name != "baseUrl")
        .collect();
    assert_eq!(kinds, [("clientId", false), ("clientSecret", true)]);
    let parent = TempDir::new().expect("tempdir");
    let (_, report) = write_collection(
        parent.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .expect("writes");
    assert!(
        report
            .secrets_to_enter
            .iter()
            .all(|(_, name)| name != "clientId"),
        "{:?}",
        report.secrets_to_enter
    );
    assert!(
        report
            .secrets_to_enter
            .iter()
            .any(|(_, name)| name == "clientSecret")
    );
}

#[test]
fn authorization_code_beats_client_credentials() {
    let flows = json!({
        "clientCredentials": {"tokenUrl": "https://a.test/c", "scopes": {}},
        "authorizationCode": {"authorizationUrl": "https://a.test/a", "tokenUrl": "https://a.test/t", "scopes": {}},
        "implicit": {"authorizationUrl": "https://a.test/i", "scopes": {}}
    });
    let b = oauth3(flows, json!([]));
    assert_eq!(oauth_of(&b).flow, OAuth2Flow::AuthorizationCode);
    let r = request(&b.collection, "G");
    assert!(
        noted(r, NoteKind::Converted, "`clientCredentials`"),
        "{:?}",
        r.notes
    );
    assert!(noted(r, NoteKind::Converted, "`implicit`"), "{:?}", r.notes);
}

#[test]
fn swagger_2_password_and_implicit_flows_are_exact() {
    for (name, flow) in [
        ("password", OAuth2Flow::Password),
        ("implicit", OAuth2Flow::Implicit),
    ] {
        let doc = json!({"swagger": "2.0", "host": "x.test", "security": [{"s": []}],
            "securityDefinitions": {"s": {"type": "oauth2", "flow": name, "authorizationUrl": "https://a.test/a", "tokenUrl": "https://a.test/t", "scopes": {}}},
            "paths": {}});
        let b = import(&doc);
        let AuthSpec::OAuth2(o) = &b.collection.auth else {
            panic!("{name}: {:?}", b.collection.auth)
        };
        assert_eq!(o.flow, flow, "{name}");
    }
}
