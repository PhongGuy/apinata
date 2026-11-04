//! `export::postman` auth at request, folder and collection level, and its round trip.

use crate::export_support;

use apinata_core::export::postman;
use apinata_core::import::parse_postman;
use apinata_core::{AuthSpec, Collection};
use export_support::*;
use serde_json::{Value, json};

const AWS: &str = "  auth:\n    type: awsv4\n    accessKeyId: ak\n    secretAccessKey: sk\n    sessionToken: st\n    service: execute-api\n    region: eu-west-1\n    profileName: \"\"\n";
const OAUTH_CC: &str = "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://auth.test/token\n    credentials:\n      clientId: id1\n      clientSecret: s1\n      placement: body\n    scope: read\n    tokenConfig:\n      id: creds\n      placement:\n        header: Bearer\n    additionalParameters:\n      accessTokenRequest:\n        - name: audience\n          value: api\n          placement: body\n";
const OAUTH_CODE: &str = "  auth:\n    type: oauth2\n    flow: authorization_code\n    authorizationUrl: https://auth.test/authorize\n    accessTokenUrl: https://auth.test/token\n    callbackUrl: http://localhost/cb\n    credentials:\n      clientId: id2\n    pkce:\n      method: S256\n    tokenConfig:\n      placement:\n        query: access_token\n";
const MODES: [&str; 12] = [
    "  auth: none\n",
    "",
    INHERIT,
    "  auth:\n    type: basic\n    username: u\n    password: p\n",
    "  auth:\n    type: bearer\n    token: t\n",
    "  auth:\n    type: apikey\n    key: k\n    value: v\n    placement: query\n",
    "  auth:\n    type: apikey\n    key: k\n    value: v\n    placement: header\n",
    "  auth:\n    type: digest\n    username: u\n    password: p\n",
    AWS,
    "  auth:\n    type: ntlm\n    username: u\n    password: \"{{pw}}\"\n    domain: CORP\n",
    OAUTH_CC,
    OAUTH_CODE,
];

fn s(key: &str, value: &str) -> Value {
    json!({"key": key, "value": value, "type": "string"})
}

/// One request per auth case, named `r<i>` in files `<i>.yml`.
fn auth_collection(cases: &[&str]) -> (tempfile::TempDir, Collection) {
    let files: Vec<(String, String)> = cases
        .iter()
        .enumerate()
        .map(|(i, auth)| {
            (
                format!("{i}.yml"),
                http(&format!("r{i}"), "GET", "http://x.test", auth),
            )
        })
        .collect();
    let mut all = vec![("opencollection.yml", ROOT)];
    all.extend(files.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    open(&all)
}

#[test]
fn request_auth_maps_each_mode_like_the_reference() {
    let (_d, c) = auth_collection(&MODES);
    let v = exported(&c);
    let noauth = json!({"type": "noauth"});
    let want = [
        Some(noauth.clone()),
        Some(noauth.clone()),
        None,
        Some(json!({"type": "basic", "basic": [s("password", "p"), s("username", "u")]})),
        Some(json!({"type": "bearer", "bearer": [s("token", "t")]})),
        Some(
            json!({"type": "apikey", "apikey": [s("key", "k"), s("value", "v"), s("in", "query")]}),
        ),
        Some(
            json!({"type": "apikey", "apikey": [s("key", "k"), s("value", "v"), s("in", "header")]}),
        ),
        Some(json!({"type": "digest", "digest": [s("password", "p"), s("username", "u")]})),
        Some(
            json!({"type": "awsv4", "awsv4": [s("sessionToken", "st"), s("service", "execute-api"), s("region", "eu-west-1"), s("secretKey", "sk"), s("accessKey", "ak")]}),
        ),
        Some(
            json!({"type": "ntlm", "ntlm": [s("domain", "CORP"), s("password", "{{pw}}"), s("username", "u")]}),
        ),
        Some(json!({"type": "oauth2", "oauth2": [
            s("grant_type", "client_credentials"), s("accessTokenUrl", "https://auth.test/token"),
            s("clientId", "id1"), s("clientSecret", "s1"), s("scope", "read"), s("tokenName", "creds"),
            s("addTokenTo", "header"), s("headerPrefix", "Bearer"), s("client_authentication", "body"),
            {"key": "tokenRequestParams", "value": [{"key": "audience", "value": "api", "enabled": true, "send_as": "request_body"}], "type": "any"}]})),
        Some(json!({"type": "oauth2", "oauth2": [
            s("grant_type", "authorization_code_with_pkce"), s("accessTokenUrl", "https://auth.test/token"),
            s("clientId", "id2"), s("addTokenTo", "queryParams"), s("client_authentication", "header"),
            s("authUrl", "https://auth.test/authorize"), s("redirect_uri", "http://localhost/cb")]})),
    ];
    // Files sort by name, so `10.yml` and `11.yml` come right after `1.yml`.
    let order = [0usize, 1, 10, 11, 2, 3, 4, 5, 6, 7, 8, 9];
    for (at, case) in order.iter().enumerate() {
        assert_eq!(v["item"][at]["name"], format!("r{case}"));
        assert_eq!(
            v["item"][at]["request"].get("auth"),
            want[*case].as_ref(),
            "case {case}"
        );
    }
    assert_eq!(
        keys(&v["item"][5]["request"]),
        ["method", "header", "auth", "description", "url"]
    );
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
}

#[test]
fn collection_auth_sits_on_top_and_deferring_or_unmappable_folders_omit_auth() {
    let folder = |name: &str, seq: u32, auth: &str| {
        format!("info:\n  name: {name}\n  type: folder\n  seq: {seq}\nrequest:\n{auth}")
    };
    let bearer = format!("{ROOT}request:\n  auth:\n    type: bearer\n    token: t\n");
    let (_d, c) = open(&[
        ("opencollection.yml", &bearer),
        ("a/folder.yml", &folder("a", 1, "  auth: inherit\n")),
        ("b/folder.yml", &folder("b", 2, "  auth: none\n")),
        (
            "c/folder.yml",
            &folder(
                "c",
                3,
                "  auth:\n    type: basic\n    username: u\n    password: p\n",
            ),
        ),
        ("d/folder.yml", &folder("d", 4, "  auth:\n    type: hawk\n")),
    ]);
    let v = exported(&c);
    assert_eq!(keys(&v), ["info", "item", "variable", "auth"]);
    assert_eq!(
        v["auth"],
        json!({"type": "bearer", "bearer": [s("token", "t")]})
    );
    for (at, want) in [
        (0, vec!["name", "item"]),
        (1, vec!["name", "item"]),
        (2, vec!["name", "item", "auth"]),
        (3, vec!["name", "item"]),
    ] {
        assert_eq!(keys(&v["item"][at]), want, "folder {at}");
    }
    assert_eq!(
        notes(&c),
        pairs(&[("d / auth", "the hawk authentication cannot be exported")])
    );

    let hawk = format!("{ROOT}request:\n  auth:\n    type: hawk\n");
    let (_d, c) = open(&[("opencollection.yml", &hawk)]);
    assert_eq!(keys(&exported(&c)), ["info", "item", "variable"]);
    assert_eq!(
        notes(&c),
        pairs(&[(
            "collection / auth",
            "the hawk authentication cannot be exported"
        )])
    );
}

#[test]
fn auth_survives_the_round_trip_where_the_importer_maps_it() {
    let (_d, modes) = auth_collection(&MODES);
    let mut all = vec![("modes", modes)];
    all.extend(
        corpus()
            .into_iter()
            .map(|(l, root)| (l, Collection::open(&root).expect("opens"))),
    );
    for (label, c) in &all {
        let back =
            parse_postman(postman(c).json.as_bytes()).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let ours = source_requests(c.tree(), "");
        let theirs = imported_requests(&back.items, "");
        assert_eq!(ours.len(), theirs.len(), "{label}");
        for ((name, r), (_, item)) in ours.iter().zip(&theirs) {
            match r.auth() {
                // The exporter's own keys, read back: the credentials and the token endpoint
                // survive, while the Postman-side defaults may differ from the file's.
                AuthSpec::OAuth2(spec) => {
                    let AuthSpec::OAuth2(back) = &item.request.auth else {
                        panic!("{label}: {name}: {:?}", item.request.auth)
                    };
                    assert_eq!(back.flow, spec.flow, "{label}: {name}");
                    assert_eq!(back.client_id, spec.client_id, "{label}: {name}");
                    assert_eq!(
                        back.access_token_url, spec.access_token_url,
                        "{label}: {name}"
                    );
                    assert_eq!(back.client_secret, spec.client_secret, "{label}: {name}");
                    assert_eq!(back.scope, spec.scope, "{label}: {name}");
                    // A blank token name imports as the default `credentials`.
                    if !spec.token_id.is_empty() {
                        assert_eq!(back.token_id, spec.token_id, "{label}: {name}");
                    }
                }
                AuthSpec::Unsupported { .. } => {
                    assert_eq!(item.request.auth, AuthSpec::None)
                }
                auth => assert_eq!(item.request.auth, auth, "{label}: {name}"),
            }
        }
    }
}

#[test]
fn oauth1_exports_like_the_reference() {
    let rsa_file = "  auth:\n    type: oauth1\n    consumerKey: ck\n    signatureMethod: RSA-SHA1\n    privateKey:\n      type: file\n      value: keys/p.pem\n    placement: body\n";
    let hmac = "  auth:\n    type: oauth1\n    accessToken: tk\n    callbackUrl: oob\n    privateKey: inline\n    realm: r\n    placement: query\n    includeBodyHash: true\n";
    let rsa_text =
        "  auth:\n    type: oauth1\n    signatureMethod: RSA-SHA512\n    privateKey: PEM\n";
    let (_d, c) = auth_collection(&[rsa_file, hmac, rsa_text]);
    let v = exported(&c);
    let b = |key: &str, value: bool| json!({"key": key, "value": value, "type": "boolean"});
    let entries =
        |head: [Value; 5], key: Option<Value>, tail: [Value; 6], header: bool, hash: bool| {
            let mut all: Vec<Value> = head.into();
            all.extend(key);
            all.extend(tail);
            all.extend([b("addParamsToHeader", header), b("includeBodyHash", hash)]);
            json!({"type": "oauth1", "oauth1": all})
        };
    let blank_tail = || {
        [
            s("callback", ""),
            s("verifier", ""),
            s("timestamp", ""),
            s("nonce", ""),
            s("version", "1.0"),
            s("realm", ""),
        ]
    };
    let want = [
        entries(
            [
                s("consumerKey", "ck"),
                s("consumerSecret", ""),
                s("token", ""),
                s("tokenSecret", ""),
                s("signatureMethod", "RSA-SHA1"),
            ],
            Some(s("privateKey", "")),
            blank_tail(),
            true,
            false,
        ),
        entries(
            [
                s("consumerKey", ""),
                s("consumerSecret", ""),
                s("token", "tk"),
                s("tokenSecret", ""),
                s("signatureMethod", "HMAC-SHA1"),
            ],
            None,
            [
                s("callback", "oob"),
                s("verifier", ""),
                s("timestamp", ""),
                s("nonce", ""),
                s("version", "1.0"),
                s("realm", "r"),
            ],
            false,
            true,
        ),
        entries(
            [
                s("consumerKey", ""),
                s("consumerSecret", ""),
                s("token", ""),
                s("tokenSecret", ""),
                s("signatureMethod", "RSA-SHA512"),
            ],
            Some(s("privateKey", "PEM")),
            blank_tail(),
            true,
            false,
        ),
    ];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(v["item"][i]["request"]["auth"], *w, "r{i}");
    }
    assert!(notes(&c).is_empty(), "{:?}", notes(&c));
}

#[test]
fn ntlm_round_trips_through_postman_at_every_level() {
    let folder = |name: &str, seq: u32, auth: &str| {
        format!("info:\n  name: {name}\n  type: folder\n  seq: {seq}\nrequest:\n{auth}")
    };
    let ntlm = "  auth:\n    type: ntlm\n    username: u\n    password: p\n    domain: \"\"\n";
    let root = format!("{ROOT}request:\n{ntlm}");
    let (_d, c) = open(&[
        ("opencollection.yml", &root),
        ("f/folder.yml", &folder("f", 1, ntlm)),
        ("f/r.yml", &http("r", "GET", "http://x.test", ntlm)),
    ]);
    let v = exported(&c);
    let want =
        json!({"type": "ntlm", "ntlm": [s("domain", ""), s("password", "p"), s("username", "u")]});
    assert_eq!(v["auth"], want);
    assert_eq!(v["item"][0]["auth"], want);
    assert_eq!(v["item"][0]["item"][0]["request"]["auth"], want);
    assert!(notes(&c).is_empty());
    let back = parse_postman(&serde_json::to_vec(&v).unwrap()).expect("parses");
    assert_eq!(
        back.auth,
        AuthSpec::Ntlm {
            username: "u".into(),
            password: "p".into(),
            domain: String::new()
        }
    );
}
