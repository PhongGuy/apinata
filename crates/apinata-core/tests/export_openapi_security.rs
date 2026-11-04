//! `export::openapi` security: schemes per mode, naming, inheritance, unmappable auth.

use crate::export_support;

use apinata_core::export::openapi;
use export_support::*;
use serde_json::{Value, json};

const BASIC: &str =
    "  auth:\n    type: basic\n    username: user-secret\n    password: pw-secret\n";
const BASIC2: &str = "  auth:\n    type: basic\n    username: u2\n    password: pw2-secret\n";
const BEARER: &str = "  auth:\n    type: bearer\n    token: tok-secret\n";
const DIGEST: &str =
    "  auth:\n    type: digest\n    username: user-secret\n    password: pw-secret\n";
const KEY_HEADER: &str =
    "  auth:\n    type: apikey\n    key: X-Key\n    value: key-secret\n    placement: header\n";
const KEY_QUERY: &str =
    "  auth:\n    type: apikey\n    key: key\n    value: key-secret\n    placement: query\n";
const CC: &str = "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://auth.test/token\n    credentials:\n      clientId: id-secret\n      clientSecret: cs-secret\n    scope: read write\n";
const CODE: &str = "  auth:\n    type: oauth2\n    flow: authorization_code\n    authorizationUrl: https://auth.test/authorize\n    accessTokenUrl: https://auth.test/token\n    callbackUrl: http://localhost/cb\n    credentials:\n      clientId: id-secret\n      clientSecret: cs-secret\n";
const PASSWORD: &str = "  auth:\n    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: https://auth.test/token\n    resourceOwner:\n      username: user-secret\n      password: pw-secret\n";
const IMPLICIT: &str = "  auth:\n    type: oauth2\n    flow: implicit\n    authorizationUrl: https://auth.test/authorize\n    scope: read\n";
const AWS: &str = "  auth:\n    type: awsv4\n    accessKeyId: ak-secret\n    secretAccessKey: sk-secret\n    sessionToken: st\n    service: s\n    region: r\n    profileName: \"\"\n";
const NONE: &str = "  auth: none\n";
const SECRETS: [&str; 10] = [
    "user-secret",
    "pw-secret",
    "pw2-secret",
    "tok-secret",
    "key-secret",
    "id-secret",
    "cs-secret",
    "ak-secret",
    "sk-secret",
    "other-secret",
];

fn req(name: &str, seq: u32, auth: &str) -> (String, String) {
    (
        format!("{name}.yml"),
        http_at(name, seq, "GET", &format!("https://x.test/{name}"), auth),
    )
}

fn security(v: &Value, name: &str) -> Option<Value> {
    v["paths"][format!("/{name}")]["get"]
        .get("security")
        .cloned()
}

fn assert_no_secrets(json: &str) {
    for s in SECRETS {
        assert!(!json.contains(s), "{s} leaked");
    }
}

#[test]
fn each_mode_maps_to_one_named_scheme_without_credentials() {
    let modes = [
        BASIC, BEARER, DIGEST, KEY_HEADER, KEY_QUERY, CC, CODE, PASSWORD, IMPLICIT, BASIC2, NONE,
    ];
    let files: Vec<(String, String)> = modes
        .iter()
        .enumerate()
        .map(|(i, a)| req(&format!("r{i}"), i as u32 + 1, a))
        .collect();
    let (_d, c) = open_with_root(ROOT, &files);
    let v = openapi_doc(&c);
    let want = [
        "basicAuth",
        "bearerAuth",
        "digestAuth",
        "apiKeyAuth",
        "apiKeyAuth2",
        "oauth2",
        "oauth22",
        "oauth23",
        "oauth24",
        "basicAuth",
    ];
    for (i, name) in want.iter().enumerate() {
        let scopes = match *name {
            "oauth2" => json!(["read", "write"]),
            "oauth24" => json!(["read"]),
            _ => json!([]),
        };
        assert_eq!(
            security(&v, &format!("r{i}")),
            Some(json!([{*name: scopes}])),
            "r{i}"
        );
    }
    assert_eq!(security(&v, "r10"), None);
    assert_eq!(
        keys(&v["paths"]["/r0"]["get"]),
        ["summary", "operationId", "responses", "security"]
    );
    let token = "https://auth.test/token";
    let authorize = "https://auth.test/authorize";
    let schemes = &v["components"]["securitySchemes"];
    assert_eq!(
        keys(schemes),
        [
            "basicAuth",
            "bearerAuth",
            "digestAuth",
            "apiKeyAuth",
            "apiKeyAuth2",
            "oauth2",
            "oauth22",
            "oauth23",
            "oauth24"
        ]
    );
    assert_eq!(
        *schemes,
        json!({
            "basicAuth": {"type": "http", "scheme": "basic"},
            "bearerAuth": {"type": "http", "scheme": "bearer"},
            "digestAuth": {"type": "http", "scheme": "digest"},
            "apiKeyAuth": {"type": "apiKey", "name": "X-Key", "in": "header"},
            "apiKeyAuth2": {"type": "apiKey", "name": "key", "in": "query"},
            "oauth2": {"type": "oauth2", "flows": {"clientCredentials": {"tokenUrl": token, "scopes": {"read": "", "write": ""}}}},
            "oauth22": {"type": "oauth2", "flows": {"authorizationCode": {"authorizationUrl": authorize, "tokenUrl": token, "scopes": {}}}},
            "oauth23": {"type": "oauth2", "flows": {"password": {"tokenUrl": token, "scopes": {}}}},
            "oauth24": {"type": "oauth2", "flows": {"implicit": {"authorizationUrl": authorize, "scopes": {"read": ""}}}},
        })
    );
    assert_eq!(
        keys(&v),
        ["openapi", "info", "servers", "paths", "components"]
    );
    assert_no_secrets(&openapi(&c).json);
}

#[test]
fn inherited_auth_applies_and_only_a_differing_requirement_is_written() {
    let root = format!("{ROOT}request:\n{BEARER}");
    let folder = |name: &str, auth: &str| {
        format!("info:\n  name: {name}\n  type: folder\n  seq: 1\nrequest:\n{auth}")
    };
    let mut files = vec![];
    for (name, auth) in [
        ("a", "  auth: inherit\n"),
        ("b", NONE),
        ("c", BASIC),
        ("d", AWS),
    ] {
        files.push((format!("{name}/folder.yml"), folder(name, auth)));
        files.push((
            format!("{name}/r.yml"),
            http_at(name, 1, "GET", &format!("https://x.test/{name}"), INHERIT),
        ));
    }
    files.push(req(
        "e",
        1,
        "  auth:\n    type: bearer\n    token: other-secret\n",
    ));
    files.push(req("f", 2, KEY_HEADER));
    files.push(req("g", 3, NONE));
    let (_d, c) = open_with_root(&root, &files);
    let v = openapi_doc(&c);
    assert_eq!(v["security"], json!([{"bearerAuth": []}]));
    assert_eq!(
        keys(&v),
        [
            "openapi",
            "info",
            "servers",
            "tags",
            "paths",
            "components",
            "security"
        ]
    );
    // A folder's `none` passes through to the collection, as a send does.
    for (name, want) in [
        ("a", None),
        ("b", None),
        ("c", Some(json!([{"basicAuth": []}]))),
        ("d", Some(json!([]))),
        ("e", None),
        ("f", Some(json!([{"apiKeyAuth": []}]))),
        ("g", Some(json!([]))),
    ] {
        assert_eq!(security(&v, name), want, "{name}");
    }
    assert_eq!(
        keys(&v["components"]["securitySchemes"]),
        ["bearerAuth", "basicAuth", "apiKeyAuth"]
    );
    assert_eq!(
        openapi_notes(&c),
        pairs(&[("d / auth", "the awsv4 authentication cannot be exported")])
    );
    assert_no_secrets(&openapi(&c).json);
}

#[test]
fn unmappable_auth_is_noted_where_it_is_written() {
    let root = format!("{ROOT}request:\n  auth:\n    type: ntlm\n");
    let cc = |url: &str| {
        format!(
            "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{url}\"\n"
        )
    };
    let files = [
        req("a", 1, INHERIT),
        req("b", 2, NONE),
        req("c", 3, BASIC),
        req("d", 4, &cc("{{authUrl}}/token")),
        req("e", 5, &cc("/token")),
        req("f", 6, &cc("")),
        req("g", 7, &cc("ftp://auth.test/token")),
        req("h", 8, "  auth:\n    type: oauth2\n    flow: implicit\n"),
    ];
    let (_d, c) = open_with_root(&root, &files);
    let v = openapi_doc(&c);
    assert_eq!(
        keys(&v),
        ["openapi", "info", "servers", "paths", "components"]
    );
    for (name, want) in [
        ("a", None),
        ("b", None),
        ("c", Some(json!([{"basicAuth": []}]))),
        ("d", None),
        ("h", None),
    ] {
        assert_eq!(security(&v, name), want, "{name}");
    }
    let token = "the oauth2 authentication cannot be exported: its token URL is not an absolute http or https URL";
    let authorize = "the oauth2 authentication cannot be exported: its authorization URL is not an absolute http or https URL";
    assert_eq!(
        openapi_notes(&c),
        pairs(&[
            (
                "collection / auth",
                "the ntlm authentication cannot be exported"
            ),
            ("d / auth", token),
            ("e / auth", token),
            ("f / auth", token),
            ("g / auth", token),
            ("h / auth", authorize),
        ])
    );
}

#[test]
fn oauth2_urls_with_credentials_or_padding_and_blank_api_key_names_are_unmappable() {
    let cc = |url: &str| {
        format!(
            "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{url}\"\n"
        )
    };
    let key = |name: &str| {
        format!(
            "  auth:\n    type: apikey\n    key: \"{name}\"\n    value: key-secret\n    placement: header\n"
        )
    };
    let files = [
        req("a", 1, &cc("https://id-secret:cs-secret@auth.test/token")),
        req("b", 2, &cc("https://user-secret@auth.test/token")),
        req("c", 3, &cc(" https://auth.test/token")),
        req("d", 4, &cc("https://auth.test/token ")),
        req("e", 5, &key("")),
        req("f", 6, &key("  ")),
    ];
    let (_d, c) = open_with_root(ROOT, &files);
    let v = openapi_doc(&c);
    assert!(v.get("components").is_none(), "{v}");
    let token = "the oauth2 authentication cannot be exported: its token URL is not an absolute http or https URL";
    let apikey = "the apikey authentication cannot be exported";
    assert_eq!(
        openapi_notes(&c),
        pairs(&[
            ("a / auth", token),
            ("b / auth", token),
            ("c / auth", token),
            ("d / auth", token),
            ("e / auth", apikey),
            ("f / auth", apikey),
        ])
    );
    assert_no_secrets(&openapi(&c).json);
}

#[test]
fn oauth1_is_noted_as_not_exportable() {
    let files = [req(
        "o",
        1,
        "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: cs\n",
    )];
    let (_d, c) = open_with_root(ROOT, &files);
    let v = openapi_doc(&c);
    assert_eq!(security(&v, "o"), None);
    assert_eq!(
        openapi_notes(&c),
        pairs(&[("o / auth", "the oauth1 authentication cannot be exported")])
    );
}

#[test]
fn an_oauth2_url_with_a_literal_credential_in_its_query_gets_a_note() {
    let auth = "  auth:\n    type: oauth2\n    flow: authorization_code\n    authorizationUrl: https://auth.test/authorize?api_key=k-1&state=x\n    accessTokenUrl: https://auth.test/token?client_secret=cs-1&v=2\n    credentials:\n      clientId: id\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", "https://x.test/r", auth)),
    ]);
    let tail = "holds a literal credential, written into the document as a flow URL";
    assert_eq!(
        openapi_notes(&c),
        pairs(&[
            (
                "r / auth",
                &format!("oauth2 authorization URL query parameter api_key {tail}")
            ),
            (
                "r / auth",
                &format!("oauth2 token URL query parameter client_secret {tail}")
            ),
        ])
    );
}

#[test]
fn an_oauth2_url_with_no_literal_credential_gets_no_note() {
    let auth = "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://auth.test/token?audience=api&token=tok\n    credentials:\n      clientId: id\n";
    let clean = auth.replace("&token=tok", "&scope=a");
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", "https://x.test/r", &clean)),
    ]);
    assert!(openapi_notes(&c).is_empty());
    let templated = auth.replace("token=tok", "token={{t}}");
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", "GET", "https://x.test/r", &templated)),
    ]);
    // A templated URL is not exported at all, so it is unmappable and never a credential note.
    assert!(
        openapi_notes(&c)
            .iter()
            .all(|(_, d)| !d.contains("literal credential"))
    );
}

#[test]
fn a_collection_level_oauth2_url_with_a_literal_credential_gets_a_note_once() {
    let root = ROOT.to_owned()
        + "request:\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://auth.test/token?access_token=t-1\n    credentials:\n      clientId: id\n";
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        (
            "r.yml",
            &http("r", "GET", "https://x.test/r", "  auth: inherit\n"),
        ),
    ]);
    assert_eq!(
        openapi_notes(&c),
        pairs(&[(
            "collection / auth",
            "oauth2 token URL query parameter access_token holds a literal credential, written into the document as a flow URL"
        )])
    );
}
