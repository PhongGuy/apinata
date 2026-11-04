//! The oauth2 auth model as read from disk, its defaults and its redaction.

use apinata_core::{
    AuthSpec, ClientAuth, OAuth2Flow, OAuth2Param, OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage,
    RequestFile, TokenPlacement, TokenSource,
};
use std::path::{Path, PathBuf};

fn corpus(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-11-oauth2-parity")
        .join(rel)
}

fn read(path: &Path) -> AuthSpec {
    RequestFile::load(path).expect("loads").auth()
}

fn read_text(auth: &str) -> AuthSpec {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("r.yml");
    let text = format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: u\n{auth}");
    std::fs::write(&path, text).unwrap();
    read(&path)
}

#[test]
fn new_has_the_reference_apps_fresh_values() {
    let o = OAuth2Spec::new(OAuth2Flow::AuthorizationCode);
    assert_eq!(o.flow, OAuth2Flow::AuthorizationCode);
    assert_eq!(o.client_auth, Some(ClientAuth::Body));
    assert_eq!(o.token_id, "credentials");
    assert_eq!(
        o.token_placement,
        TokenPlacement::Header {
            prefix: "Bearer".into()
        }
    );
    assert_eq!(o.token_source, TokenSource::AccessToken);
    assert_eq!((o.auto_fetch, o.auto_refresh, o.pkce), (true, false, false));
    for s in [
        &o.authorization_url,
        &o.access_token_url,
        &o.refresh_token_url,
        &o.callback_url,
        &o.client_id,
        &o.client_secret,
        &o.username,
        &o.password,
        &o.scope,
        &o.state,
    ] {
        assert_eq!(s, "");
    }
    assert!(o.params.is_empty());
}

#[test]
fn a_full_authorization_code_block_reads_every_field() {
    let mut want = OAuth2Spec::new(OAuth2Flow::AuthorizationCode);
    want.authorization_url = "https://id.example.com/authorize".into();
    want.access_token_url = "https://id.example.com/token".into();
    want.refresh_token_url = "https://id.example.com/refresh".into();
    want.callback_url = "http://127.0.0.1:7777/callback".into();
    want.client_id = "app".into();
    want.client_secret = "{{secret}}".into();
    want.scope = "read write".into();
    want.state = "xyz".into();
    want.pkce = true;
    want.token_id = "main".into();
    want.auto_refresh = true;
    want.params = vec![
        OAuth2Param::new(
            OAuth2Stage::Authorization,
            "prompt",
            "consent",
            OAuth2ParamPlacement::Query,
        ),
        OAuth2Param::new(
            OAuth2Stage::Token,
            "audience",
            "https://api.example.com",
            OAuth2ParamPlacement::Body,
        ),
        OAuth2Param::new(
            OAuth2Stage::Refresh,
            "X-Trace",
            "1",
            OAuth2ParamPlacement::Header,
        ),
    ];
    assert_eq!(
        read(&corpus("normalized/synthetic/auth-code.yml")),
        AuthSpec::OAuth2(want)
    );
}

#[test]
fn client_credentials_reads_basic_header_query_placement_and_id_token() {
    let AuthSpec::OAuth2(o) = read(&corpus("normalized/synthetic/client-credentials.yml")) else {
        panic!("oauth2")
    };
    assert_eq!(o.flow, OAuth2Flow::ClientCredentials);
    assert_eq!(o.client_auth, Some(ClientAuth::BasicHeader));
    assert_eq!(
        o.token_placement,
        TokenPlacement::Query {
            key: "token".into()
        }
    );
    assert_eq!(o.token_source, TokenSource::IdToken);
    assert_eq!(
        (o.auto_fetch, o.auto_refresh, o.pkce),
        (false, false, false)
    );
    assert_eq!(
        (
            o.client_id.as_str(),
            o.client_secret.as_str(),
            o.scope.as_str()
        ),
        ("svc", "s3cret", "jobs")
    );
}

#[test]
fn a_sparse_block_reads_the_reference_readers_defaults() {
    let mut want = OAuth2Spec::new(OAuth2Flow::ClientCredentials);
    want.access_token_url = "https://id.example.com/token".into();
    want.client_auth = None;
    want.token_id = String::new();
    want.token_placement = TokenPlacement::Unset;
    assert_eq!(read(&corpus("sparse/minimal.yml")), AuthSpec::OAuth2(want));
}

#[test]
fn unknown_strings_are_kept_and_unknown_placements_fall_back() {
    let AuthSpec::OAuth2(o) = read(&corpus("sparse/unknown.yml")) else {
        panic!("oauth2")
    };
    assert_eq!(
        o.client_auth,
        Some(ClientAuth::Other("private_key_jwt".into()))
    );
    assert_eq!(o.token_source, TokenSource::Other("refresh_token".into()));
    assert_eq!(o.token_placement, TokenPlacement::Unset);
    assert!(o.pkce, "a pkce mapping without disabled is on");
    assert_eq!(
        o.params,
        [OAuth2Param::new(
            OAuth2Stage::Authorization,
            "resource",
            "https://api.example.com",
            OAuth2ParamPlacement::Header
        )]
    );
}

#[test]
fn only_the_flows_own_keys_are_read() {
    let o = read_text(concat!(
        "  auth:\n    type: oauth2\n    flow: client_credentials\n",
        "    authorizationUrl: https://a\n    pkce: {}\n    resourceOwner:\n      username: u\n",
        "    additionalParameters:\n      authorizationRequest:\n        - name: n\n          placement: query\n",
    ));
    let AuthSpec::OAuth2(o) = o else {
        panic!("oauth2")
    };
    assert_eq!(
        (o.authorization_url.as_str(), o.username.as_str(), o.pkce),
        ("", "", false)
    );
    assert!(o.params.is_empty());
}

#[test]
fn a_missing_or_unknown_flow_stays_unsupported() {
    for auth in [
        "  auth:\n    type: oauth2\n",
        "  auth:\n    type: oauth2\n    flow: device_code\n",
    ] {
        assert_eq!(
            read_text(auth),
            AuthSpec::Unsupported {
                mode: "oauth2".into()
            },
            "{auth}"
        );
    }
}

#[test]
fn debug_hides_the_client_secret_username_password_and_parameter_values() {
    let mut o = OAuth2Spec::new(OAuth2Flow::Password);
    o.client_id = "visible-client".into();
    o.client_secret = "cs-SECRET-1".into();
    o.password = "pw-SECRET-2".into();
    o.username = "un-SECRET-4".into();
    o.params = vec![OAuth2Param::new(
        OAuth2Stage::Token,
        "audience",
        "pv-SECRET-3",
        OAuth2ParamPlacement::Body,
    )];
    let shown = format!("{:?} {:?}", AuthSpec::OAuth2(o.clone()), o.params[0]);
    for secret in ["cs-SECRET-1", "pw-SECRET-2", "pv-SECRET-3", "un-SECRET-4"] {
        assert!(!shown.contains(secret), "{secret} leaked: {shown}");
    }
    assert!(
        shown.contains("visible-client") && shown.contains("audience"),
        "{shown}"
    );
}
