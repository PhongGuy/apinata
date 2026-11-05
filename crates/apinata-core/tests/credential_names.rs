//! The credential name rule and the history snapshot mask built on it.

use apinata_core::{
    DraftSnapshot, is_credential_name, mask_credential_query, mask_credential_shape,
    mask_credential_value,
};

#[test]
fn every_listed_header_name_matches_in_any_case() {
    for name in [
        "Authorization",
        "authorization",
        "PROXY-AUTHORIZATION",
        "Cookie",
        "set-cookie",
        "Set-Cookie",
        "X-API-Key",
        "x-api-key",
    ] {
        assert!(is_credential_name(name), "{name}");
    }
}

#[test]
fn a_name_ending_in_a_credential_word_on_a_boundary_matches() {
    for name in [
        "X-Auth-Token",
        "x-session-token",
        "access_token",
        "CLIENT_SECRET",
        "client-secret",
        "api_key",
        "X-Signing-Key",
        "db_password",
        "X-Admin-Password",
        // The bare word is the whole name, so it ends on a boundary too.
        "token",
        "Secret",
        "key",
        "PASSWORD",
        // A camelCase word boundary counts like `-` or `_`.
        "apiKey",
        "accessToken",
        "clientSecret",
        "userPassword",
        // An upper-case run ends where a capitalised or all-caps final word begins.
        "APIKey",
        "ApiKEY",
        "apiKEY",
        "AccessTOKEN",
        "ClientSecret",
    ] {
        assert!(is_credential_name(name), "{name}");
    }
}

#[test]
fn near_misses_do_not_match() {
    for name in [
        "X-Keyboard",
        "monkey",
        "MONKEY",
        "tokenizer",
        "x-token-type",
        "turkey",
        "secretary",
        "passwords",
        "keys",
        "x-key-id",
        "hotkey",
        "Content-Type",
        "x-cookie-policy",
        "authorization-hint",
        "",
        // No visible word boundary, so these stay unmatched to spare `hotkey` and `monkey`.
        "apikey",
        "X-APIKEY",
    ] {
        assert!(!is_credential_name(name), "{name}");
    }
}

#[test]
fn a_named_value_is_hidden_unless_it_only_names_variables() {
    assert_eq!(mask_credential_value("abc123"), "[redacted]");
    assert_eq!(mask_credential_value("Bearer abc123"), "Bearer [redacted]");
    assert_eq!(
        mask_credential_value("basic dXNlcjpwdw=="),
        "basic [redacted]"
    );
    assert_eq!(mask_credential_value("{{token}}"), "{{token}}");
    assert_eq!(
        mask_credential_value("Bearer {{token}}"),
        "Bearer {{token}}"
    );
    assert_eq!(mask_credential_value(""), "");
    assert_eq!(mask_credential_value("pre-{{token}}"), "[redacted]");
}

#[test]
fn a_bearer_or_basic_credential_is_hidden_wherever_it_appears() {
    assert_eq!(
        mask_credential_shape("Bearer eyJhbGciOi.x.y"),
        "Bearer [redacted]"
    );
    assert_eq!(
        mask_credential_shape("basic dXNlcjpwdw=="),
        "basic [redacted]"
    );
    assert_eq!(
        mask_credential_shape("via BEARER abc, then Basic def"),
        "via BEARER [redacted], then Basic [redacted]"
    );
    assert_eq!(
        mask_credential_shape("Bearer {{token}}"),
        "Bearer {{token}}"
    );
    // The scheme is a whole word followed by space; a lone word holds nothing.
    for kept in [
        "Bearer",
        "abasic x",
        "Bearerless",
        "text/plain",
        "basically",
    ] {
        assert_eq!(mask_credential_shape(kept), kept);
    }
}

#[test]
fn query_values_under_credential_names_are_hidden() {
    assert_eq!(
        mask_credential_query("https://h/p?api_key=k1&page=2&access_token=t#frag"),
        "https://h/p?api_key=[redacted]&page=2&access_token=[redacted]#frag"
    );
    assert_eq!(
        mask_credential_query("{{base}}/p?token={{token}}&apiKey="),
        "{{base}}/p?token={{token}}&apiKey="
    );
    assert_eq!(
        mask_credential_query("https://h/token/p?q=1"),
        "https://h/token/p?q=1"
    );
    // A Bearer or Basic credential is hidden under any name, encoded or not.
    assert_eq!(
        mask_credential_query("https://h/?auth=Bearer%20abc123&x=1"),
        "https://h/?auth=[redacted]&x=1"
    );
    assert_eq!(
        mask_credential_query("https://h/?h=basic+dXNlcjpwdw=="),
        "https://h/?h=[redacted]"
    );
    assert_eq!(
        mask_credential_query("https://h/?h=Bearer+{{t}}&q=basically"),
        "https://h/?h=Bearer+{{t}}&q=basically"
    );
    // A percent-encoded name is read decoded.
    assert_eq!(
        mask_credential_query("https://h/?api%5Fkey=k1"),
        "https://h/?api%5Fkey=[redacted]"
    );
}

const SNAP: &str = r#"{
  "info": {"name": "r", "type": "http", "seq": 1},
  "http": {
    "method": "POST",
    "url": "https://h/p?api_key=lit-key-1&page=2",
    "headers": [
      {"name": "X-API-Key", "value": "lit-key-2"},
      {"name": "x-session-token", "value": "{{session}}"},
      {"name": "X-Forward", "value": "Bearer lit-token-3"},
      {"name": "X-Keyboard", "value": "qwerty"}
    ],
    "params": [
      {"name": "api_key", "value": "lit-key-1", "type": "query"},
      {"name": "id", "value": "7", "type": "path"},
      {"name": "page", "value": "2", "type": "query"},
      {"name": "auth", "value": "Basic lit-basic-5", "type": "query"}
    ],
    "body": {
      "type": "form-urlencoded",
      "data": [
        {"name": "password", "value": "lit-pass-4"},
        {"name": "user", "value": "joe"},
        {"name": "note", "value": "Bearer lit-bearer-6"}
      ]
    }
  }
}"#;

#[test]
fn a_snapshot_hides_named_headers_query_and_form_values() {
    let mut snap = DraftSnapshot::from_json(SNAP).unwrap();
    assert!(snap.mask_credentials_by_name());
    let json = snap.to_json();
    for literal in [
        "lit-key-1",
        "lit-key-2",
        "lit-token-3",
        "lit-pass-4",
        "lit-basic-5",
        "lit-bearer-6",
    ] {
        assert!(!json.contains(literal), "{literal} in {json}");
    }
    for kept in ["{{session}}", "qwerty", "\"joe\"", "page=2", "\"7\""] {
        assert!(json.contains(kept), "{kept} gone from {json}");
    }
    assert!(json.contains("Bearer [redacted]"), "{json}");
    // Masking twice changes nothing more.
    let again = json.clone();
    assert!(!snap.mask_credentials_by_name());
    assert_eq!(snap.to_json(), again);
}

#[test]
fn a_snapshot_without_credentials_is_unchanged() {
    let json = r#"{"info":{"type":"http"},"http":{"method":"GET","url":"https://h/?q=1","headers":[{"name":"Accept","value":"*/*"}]}}"#;
    let mut snap = DraftSnapshot::from_json(json).unwrap();
    assert!(!snap.mask_credentials_by_name());
    assert_eq!(snap.to_json(), json);
}

#[test]
fn a_graphql_snapshot_hides_its_named_headers() {
    let json = r#"{"info":{"type":"graphql"},"graphql":{"method":"POST","url":"https://h/q","headers":[{"name":"x-api-key","value":"gq-lit-9"}]}}"#;
    let mut snap = DraftSnapshot::from_json(json).unwrap();
    assert!(snap.mask_credentials_by_name());
    assert!(!snap.to_json().contains("gq-lit-9"));
}
