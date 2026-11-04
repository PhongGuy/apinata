//! `parse_auth_json`: the one JSON auth shape the CLI and MCP both write through, and its
//! errors, which never repeat a value from the input.

use apinata_core::{
    ApiKeyPlacement, AuthSpec, ClientAuth, EdgeGridSpec, OAuth2Flow, OAuth2Param,
    OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage, TokenPlacement, TokenSource, parse_auth_json,
};

#[test]
fn credential_modes_parse_to_their_specs() {
    let cases = [
        (r#"{"type":"none"}"#, AuthSpec::None),
        (r#"{"type":"inherit"}"#, AuthSpec::Inherit),
        (
            r#"{"type":"bearer","token":"{{t}}"}"#,
            AuthSpec::Bearer {
                token: "{{t}}".into(),
            },
        ),
        (
            r#"{"type":"apikey","key":"X-Key","value":"v","placement":"query"}"#,
            AuthSpec::ApiKey {
                key: "X-Key".into(),
                value: "v".into(),
                placement: ApiKeyPlacement::Query,
            },
        ),
        (
            r#"{"type":"ntlm","username":"u","password":"p","domain":""}"#,
            AuthSpec::Ntlm {
                username: "u".into(),
                password: "p".into(),
                domain: String::new(),
            },
        ),
    ];
    for (json, want) in cases {
        assert_eq!(parse_auth_json(json).expect(json), want, "{json}");
    }
}

#[test]
fn edgegrid_takes_the_javascript_spelling_of_infinity() {
    let json = r#"{"type":"edgegrid","access_token":"a","client_token":"c","client_secret":"s",
        "nonce":"","timestamp":"","base_url":"","headers_to_sign":"","max_body_size":"Infinity"}"#;
    let AuthSpec::EdgeGrid(e) = parse_auth_json(json).expect("parses") else {
        panic!("edgegrid")
    };
    let mut want = EdgeGridSpec::default();
    want.access_token = "a".into();
    want.client_token = "c".into();
    want.client_secret = "s".into();
    want.max_body_size = Some(f64::INFINITY);
    assert!(AuthSpec::EdgeGrid(e) == AuthSpec::EdgeGrid(want));
}

#[test]
fn oauth2_parses_every_field_in_the_files_spellings() {
    let json = r#"{"type":"oauth2","flow":"authorization_code",
        "authorization_url":"https://id/auth","access_token_url":"https://id/token",
        "refresh_token_url":"","callback_url":"http://localhost/cb","client_id":"app",
        "client_secret":"{{cs}}","client_auth":"basic_auth_header","username":"","password":"",
        "scope":"read","state":"st","pkce":true,"token_id":"credentials",
        "token_placement":{"type":"query","key":"access_token"},"token_source":"id_token",
        "auto_fetch":false,"auto_refresh":true,
        "params":[{"stage":"authorizationRequest","name":"aud","value":"api","placement":"query"}]}"#;
    let mut want = OAuth2Spec::new(OAuth2Flow::AuthorizationCode);
    want.authorization_url = "https://id/auth".into();
    want.access_token_url = "https://id/token".into();
    want.callback_url = "http://localhost/cb".into();
    want.client_id = "app".into();
    want.client_secret = "{{cs}}".into();
    want.client_auth = Some(ClientAuth::BasicHeader);
    want.scope = "read".into();
    want.state = "st".into();
    want.pkce = true;
    want.token_placement = TokenPlacement::Query {
        key: "access_token".into(),
    };
    want.token_source = TokenSource::IdToken;
    want.auto_fetch = false;
    want.auto_refresh = true;
    want.params = vec![OAuth2Param::new(
        OAuth2Stage::Authorization,
        "aud",
        "api",
        OAuth2ParamPlacement::Query,
    )];
    assert_eq!(
        parse_auth_json(json).expect("parses"),
        AuthSpec::OAuth2(want)
    );
}

#[test]
fn an_unknown_mode_is_named() {
    let err = parse_auth_json(r#"{"type":"hawk","id":"x"}"#).expect_err("refused");
    assert!(err.to_string().contains("hawk"), "{err}");
}

#[test]
fn an_unknown_or_missing_field_is_named() {
    let err = parse_auth_json(r#"{"type":"bearer","token":"t","tokn":"t"}"#).expect_err("unknown");
    assert!(err.to_string().contains("tokn"), "{err}");
    let err = parse_auth_json(r#"{"type":"basic","username":"u"}"#).expect_err("missing");
    assert!(err.to_string().contains("password"), "{err}");
}

#[test]
fn errors_never_repeat_a_value_from_the_input() {
    let secret = "s3cr3t-Value-9";
    let cases = [
        // A string where a bool belongs.
        format!(
            r#"{{"type":"oauth2","flow":"implicit","authorization_url":"","access_token_url":"",
            "refresh_token_url":"","callback_url":"","client_id":"","client_secret":"",
            "username":"","password":"","scope":"","state":"","pkce":"{secret}","token_id":"",
            "token_placement":{{"type":"unset"}},"token_source":"access_token",
            "auto_fetch":true,"auto_refresh":false,"params":[]}}"#
        ),
        // A string where an enumerated value belongs.
        format!(r#"{{"type":"apikey","key":"k","value":"v","placement":"{secret}"}}"#),
        // A string where a number belongs.
        format!(
            r#"{{"type":"edgegrid","access_token":"","client_token":"","client_secret":"",
            "nonce":"","timestamp":"","base_url":"","headers_to_sign":"","max_body_size":"{secret}"}}"#
        ),
        // Malformed documents.
        format!(r#"{{"type":"bearer","token":"{secret}""#),
        format!(r#"{{"type":"bearer","token":"{secret}",}}"#),
        format!(r#"{{"type":"bearer","token":{secret}}}"#),
        format!(r#"["{secret}"]"#),
    ];
    for json in &cases {
        let err = parse_auth_json(json).expect_err(json);
        let text = err.to_string();
        assert!(!text.contains(secret), "{text}");
        assert!(!format!("{err:?}").contains(secret), "{err:?}");
        assert!(!text.contains("line 0"), "{text}");
        if matches!(err, apinata_core::AuthJsonError::Malformed { .. }) {
            assert!(
                text.contains("line"),
                "a malformed document keeps its position: {text}"
            );
        }
    }
    // The three malformed documents above are reported as such, with their position.
    for json in &cases[3..6] {
        let text = parse_auth_json(json).expect_err(json).to_string();
        assert!(text.contains("not well-formed (line "), "{text}");
    }
}

/// An implicit-flow oauth2 document with `field` (a top-level key) replaced by `value`.
fn oauth2_with(field: &str, value: serde_json::Value) -> String {
    let mut doc = serde_json::json!({"type": "oauth2", "flow": "implicit",
        "authorization_url": "", "access_token_url": "", "refresh_token_url": "",
        "callback_url": "", "client_id": "", "client_secret": "", "username": "",
        "password": "", "scope": "", "state": "", "pkce": false, "token_id": "",
        "token_placement": {"type": "unset"}, "token_source": "access_token",
        "auto_fetch": true, "auto_refresh": false,
        "params": [{"stage": "authorizationRequest", "name": "a", "value": "1",
            "placement": "query"}]});
    doc[field] = value;
    doc.to_string()
}

#[test]
fn a_bad_value_names_its_field_path_and_no_unknown_position() {
    let secret = "s3cr3t-Value-9";
    let cases = [
        (oauth2_with("pkce", serde_json::json!(secret)), "`pkce`"),
        (
            oauth2_with("token_placement", serde_json::json!({"type": secret})),
            "`token_placement.type`",
        ),
        (
            oauth2_with(
                "params",
                serde_json::json!([
                    {"stage": "authorizationRequest", "name": "a", "value": "1", "placement": "query"},
                    {"stage": "authorizationRequest", "name": "b", "value": "2", "placement": secret},
                ]),
            ),
            "`params[1].placement`",
        ),
        (
            format!(r#"{{"type":"apikey","key":"k","value":"v","placement":"{secret}"}}"#),
            "`placement`",
        ),
    ];
    for (json, path) in &cases {
        let text = parse_auth_json(json).expect_err(json).to_string();
        assert!(text.contains(path), "{path}: {text}");
        assert!(!text.contains("line 0"), "{text}");
        assert!(!text.contains(secret), "{text}");
    }
    let text = parse_auth_json(r#"{"type":"basic","username":"u"}"#)
        .expect_err("missing")
        .to_string();
    assert!(!text.contains("line 0"), "{text}");
}

#[test]
fn a_bad_last_param_among_thousands_is_found_quickly() {
    let mut params: Vec<serde_json::Value> = (0..5000)
        .map(|i| {
            serde_json::json!({"stage": "authorizationRequest", "name": format!("p{i}"),
                "value": "v", "placement": "query"})
        })
        .collect();
    params[4999]["placement"] = serde_json::json!("sideways");
    let json = oauth2_with("params", serde_json::Value::Array(params));
    let started = std::time::Instant::now();
    let text = parse_auth_json(&json).expect_err("refused").to_string();
    let elapsed = started.elapsed();
    assert!(text.contains("`params[4999].placement`"), "{text}");
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "took {elapsed:?}"
    );
}

/// An oauth2 document of about 5 MB: `count` params, the last with `placement` set to `last`.
fn large_oauth2(count: usize, last: serde_json::Value) -> String {
    let filler = "v".repeat(80);
    let mut params: Vec<serde_json::Value> = (0..count)
        .map(|i| {
            serde_json::json!({"stage": "accessTokenRequest", "name": format!("p{i}"),
                "value": filler, "placement": "body"})
        })
        .collect();
    params[count - 1]["placement"] = last;
    oauth2_with("params", serde_json::Value::Array(params))
}

#[test]
fn a_bad_last_param_in_a_five_megabyte_document_is_found_in_linear_time() {
    let count = 35_000;
    let valid = large_oauth2(count, serde_json::json!("body"));
    assert!(valid.len() > 5_000_000, "{} bytes", valid.len());
    let bad = large_oauth2(count, serde_json::json!(7));
    let started = std::time::Instant::now();
    let parsed = parse_auth_json(&valid);
    let valid_took = started.elapsed();
    assert!(parsed.is_ok(), "{parsed:?}");
    let started = std::time::Instant::now();
    let text = parse_auth_json(&bad).expect_err("refused").to_string();
    let bad_took = started.elapsed();
    let started = std::time::Instant::now();
    let slow = apinata_core::auth_json_paths::slow(&valid);
    let slow_took = started.elapsed();
    assert!(slow.is_ok(), "{slow:?}");
    eprintln!(
        "auth json {} bytes, {count} params: valid {valid_took:?}, bad last {bad_took:?}, \
         slow valid {slow_took:?}",
        bad.len()
    );
    assert!(
        text.contains(&format!("`params[{}].placement`", count - 1)),
        "{text}"
    );
    assert!(
        bad_took < std::time::Duration::from_secs(5),
        "took {bad_took:?}"
    );
    // Load scales both parses alike; finding the bad value must cost about one more parse
    // than the error path's own pass.
    assert!(
        bad_took < slow_took * 3 + std::time::Duration::from_millis(100),
        "bad {bad_took:?} against slow valid {slow_took:?}"
    );
}

#[test]
fn a_wrong_stage_names_its_path_and_the_files_spellings() {
    let json = oauth2_with(
        "params",
        serde_json::json!([{"stage": "access_token_request", "name": "a", "value": "1",
            "placement": "query"}]),
    );
    let text = parse_auth_json(&json).expect_err("refused").to_string();
    assert!(text.contains("`params[0].stage`"), "{text}");
    for spelling in [
        "`authorizationRequest`",
        "`accessTokenRequest`",
        "`refreshTokenRequest`",
    ] {
        assert!(text.contains(spelling), "{spelling}: {text}");
    }
    assert!(!text.contains("access_token_request"), "{text}");
}

#[test]
fn a_null_token_placement_key_is_refused_and_named() {
    let cases = [
        (
            serde_json::json!({"type": "unset", "prefix": null}),
            "prefix",
        ),
        (serde_json::json!({"type": "unset", "key": null}), "key"),
        (
            serde_json::json!({"type": "header", "prefix": "x", "key": null}),
            "key",
        ),
        (
            serde_json::json!({"type": "header", "prefix": null}),
            "prefix",
        ),
        (serde_json::json!({"type": "query", "key": null}), "key"),
    ];
    for (placement, field) in cases {
        let json = oauth2_with("token_placement", placement.clone());
        let text = parse_auth_json(&json)
            .expect_err(&placement.to_string())
            .to_string();
        assert!(text.contains(field), "{placement}: {text}");
        assert!(!text.contains("null"), "{placement}: {text}");
    }
}

#[test]
fn the_fast_and_slow_paths_agree_on_every_valid_document() {
    use apinata_core::auth_json_paths::{fast, slow};
    let mut docs: Vec<String> = [
        r#"{"type":"none"}"#,
        r#"{"type":"inherit"}"#,
        r#"{"type":"basic","username":"u","password":"p"}"#,
        r#"{"type":"digest","username":"u","password":"p"}"#,
        r#"{"type":"wsse","username":"u","password":"p"}"#,
        r#"{"type":"bearer","token":"{{t}}"}"#,
        r#"{"type":"apikey","key":"X-Key","value":"v","placement":"query"}"#,
        r#"{"type":"ntlm","username":"u","password":"p","domain":""}"#,
        r#"{"type":"awsv4","access_key_id":"a","secret_access_key":"s","session_token":"",
            "service":"","region":"eu","profile_name":""}"#,
        r#"{"type":"oauth1","consumer_key":"ck","consumer_secret":null,"access_token":"",
            "access_token_secret":null,"callback_url":null,"verifier":null,
            "signature_method":"RSA-SHA256","private_key":{"type":"file","value":"k.pem"},
            "timestamp":null,"nonce":null,"version":"1.0","realm":"r","placement":"header",
            "include_body_hash":true}"#,
        r#"{"type":"edgegrid","access_token":"a","client_token":"c","client_secret":"s",
            "nonce":"","timestamp":"","base_url":"","headers_to_sign":"","max_body_size":"Infinity"}"#,
        r#"{"type":"edgegrid","access_token":"a","client_token":"c","client_secret":"s",
            "nonce":"","timestamp":"","base_url":"","headers_to_sign":"","max_body_size":1024}"#,
        r#"{"type":"edgegrid","access_token":"a","client_token":"c","client_secret":"s",
            "nonce":"","timestamp":"","base_url":"","headers_to_sign":"","max_body_size":null}"#,
        r#"{"type":"oauth2","flow":"authorization_code",
            "authorization_url":"https://id/auth","access_token_url":"https://id/token",
            "refresh_token_url":"","callback_url":"http://localhost/cb","client_id":"app",
            "client_secret":"{{cs}}","client_auth":"basic_auth_header","username":"","password":"",
            "scope":"read","state":"st","pkce":true,"token_id":"credentials",
            "token_placement":{"type":"query","key":"access_token"},"token_source":"id_token",
            "auto_fetch":false,"auto_refresh":true,
            "params":[{"stage":"authorizationRequest","name":"aud","value":"api","placement":"query"}]}"#,
    ]
    .map(String::from)
    .to_vec();
    docs.push(oauth2_with("params", serde_json::json!([])));
    docs.push(oauth2_with(
        "token_placement",
        serde_json::json!({"type": "header", "prefix": "Bearer"}),
    ));
    docs.push(large_oauth2(100, serde_json::json!("header")));
    for doc in &docs {
        let slow = slow(doc).expect(doc);
        assert_eq!(fast(doc).as_ref(), Some(&slow), "{doc}");
    }
}
