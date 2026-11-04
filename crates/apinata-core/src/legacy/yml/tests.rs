//! Corpus-driven mapping tests: an accepted input's reference parse, mapped and emitted, is
//! byte-equal to the YAML the reference migration wrote for it. Later tasks append modules.

use super::map_environment;
use super::value::{
    ensure_string, get, is_non_empty_string, js_or, number, serialize_variable_value, to_mapping,
    to_number, truthy, value_to_string,
};
use crate::yaml::emit::to_bruno_yaml;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use yaml_serde::Mapping;

/// Every group with per-file outputs; `collections` inputs are compared as whole trees.
const ALL_GROUPS: [&str; 5] = ["basics", "auth", "request", "examples", "collection"];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-10-14-bru-migration-parity")
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // Oracles may hold lone-surrogate escapes and 1e400-style numbers.
    crate::legacy::bru::parse_js_json(&text)
        .unwrap_or_else(|e| panic!("{}: {}", path.display(), e.message))
}

/// One accepted unit or synthetic input and its reference parse.
struct Case {
    input: String,
    parser: String,
    parsed: Value,
}

fn cases(groups: &[&str]) -> Vec<Case> {
    let manifest = read_json(&corpus().join("parsed/manifest.json"));
    let mut out = Vec::new();
    for row in manifest.as_array().expect("the manifest is a JSON array") {
        let field = |key: &str| {
            row[key]
                .as_str()
                .unwrap_or_else(|| panic!("manifest row lacks {key}: {row}"))
                .to_owned()
        };
        let (input, parser, group) = (field("input"), field("parser"), field("group"));
        if !groups.contains(&group.as_str()) || parser == "bruno_json" {
            continue;
        }
        // The reader's JSON cannot tell an infinite number from the string "Infinity" or hold
        // a lone surrogate.
        if input.ends_with("-carrier.bru") {
            continue;
        }
        // A rejected input has only an `.error.json` oracle and no migration output.
        let oracle = corpus().join("parsed").join(format!("{input}.json"));
        if oracle.is_file() {
            out.push(Case {
                parsed: read_json(&oracle),
                input,
                parser,
            });
        }
    }
    out
}

/// The first differing line with up to three lines of context: `-` expected, `+` actual.
fn first_difference(want: &str, got: &str) -> String {
    let want: Vec<&str> = want.split('\n').collect();
    let got: Vec<&str> = got.split('\n').collect();
    let at = want
        .iter()
        .zip(&got)
        .position(|(w, g)| w != g)
        .unwrap_or(want.len().min(got.len()));
    let from = at.saturating_sub(3);
    let mut out = format!("  first difference at line {}\n", at + 1);
    for (i, line) in want.iter().enumerate().take(at + 4).skip(from) {
        let mark = if i < at { ' ' } else { '-' };
        out.push_str(&format!("  {mark}{:>4} {line:?}\n", i + 1));
    }
    for (i, line) in got.iter().enumerate().take(at + 4).skip(at) {
        out.push_str(&format!("  +{:>4} {line:?}\n", i + 1));
    }
    out
}

type SelectFn<'a> = &'a dyn Fn(&Case) -> bool;
type MapFn<'a> = &'a dyn Fn(&Case) -> Mapping;

/// Map every selected input of `groups`, emit it, and compare with `expected/<input minus .bru>.yml`.
fn assert_mapped_files_match(groups: &[&str], select: SelectFn<'_>, map: MapFn<'_>) {
    let selected: Vec<Case> = cases(groups).into_iter().filter(|c| select(c)).collect();
    assert!(
        !selected.is_empty(),
        "no accepted input in {groups:?} matches the selection"
    );
    let mut failures = Vec::new();
    for case in &selected {
        let stem = case
            .input
            .strip_suffix(".bru")
            .expect("per-file inputs are .bru files");
        let Ok(want) = fs::read_to_string(corpus().join("expected").join(format!("{stem}.yml")))
        else {
            let thrown = corpus()
                .join("expected")
                .join(format!("{stem}.error.json"))
                .is_file();
            let why = if thrown {
                "the reference migration throws; a2 does not model that"
            } else {
                "no expected/ output"
            };
            failures.push(format!("{}: {why}", case.input));
            continue;
        };
        let got = to_bruno_yaml(&map(case));
        if got != want {
            failures.push(format!(
                "{}:\n{}",
                case.input,
                first_difference(&want, &got)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} mapped files differ:\n{}",
        failures.len(),
        selected.len(),
        failures
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn value_helpers_follow_javascript() {
    let doc = json!({"a": {"b": [1, {"c": 2}], "n": null}});
    assert_eq!(get(&doc, "a.b.1.c"), Some(&json!(2)));
    assert_eq!(get(&doc, "a.n"), Some(&Value::Null));
    assert_eq!(get(&doc, "a.x"), None);
    assert_eq!(get(&doc, "a.n.deeper"), None);
    assert!(!truthy(None));
    for v in [Value::Null, json!(false), json!(0), json!(0.0), json!("")] {
        assert!(!truthy(Some(&v)), "{v}");
    }
    for v in [json!("0"), json!(" "), json!([]), json!({}), json!(-1)] {
        assert!(truthy(Some(&v)), "{v}");
    }
    assert!(is_non_empty_string(Some(&json!("x"))));
    assert!(!is_non_empty_string(None));
    for v in [json!(" \u{a0}\t"), json!(""), json!(5)] {
        assert!(!is_non_empty_string(Some(&v)), "{v}");
    }
    assert_eq!(js_or(Some(&json!(0)), json!("")), json!(""));
    assert_eq!(js_or(Some(&json!(7)), json!("")), json!(7));
    assert_eq!(js_or(None, json!("GET")), json!("GET"));
    assert!(to_number(None).is_nan());
    let numbers = [
        (Value::Null, 0.0),
        (json!(true), 1.0),
        (json!(" 12 "), 12.0),
        (json!(""), 0.0),
        (json!("1e3"), 1000.0),
        (json!("0x10"), 16.0),
        (json!([]), 0.0),
        (json!(["7"]), 7.0),
        (json!(2.5), 2.5),
    ];
    for (v, n) in numbers {
        assert_eq!(to_number(Some(&v)), n, "{v}");
    }
    for v in [json!("abc"), json!([1, 2]), json!({})] {
        assert!(to_number(Some(&v)).is_nan(), "{v}");
    }
    assert_eq!(number(1.0).to_string(), "1");
    assert_eq!(number(1.5).to_string(), "1.5");
    assert_eq!(number(-0.0).to_string(), "0");
    assert_eq!(ensure_string(None), "");
    assert_eq!(ensure_string(Some(&Value::Null)), "");
    assert_eq!(ensure_string(Some(&json!(7))), "7");
    assert_eq!(ensure_string(Some(&json!(1e21))), "1e+21");
    assert_eq!(ensure_string(Some(&json!([1, [2, null], "a"]))), "1,2,,a");
    assert_eq!(ensure_string(Some(&json!({"a": 1}))), "[object Object]");
    assert_eq!(value_to_string(Some(&json!(true))), "true");
    assert_eq!(value_to_string(Some(&json!({"a": 1}))), r#"{"a":1}"#);
    assert_eq!(
        serialize_variable_value(Some(&json!({"k": 1}))),
        "{\n  \"k\": 1\n}"
    );
    assert_eq!(serialize_variable_value(Some(&json!(42))), "42");
    assert_eq!(serialize_variable_value(None), "");
    let doc = to_mapping(json!({"b": 1, "a": 2.0, "c": "1"}));
    assert_eq!(to_bruno_yaml(&doc), "b: 1\na: 2\nc: \"1\"\n");
}

#[test]
fn environment_fields_follow_the_reference_writer() {
    let env = json!({
        "variables": [
            {"name": "host", "value": "http://x", "enabled": true, "secret": false},
            {"name": "off", "value": "", "enabled": false, "secret": false, "description": ""},
            {"name": "n", "value": 42, "dataType": "number", "enabled": true, "secret": false},
            {"name": "o", "value": {"a": [1]}, "dataType": "object", "enabled": true, "secret": false},
            {"name": "tok", "value": "", "enabled": true, "secret": true, "dataType": "boolean", "description": "d"},
            {"name": "tok2", "value": "", "enabled": false, "secret": true, "dataType": "string"}
        ],
        "color": "#ff0000",
        "extends": [" Base ", "Other"],
        "externalSecrets": {"type": "vault", "variables": [{"name": "a", "path": "p"}]}
    });
    let want = r##"extends:
  - Base
  - Other
color: "#ff0000"
variables:
  - name: host
    value: http://x
  - name: off
    value: ""
    disabled: true
    description: ""
  - name: n
    value:
      type: number
      data: "42"
  - name: o
    value:
      type: object
      data: |-
        {
          "a": [
            1
          ]
        }
  - secret: true
    name: tok
    type: boolean
    description: d
  - secret: true
    name: tok2
    disabled: true
externalSecrets:
  type: vault
  variables:
    - name: a
      path: p
"##;
    assert_eq!(to_bruno_yaml(&map_environment(&env)), want);
    let bare = json!({"variables": [], "extends": ["ok", ""], "color": ""});
    assert_eq!(to_bruno_yaml(&map_environment(&bare)), "{}\n");
    let one = json!({"variables": [{"name": "a", "value": "b", "enabled": true, "secret": false}], "extends": "  Base  "});
    assert_eq!(
        to_bruno_yaml(&map_environment(&one)),
        "extends: Base\nvariables:\n  - name: a\n    value: b\n"
    );
}

#[test]
fn environment_files_match_the_reference() {
    assert_mapped_files_match(&ALL_GROUPS, &|c| c.parser == "environment", &|c| {
        map_environment(&c.parsed)
    });
}

/// Each case's result as the reference's `JSON.stringify` prints it; `undefined` for none.
fn check_js(cases: Vec<(&str, Option<Value>, &str)>) {
    let failures: Vec<String> = cases
        .into_iter()
        .filter_map(|(label, got, want)| {
            let got = got.map_or_else(|| "undefined".to_owned(), |v| crate::js::stringify(&v));
            (got != want).then(|| format!("{label}\n  want {want}\n  got  {got}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

mod row_mappers {
    use super::check_js;
    use crate::legacy::yml::rows::{self, GRPC_SCRIPT_KEYS, HTTP_SCRIPT_KEYS};
    use serde_json::json;

    #[test]
    fn row_mappers_match_the_reference() {
        check_js(vec![
            (
                "headers",
                rows::http_headers(Some(
                    &json!([{"name": "a", "value": "1", "enabled": true}, {"name": "", "value": "", "enabled": false, "description": "  "}, {"name": "b", "value": "x", "enabled": true, "description": "d"}]),
                )),
                r#"[{"name":"a","value":"1"},{"name":"","value":"","disabled":true},{"name":"b","value":"x","description":"d"}]"#,
            ),
            (
                "headers-empty",
                rows::http_headers(Some(&json!([]))),
                "undefined",
            ),
            (
                "response-headers",
                rows::response_headers(Some(
                    &json!([{"name": "a", "value": "1", "enabled": false, "description": "d"}]),
                )),
                r#"[{"name":"a","value":"1"}]"#,
            ),
            (
                "params",
                rows::params(Some(
                    &json!([{"name": "q", "value": "1", "type": "query", "enabled": true}, {"name": "id", "value": "", "type": "path", "enabled": false, "description": "p"}]),
                )),
                r#"[{"name":"q","value":"1","type":"query"},{"name":"id","value":"","type":"path","description":"p","disabled":true}]"#,
            ),
            (
                "variables",
                rows::variables(Some(
                    &json!({"req": [{"name": "a", "value": "1", "enabled": true, "local": false}, {"name": "n", "value": 5, "dataType": "number", "enabled": true}, {"name": "o", "value": {"k": 1}, "dataType": "object", "enabled": false}, {"name": "s", "value": "x", "dataType": "string", "enabled": true, "description": "d"}, {"name": "b", "value": true, "dataType": "boolean", "enabled": true}], "res": [{"name": "z", "value": "q"}]}),
                )),
                r#"[{"name":"a","value":"1"},{"name":"n","value":{"type":"number","data":"5"}},{"name":"o","value":{"type":"object","data":"{\n  \"k\": 1\n}"},"disabled":true},{"name":"s","value":"x","description":"d"},{"name":"b","value":{"type":"boolean","data":"true"}}]"#,
            ),
            (
                "variables-none",
                rows::variables(Some(
                    &json!({"req": [], "res": [{"name": "z", "value": "q"}]}),
                )),
                "undefined",
            ),
            (
                "actions",
                rows::actions(Some(
                    &json!([{"name": "tok", "value": "res.body.t", "enabled": true, "local": false}, {"name": "l", "value": 7, "enabled": false, "local": true, "description": "d"}]),
                )),
                r#"[{"type":"set-variable","phase":"after-response","selector":{"expression":"res.body.t","method":"jsonq"},"variable":{"name":"tok","scope":"runtime"}},{"type":"set-variable","phase":"after-response","selector":{"expression":"7","method":"jsonq"},"variable":{"name":"l","scope":"request"},"description":"d","disabled":true}]"#,
            ),
            (
                "scripts-http",
                rows::scripts(
                    Some(
                        &json!({"script": {"req": "  a  ", "res": " ", "beforeCallStart": "g"}, "tests": "t\n"}),
                    ),
                    HTTP_SCRIPT_KEYS,
                ),
                r#"[{"type":"before-request","code":"a"},{"type":"tests","code":"t"}]"#,
            ),
            (
                "scripts-grpc",
                rows::scripts(
                    Some(
                        &json!({"script": {"req": "a", "beforeCallStart": " g ", "afterCallEnd": "e"}, "tests": ""}),
                    ),
                    GRPC_SCRIPT_KEYS,
                ),
                r#"[{"type":"grpc:before-call-start","code":"g"},{"type":"grpc:after-call-end","code":"e"}]"#,
            ),
            (
                "scripts-none",
                rows::scripts(
                    Some(&json!({"script": {}, "tests": "  "})),
                    HTTP_SCRIPT_KEYS,
                ),
                "undefined",
            ),
            (
                "assertions",
                rows::assertions(Some(
                    &json!([{"name": "res.status", "value": "eq 200", "enabled": true}, {"name": "x", "value": "isEmpty", "enabled": false}, {"name": "y", "value": "200", "enabled": true}, {"name": "z", "value": "", "enabled": true}, {"name": "w", "value": "  notIn a  b", "enabled": true, "description": "d"}, {"name": "v", "value": "length", "enabled": true}]),
                )),
                r#"[{"expression":"res.status","operator":"eq","value":"200"},{"expression":"x","operator":"isEmpty","disabled":true},{"expression":"y","operator":"eq","value":"200"},{"expression":"z","operator":"eq","value":""},{"expression":"w","operator":"notIn","value":"a  b","description":"d"},{"expression":"v","operator":"length","value":""}]"#,
            ),
            (
                "body-json",
                rows::body(Some(&json!({"mode": "json", "json": "{}"}))),
                r#"{"type":"json","data":"{}"}"#,
            ),
            (
                "body-text-missing",
                rows::body(Some(&json!({"mode": "text"}))),
                r#"{"type":"text","data":""}"#,
            ),
            (
                "body-form",
                rows::body(Some(
                    &json!({"mode": "formUrlEncoded", "formUrlEncoded": [{"name": "a", "value": "1", "enabled": false, "description": "d"}]}),
                )),
                r#"{"type":"form-urlencoded","data":[{"name":"a","value":"1","description":"d","disabled":true}]}"#,
            ),
            (
                "body-form-empty",
                rows::body(Some(
                    &json!({"mode": "formUrlEncoded", "formUrlEncoded": []}),
                )),
                r#"{"type":"form-urlencoded"}"#,
            ),
            (
                "body-multipart",
                rows::body(Some(
                    &json!({"mode": "multipartForm", "multipartForm": [{"name": "f", "type": "file", "value": [], "enabled": true, "contentType": "image/png"}, {"name": "t", "type": "text", "value": "v", "enabled": false, "contentType": ""}]}),
                )),
                r#"{"type":"multipart-form","data":[{"name":"f","type":"file","value":[],"contentType":"image/png"},{"name":"t","type":"text","value":"v","disabled":true}]}"#,
            ),
            (
                "body-file",
                rows::body(Some(
                    &json!({"mode": "file", "file": [{"filePath": "a.bin", "contentType": "", "selected": true}, {"filePath": "b.bin"}]}),
                )),
                r#"{"type":"file","data":[{"filePath":"a.bin","contentType":"","selected":true},{"filePath":"b.bin","contentType":"","selected":false}]}"#,
            ),
            (
                "body-graphql",
                rows::body(Some(&json!({"mode": "graphql", "graphql": {"query": "q"}}))),
                "undefined",
            ),
            (
                "body-none",
                rows::body(Some(&json!({"mode": "none"}))),
                "undefined",
            ),
            (
                "omit",
                rows::omit_headers(Some(&json!([" A ", 3, ""]))),
                r#"["A"]"#,
            ),
            (
                "omit-empty",
                rows::omit_headers(Some(&json!([" "]))),
                "undefined",
            ),
            (
                "app-enabled",
                rows::app(Some(&json!({"code": null, "enabled": true}))),
                r#"{"enabled":true}"#,
            ),
            (
                "app-code",
                rows::app(Some(&json!({"code": "x", "enabled": false}))),
                r#"{"code":"x"}"#,
            ),
            (
                "app-none",
                rows::app(Some(&json!({"code": null, "enabled": false}))),
                "undefined",
            ),
        ]);
    }
}

mod auth_modes {
    use super::check_js;
    use crate::legacy::yml::auth::map_auth;
    use serde_json::{Value, json};

    #[test]
    fn auth_modes_match_the_reference() {
        let a = |v: Value| map_auth(Some(&v));
        check_js(vec![
            ("none", a(json!({"mode": "none"})), "undefined"),
            ("missing", map_auth(None), "undefined"),
            ("inherit", a(json!({"mode": "inherit"})), r#""inherit""#),
            (
                "basic",
                a(json!({"mode": "basic", "basic": {"username": "u", "password": null}})),
                r#"{"type":"basic","username":"u"}"#,
            ),
            (
                "bearer-bare",
                a(json!({"mode": "bearer"})),
                r#"{"type":"bearer"}"#,
            ),
            (
                "apikey",
                a(
                    json!({"mode": "apikey", "apikey": {"key": "k", "value": "v", "placement": "queryparams"}}),
                ),
                r#"{"type":"apikey","key":"k","value":"v","placement":"query"}"#,
            ),
            (
                "apikey-other",
                a(
                    json!({"mode": "apikey", "apikey": {"key": "k", "value": "", "placement": "cookie"}}),
                ),
                r#"{"type":"apikey","key":"k","value":""}"#,
            ),
            (
                "awsv4",
                a(
                    json!({"mode": "awsv4", "awsv4": {"accessKeyId": "a", "secretAccessKey": "s", "sessionToken": "", "service": "svc", "region": "r", "profileName": "p"}}),
                ),
                r#"{"type":"awsv4","accessKeyId":"a","secretAccessKey":"s","sessionToken":"","service":"svc","region":"r","profileName":"p"}"#,
            ),
            (
                "ntlm",
                a(
                    json!({"mode": "ntlm", "ntlm": {"username": "u", "password": "p", "domain": "d"}}),
                ),
                r#"{"type":"ntlm","username":"u","password":"p","domain":"d"}"#,
            ),
            (
                "wsse",
                a(json!({"mode": "wsse", "wsse": {"username": "u", "password": "p"}})),
                r#"{"type":"wsse","username":"u","password":"p"}"#,
            ),
            (
                "edgegrid",
                a(
                    json!({"mode": "akamai-edgegrid", "akamaiEdgegrid": {"accessToken": "at", "clientToken": "ct", "clientSecret": "cs", "baseURL": "b", "nonce": "", "timestamp": "t", "headersToSign": "h", "maxBodySize": 131072}}),
                ),
                r#"{"type":"akamai-edgegrid","accessToken":"at","clientToken":"ct","clientSecret":"cs","nonce":"","timestamp":"t","baseURL":"b","headersToSign":"h","maxBodySize":131072}"#,
            ),
            (
                "edgegrid-string-size",
                a(json!({"mode": "akamai-edgegrid", "akamaiEdgegrid": {"maxBodySize": "5"}})),
                r#"{"type":"akamai-edgegrid"}"#,
            ),
            (
                "oauth1",
                a(
                    json!({"mode": "oauth1", "oauth1": {"consumerKey": "ck", "consumerSecret": "cs", "accessToken": "at", "accessTokenSecret": "ats", "callbackUrl": "cb", "verifier": "v", "signatureMethod": "RSA-SHA1", "privateKeyType": "file", "privateKey": "key.pem", "timestamp": "", "nonce": "n", "version": "1.0", "realm": "r", "placement": "header", "includeBodyHash": false}}),
                ),
                r#"{"type":"oauth1","consumerKey":"ck","consumerSecret":"cs","accessToken":"at","accessTokenSecret":"ats","callbackUrl":"cb","verifier":"v","signatureMethod":"RSA-SHA1","privateKey":{"type":"file","value":"key.pem"},"timestamp":"","nonce":"n","version":"1.0","realm":"r","placement":"header","includeBodyHash":false}"#,
            ),
            ("unknown", a(json!({"mode": "kerberos"})), "undefined"),
            (
                "oauth2-code",
                a(
                    json!({"mode": "oauth2", "oauth2": {"grantType": "authorization_code", "callbackUrl": "cb", "authorizationUrl": "au", "accessTokenUrl": "tu", "refreshTokenUrl": "", "clientId": "id", "clientSecret": "sec", "credentialsPlacement": "body", "scope": "s", "state": "st", "pkce": false, "credentialsId": "credentials", "tokenPlacement": "header", "tokenHeaderPrefix": "Bearer", "tokenSource": "", "autoFetchToken": true, "autoRefreshToken": false, "additionalParameters": {"authorization": [{"name": " p ", "value": "v", "enabled": true, "sendIn": "queryparams"}, {"name": "", "value": "x", "sendIn": "headers"}], "token": [{"name": "t", "value": "", "enabled": true, "sendIn": "Body"}, {"name": "bad", "value": "x", "sendIn": "cookie"}]}}}),
                ),
                r#"{"type":"oauth2","flow":"authorization_code","authorizationUrl":"au","accessTokenUrl":"tu","callbackUrl":"cb","credentials":{"clientId":"id","clientSecret":"sec","placement":"body"},"additionalParameters":{"authorizationRequest":[{"name":"p","placement":"query","value":"v"}],"accessTokenRequest":[{"name":"t","placement":"body"}]},"scope":"s","state":"st","pkce":{"disabled":true},"tokenConfig":{"id":"credentials","placement":{"header":"Bearer"},"source":"access_token"},"settings":{"autoFetchToken":true,"autoRefreshToken":false}}"#,
            ),
            (
                "oauth2-code-pkce",
                a(
                    json!({"mode": "oauth2", "oauth2": {"grantType": "authorization_code", "pkce": true, "tokenPlacement": "header"}}),
                ),
                r#"{"type":"oauth2","flow":"authorization_code","pkce":{},"tokenConfig":{"placement":{},"source":"access_token"}}"#,
            ),
            (
                "oauth2-cc",
                a(
                    json!({"mode": "oauth2", "oauth2": {"grantType": "client_credentials", "accessTokenUrl": "tu", "clientId": "id", "clientSecret": "", "scope": "", "tokenPlacement": "url", "tokenQueryKey": "access_token", "tokenSource": "id_token", "autoFetchToken": "yes", "additionalParameters": {"refresh": [{"name": "r", "value": "1", "sendIn": "headers"}]}}}),
                ),
                r#"{"type":"oauth2","flow":"client_credentials","accessTokenUrl":"tu","credentials":{"clientId":"id"},"additionalParameters":{"refreshTokenRequest":[{"name":"r","placement":"header","value":"1"}]},"tokenConfig":{"placement":{"query":"access_token"},"source":"id_token"}}"#,
            ),
            (
                "oauth2-password",
                a(
                    json!({"mode": "oauth2", "oauth2": {"grantType": "password", "accessTokenUrl": "tu", "username": "u", "password": "", "clientId": "", "tokenPlacement": ""}}),
                ),
                r#"{"type":"oauth2","flow":"resource_owner_password_credentials","accessTokenUrl":"tu","resourceOwner":{"username":"u"},"tokenConfig":{"placement":{"header":""},"source":"access_token"}}"#,
            ),
            (
                "oauth2-implicit",
                a(
                    json!({"mode": "oauth2", "oauth2": {"grantType": "implicit", "authorizationUrl": "au", "callbackUrl": "cb", "clientId": "id", "clientSecret": "ignored", "scope": "s", "state": "", "tokenPlacement": "header", "additionalParameters": {"authorization": [{"name": "a", "value": "b", "sendIn": "headers"}], "token": [{"name": "t", "value": "1", "sendIn": "body"}]}}}),
                ),
                r#"{"type":"oauth2","flow":"implicit","authorizationUrl":"au","callbackUrl":"cb","credentials":{"clientId":"id"},"scope":"s","additionalParameters":{"authorizationRequest":[{"name":"a","placement":"header","value":"b"}]},"tokenConfig":{"placement":{},"source":"access_token"}}"#,
            ),
            (
                "oauth2-unknown-grant",
                a(json!({"mode": "oauth2", "oauth2": {"grantType": "device_code"}})),
                "undefined",
            ),
            ("oauth2-missing", a(json!({"mode": "oauth2"})), "undefined"),
        ]);
    }
}

impl Case {
    /// The item kind the reference dispatches on.
    pub(super) fn kind(&self) -> &str {
        match self.parsed.pointer("/meta/type").and_then(Value::as_str) {
            Some(k @ ("graphql" | "grpc" | "ws" | "app")) => k,
            _ => "http",
        }
    }

    pub(super) fn has_examples(&self) -> bool {
        self.parsed
            .get("examples")
            .and_then(Value::as_array)
            .is_some_and(|e| !e.is_empty())
    }
}

mod http_items {
    use super::*;
    use crate::legacy::yml::item::{ItemKind, parse_item};
    use crate::legacy::yml::request::http_document;
    use crate::legacy::yml::strip_bru_ext;

    #[test]
    fn strip_bru_ext_matches_the_reference_rewriter() {
        let rewritten: &[(&str, &str)] = &[
            (
                "await bru.runRequest(\"folder/one.bru\");\nawait bru.runRequest('folder/two.bru');\nawait bru.runRequest(`folder/three.bru`);",
                "await bru.runRequest(\"folder/one\");\nawait bru.runRequest('folder/two');\nawait bru.runRequest(`folder/three`);",
            ),
            (
                r#"bru.runRequest("a.bru"); bru.runRequest("b.bru");"#,
                r#"bru.runRequest("a"); bru.runRequest("b");"#,
            ),
            (
                "// bru.runRequest(\"c.bru\")\nbru.runRequest(\"real.bru\");",
                "// bru.runRequest(\"c.bru\")\nbru.runRequest(\"real\");",
            ),
            (
                r#"/* bru.runRequest("block.bru") */ bru.runRequest("real.bru");"#,
                r#"/* bru.runRequest("block.bru") */ bru.runRequest("real");"#,
            ),
            (
                "bru.runRequest\n(\n  \"spaced/out.bru\"\n)",
                "bru.runRequest\n(\n  \"spaced/out\"\n)",
            ),
            (
                r#"bru.runRequest ( "a.bru" ) ;"#,
                r#"bru.runRequest ( "a" ) ;"#,
            ),
            (
                r#"{ bru.runRequest("a.bru") }"#,
                r#"{ bru.runRequest("a") }"#,
            ),
            (
                r#"const msg = `Result: ${await bru.runRequest("nested/one.bru")}`;"#,
                r#"const msg = `Result: ${await bru.runRequest("nested/one")}`;"#,
            ),
            (
                r#"const s = `${`inner ${bru.runRequest("deep.bru")}`}`;"#,
                r#"const s = `${`inner ${bru.runRequest("deep")}`}`;"#,
            ),
            (
                r#"const half = total / 2; bru.runRequest("real.bru");"#,
                r#"const half = total / 2; bru.runRequest("real");"#,
            ),
            (
                "a = b\n/bru.runRequest(\"x.bru\")/g",
                "a = b\n/bru.runRequest(\"x\")/g",
            ),
            (
                r#"x = "\é"; bru.runRequest("é/ü.bru")"#,
                r#"x = "\é"; bru.runRequest("é/ü")"#,
            ),
        ];
        for (input, want) in rewritten {
            assert_eq!(strip_bru_ext(input), *want, "input {input:?}");
        }
        let untouched = [
            r#"const s = 'bru.runRequest("outer.bru")';"#,
            r#"const s = "bru.runRequest('outer.bru')";"#,
            r#"bru.runRequest("first.bru" + suffix)"#,
            r#"bru.runRequest("first.bru", { retries: 1 })"#,
            r#"bru.runRequest(getPath("first.bru"))"#,
            r#"bru.runRequest(`${prefix}` + ".bru")"#,
            r#"bru.runRequest("no-ext")"#,
            r#"bru.runRequest("looks/like.bruv")"#,
            r#"bru.runRequest("")"#,
            r#"notbru.runRequest("foo.bru"); mybru.runRequest("bar.bru");"#,
            r#"bru.runRequestX("a.bru")"#,
            r#"const s = `text bru.runRequest("outer.bru") text`;"#,
            r#"const re = /bru\.runRequest\("foo\.bru"\)/;"#,
            r#"if (source.match(/bru\.runRequest\("bar.bru"\)/)) {}"#,
            r#"const re = /[a-z]bru\.runRequest\("baz.bru"\)/g;"#,
            r#"function f() { return /bru\.runRequest\("kw.bru"\)/; }"#,
            r#"bru.runRequest("unterminated.bru)"#,
            "bru.runRequest(\"\n)",
            "bru.runRequest('\n)",
            "bru.runRequest(\"\r)",
            "const x = 1;\nfunction f() { return x + 2; }",
            "",
        ];
        for input in untouched {
            assert_eq!(strip_bru_ext(input), input, "input {input:?}");
        }
    }

    #[test]
    fn http_requests_without_examples_match_the_reference() {
        assert_mapped_files_match(
            &ALL_GROUPS,
            &|c| c.parser == "request" && c.kind() == "http" && !c.has_examples(),
            &|c| http_document(&parse_item(&c.parsed)),
        );
    }

    #[test]
    fn a_bare_http_request_gets_the_reference_defaults() {
        let item = parse_item(&json!({"meta": {"name": "  ", "type": "http", "seq": "0"}}));
        assert_eq!(item.kind, ItemKind::Http);
        let want = "info:\n  name: Untitled Request\n  type: http\n\nhttp:\n  method: GET\n  url: \"\"\n\nsettings:\n  encodeUrl: true\n  timeout: 0\n  followRedirects: true\n  maxRedirects: 5\n  forwardAuthorizationHeader: true\n";
        assert_eq!(to_bruno_yaml(&http_document(&item)), want);
    }
}

mod examples_and_other_items {
    use super::*;
    use crate::legacy::yml::item::parse_item;
    use crate::legacy::yml::map_request;
    use crate::legacy::yml::request::http_document;

    #[test]
    fn http_requests_with_examples_match_the_reference() {
        assert_mapped_files_match(
            &ALL_GROUPS,
            &|c| c.parser == "request" && c.kind() == "http" && c.has_examples(),
            &|c| http_document(&parse_item(&c.parsed)),
        );
    }

    #[test]
    fn graphql_grpc_ws_and_app_items_match_the_reference() {
        assert_mapped_files_match(
            &ALL_GROUPS,
            &|c| c.parser == "request" && c.kind() != "http",
            &|c| map_request(&c.parsed),
        );
    }

    #[test]
    fn every_request_file_maps_through_the_dispatcher() {
        assert_mapped_files_match(&ALL_GROUPS, &|c| c.parser == "request", &|c| {
            map_request(&c.parsed)
        });
    }

    #[test]
    fn item_defaults_follow_the_reference() {
        let cases = [
            (
                json!({"meta": {"name": "g", "type": "grpc", "seq": "2"}, "grpc": {"url": "localhost:50051", "method": "/pkg.Svc/Call", "methodType": "unary"}}),
                "info:\n  name: g\n  type: grpc\n  seq: 2\n\ngrpc:\n  url: localhost:50051\n  method: /pkg.Svc/Call\n  methodType: unary\n  message:\n    - title: message 1\n      message: \"{}\"\n",
            ),
            (
                json!({"meta": {"name": "w", "type": "ws", "seq": "1"}, "ws": {"url": "wss://x"}, "body": {"mode": "ws", "ws": [{"name": "hello", "content": "{\"a\":1}", "type": "json", "selected": true}]}, "settings": {"timeout": "5", "keepAliveInterval": "x"}}),
                "info:\n  name: w\n  type: websocket\n  seq: 1\n\nwebsocket:\n  url: wss://x\n  message:\n    - title: hello\n      selected: true\n      message:\n        type: json\n        data: '{\"a\":1}'\n\nsettings:\n  timeout: 5\n  keepAliveInterval: 0\n",
            ),
            (
                json!({"meta": {"name": "w", "type": "ws", "seq": "1"}, "ws": {"url": "wss://x"}, "body": {"mode": "ws", "ws": [{"name": "", "content": "hi"}]}}),
                "info:\n  name: w\n  type: websocket\n  seq: 1\n\nwebsocket:\n  url: wss://x\n  message:\n    type: text\n    data: hi\n\nsettings:\n  timeout: 0\n  keepAliveInterval: 0\n",
            ),
            (
                json!({"meta": {"name": "a", "type": "app", "seq": "abc", "tags": ["t"]}, "app": {"code": "<div/>"}}),
                "info:\n  name: a\n  type: app\n  seq: .nan\n  tags:\n    - t\ncode: <div/>\n",
            ),
            (
                json!({"meta": {"name": "q", "type": "graphql", "seq": 1}, "http": {"method": "post", "url": "u", "body": "graphql", "auth": "inherit"}, "body": {"graphql": {"query": "{ a }", "variables": "  "}}}),
                "info:\n  name: q\n  type: graphql\n  seq: 1\n\ngraphql:\n  method: POST\n  url: u\n  body:\n    query: \"{ a }\"\n  auth: inherit\n\nsettings:\n  encodeUrl: true\n  timeout: 0\n  followRedirects: true\n  maxRedirects: 5\n  forwardAuthorizationHeader: true\n",
            ),
        ];
        for (parsed, want) in cases {
            assert_eq!(to_bruno_yaml(&map_request(&parsed)), want, "{parsed}");
        }
    }
}

/// The `bruno.json` the per-file migration placed beside `input`: its sibling source, else the default.
fn sibling_bruno_json(input: &str) -> Value {
    let dir = Path::new(input)
        .parent()
        .expect("inputs sit in a group directory");
    let sibling = corpus().join("sources").join(dir).join("bruno.json");
    if sibling.is_file() {
        read_json(&sibling)
    } else {
        json!({"version": "1", "name": "corpus", "type": "collection"})
    }
}

mod folders_and_collections {
    use super::*;
    use crate::legacy::yml::{map_collection, map_folder};

    #[test]
    fn folder_and_collection_files_match_the_reference() {
        assert_mapped_files_match(&ALL_GROUPS, &|c| c.parser == "collection", &|c| {
            if c.input.ends_with("/collection.bru") {
                map_collection(Some(&c.parsed), &sibling_bruno_json(&c.input))
                    .expect("corpus config converts")
            } else {
                map_folder(&c.parsed)
            }
        });
    }

    #[test]
    fn folder_defaults_follow_the_reference() {
        let cases = [
            (
                json!({"meta": {"name": "f", "seq": "3", "tags": [" a ", "a", ""]}}),
                "info:\n  name: f\n  type: folder\n  seq: 3\n  tags:\n    - a\n\nrequest: {}\n",
            ),
            (
                json!({"meta": {"name": "", "seq": "x"}, "auth": {"mode": "none"}, "script": {"req": " "}, "docs": "  "}),
                "info:\n  name: Untitled Folder\n  type: folder\n  seq: 1\n\nrequest: {}\n",
            ),
            (
                json!({"meta": {"name": "f"}, "auth": {"mode": "inherit"}, "headers": [{"name": "h", "value": "v", "enabled": true}], "vars": {"req": [{"name": "a", "value": "1", "enabled": true}], "res": [{"name": "b", "value": "res.body", "enabled": true, "local": true}]}, "script": {"req": "bru.runRequest(\"x.bru\")"}, "tests": "t", "docs": "# Doc"}),
                "info:\n  name: f\n  type: folder\n\nrequest:\n  headers:\n    - name: h\n      value: v\n  auth: inherit\n  variables:\n    - name: a\n      value: \"1\"\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body\n        method: jsonq\n      variable:\n        name: b\n        scope: request\n  scripts:\n    - type: before-request\n      code: bru.runRequest(\"x\")\n    - type: tests\n      code: t\n\ndocs:\n  content: \"# Doc\"\n  type: text/markdown\n",
            ),
        ];
        for (parsed, want) in cases {
            assert_eq!(to_bruno_yaml(&map_folder(&parsed)), want, "{parsed}");
        }
    }

    #[test]
    fn bruno_json_settings_follow_the_reference() {
        let config = json!({
            "version": "1", "name": "legacy", "type": "collection", "collectionVersion": 2,
            "proxy": {"enabled": false, "protocol": "https", "hostname": "proxy.local", "auth": {"enabled": false, "username": "u"}, "bypassProxy": "localhost"},
            "protobuf": {"protoFiles": [{"path": "a.proto"}, {}], "importPaths": [{"path": "protos", "enabled": false}, {"path": "more"}]},
            "clientCertificates": {"enabled": true, "certs": [
                {"domain": "a.com", "type": "cert", "certFilePath": "c.pem", "keyFilePath": "k.pem", "passphrase": ""},
                {"domain": "b.com", "type": "pfx", "pfxFilePath": "b.pfx", "passphrase": "pw", "disabled": true},
                {"domain": "c.com", "type": "jks"}]},
            "ignore": ["node_modules", ".git", "vendor"],
            "presets": {"requestType": "graphql", "requestUrl": "", "defaultEnvironment": "Local"},
            "scripts": {"moduleWhitelist": ["crypto"], "flow": "sequential", "additionalContextRoots": []},
            "openapi": [{"sourceUrl": "https://x/spec.json", "groupBy": "tags", "autoCheck": false}, {"sourceUrl": ""}]
        });
        let want = "opencollection: 1.0.0\n\ninfo:\n  name: legacy\n  version: \"2\"\nconfig:\n  protobuf:\n    protoFiles:\n      - type: file\n        path: a.proto\n      - type: file\n    importPaths:\n      - path: protos\n        disabled: true\n      - path: more\n  proxy:\n    inherit: false\n    config:\n      protocol: https\n      hostname: proxy.local\n      port: \"\"\n      auth:\n        username: u\n        password: \"\"\n        disabled: true\n      bypassProxy: localhost\n    disabled: true\n  clientCertificates:\n    - domain: a.com\n      type: pem\n      certificateFilePath: c.pem\n      privateKeyFilePath: k.pem\n    - domain: b.com\n      type: pkcs12\n      pkcs12FilePath: b.pfx\n      passphrase: pw\n      disabled: true\nbundled: false\nextensions:\n  bruno:\n    ignore:\n      - node_modules\n      - .git\n      - vendor\n    presets:\n      request:\n        type: graphql\n      defaultEnvironment: Local\n    scripts:\n      flow: sequential\n    openapi:\n      - sourceUrl: https://x/spec.json\n        groupBy: tags\n        autoCheck: false\n        autoCheckInterval: 5\n";
        assert_eq!(
            to_bruno_yaml(&map_collection(None, &config).expect("converts")),
            want
        );
        let global = json!({"opencollection": "1.0.0", "name": "", "proxy": {"enabled": "global"}});
        let want = "opencollection: 1.0.0\n\ninfo:\n  name: Untitled Collection\nconfig:\n  proxy:\n    inherit: true\n    config:\n      protocol: http\n      hostname: \"\"\n      port: \"\"\n      auth:\n        username: \"\"\n        password: \"\"\n      bypassProxy: \"\"\nbundled: false\nextensions: {}\n";
        assert_eq!(
            to_bruno_yaml(&map_collection(None, &global).expect("converts")),
            want
        );
    }
}

mod whole_collections {
    use super::*;
    use crate::legacy::bru::{BruError, parse_collection, parse_environment, parse_request};
    use crate::legacy::yml::{
        FileError, map_collection, map_environment, map_folder, map_request, plan_files,
    };
    use std::collections::BTreeMap;

    fn yml_tree(dir: &Path, root: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).expect("readable expected/ directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                yml_tree(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).expect("under root").to_path_buf();
                out.insert(rel, fs::read_to_string(&path).expect("UTF-8 YAML"));
            }
        }
    }

    #[test]
    fn whole_collections_convert_to_the_reference_trees() {
        let sources = corpus().join("sources/collections");
        let mut names: Vec<String> = fs::read_dir(&sources)
            .expect("collections directory")
            .map(|e| {
                e.expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        assert!(names.len() >= 2, "corpus collections: {names:?}");
        let mut failures = Vec::new();
        for name in &names {
            let planned =
                plan_files(&sources.join(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let got: BTreeMap<PathBuf, String> = planned
                .iter()
                .map(|(p, m)| (p.clone(), to_bruno_yaml(m)))
                .collect();
            let root = corpus().join("expected/collections").join(name);
            let mut want = BTreeMap::new();
            yml_tree(&root, &root, &mut want);
            if got.keys().ne(want.keys()) {
                failures.push(format!(
                    "{name}: file sets differ\n  want {:?}\n  got  {:?}",
                    want.keys(),
                    got.keys()
                ));
                continue;
            }
            for (path, text) in &want {
                if got[path] != *text {
                    failures.push(format!(
                        "{name}/{}:\n{}",
                        path.display(),
                        first_difference(text, &got[path])
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} differences:\n{}",
            failures.len(),
            failures
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn the_reader_and_the_mapper_reproduce_every_per_file_output() {
        assert_mapped_files_match(&ALL_GROUPS, &|_| true, &|c| {
            let text = fs::read_to_string(corpus().join("sources").join(&c.input)).expect("source");
            let ok =
                |r: Result<Value, BruError>| r.unwrap_or_else(|e| panic!("{}: {e:?}", c.input));
            match c.parser.as_str() {
                "environment" => map_environment(&ok(parse_environment(&text))),
                "collection" if c.input.ends_with("/collection.bru") => map_collection(
                    Some(&ok(parse_collection(&text))),
                    &sibling_bruno_json(&c.input),
                )
                .expect("corpus config converts"),
                "collection" => map_folder(&ok(parse_collection(&text))),
                _ => map_request(&ok(parse_request(&text))),
            }
        });
    }

    const REQUEST: &str =
        "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n";
    const MALFORMED: &str = "meta {\n  name: a\n";

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().expect("has a parent")).expect("create directories");
        fs::write(path, text).expect("write file");
    }

    #[test]
    fn the_walk_follows_the_reference_rules() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("c");
        write(
            &root,
            "bruno.json",
            r#"{"version": "1", "name": "w", "type": "collection", "ignore": ["vendor"]}"#,
        );
        write(&root, "a.bru", REQUEST);
        write(
            &root,
            "environments/Local.bru",
            "vars {\n  host: http://localhost\n}\n",
        );
        write(&root, "environments/nested/x.bru", REQUEST);
        write(&root, "f/folder.bru", "meta {\n  name: f\n}\n");
        write(&root, "notes.txt", "not a legacy file\n");
        for skipped in [
            "vendor/v.bru",
            "deep/vendor/v.bru",
            "node_modules/n.bru",
            ".git/g.bru",
        ] {
            write(&root, skipped, MALFORMED);
        }
        #[cfg(unix)]
        {
            write(temp.path(), "outside/o.bru", REQUEST);
            std::os::unix::fs::symlink(temp.path().join("outside/o.bru"), root.join("link.bru"))
                .expect("file symlink");
            std::os::unix::fs::symlink(temp.path().join("outside"), root.join("linked"))
                .expect("directory symlink");
        }
        let planned = plan_files(&root).unwrap_or_else(|e| panic!("{e:?}"));
        let texts: BTreeMap<String, String> = planned
            .iter()
            .map(|(p, m)| (p.to_string_lossy().replace('\\', "/"), to_bruno_yaml(m)))
            .collect();
        let targets: Vec<String> = planned
            .iter()
            .map(|(p, _)| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(
            targets,
            [
                "a.yml",
                "environments/Local.yml",
                "environments/nested/x.yml",
                "f/folder.yml",
                "opencollection.yml"
            ]
        );
        assert!(
            texts["environments/Local.yml"].starts_with("variables:\n"),
            "{texts:?}"
        );
        assert!(
            texts["environments/nested/x.yml"].starts_with("info:\n  name: a\n  type: http\n"),
            "{texts:?}"
        );
        assert!(
            texts["f/folder.yml"].starts_with("info:\n  name: f\n  type: folder\n"),
            "{texts:?}"
        );
        assert!(
            texts["opencollection.yml"].contains("    ignore:\n      - vendor\n"),
            "{texts:?}"
        );
    }

    /// The reference throws on `.map` or `.forEach` of a non-empty string where the config
    /// expects an array, so each such field fails the plan naming itself.
    #[test]
    fn string_config_lists_fail_naming_the_field() {
        let cases = [
            (
                r#""protobuf": {"protoFiles": "a.proto"}"#,
                "protobuf.protoFiles",
            ),
            (
                r#""protobuf": {"importPaths": "protos"}"#,
                "protobuf.importPaths",
            ),
            (
                r#""clientCertificates": {"certs": "c.pem"}"#,
                "clientCertificates.certs",
            ),
            (r#""ignore": "vendor""#, "ignore"),
        ];
        for (field_json, field) in cases {
            let temp = tempfile::tempdir().expect("temp dir");
            let root = temp.path();
            write(
                root,
                "bruno.json",
                &format!(r#"{{"version": "1", "name": "s", "type": "collection", {field_json}}}"#),
            );
            let errors = plan_files(root).expect_err(field);
            assert_eq!(errors.len(), 1, "{field}: {errors:?}");
            assert_eq!(errors[0].path, Path::new("bruno.json"), "{field}");
            assert!(errors[0].message.contains(field), "{field}: {errors:?}");
        }
    }

    /// The reference reports no line for a `bruno.json` that fails `JSON.parse`; the plan names
    /// the line of the failure instead.
    #[test]
    fn an_invalid_bruno_json_names_its_line_where_the_reference_has_none() {
        let temp = tempfile::tempdir().expect("temp dir");
        write(temp.path(), "bruno.json", "{\n  \"name\": \"x\",\n}\n");
        write(temp.path(), "a.bru", REQUEST);
        let errors = plan_files(temp.path()).expect_err("invalid JSON");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].path, Path::new("bruno.json"));
        assert_eq!(errors[0].line, 3, "{errors:?}");
    }

    /// A file that is not UTF-8 cannot be read as text, so it fails at line 0.
    #[test]
    fn a_file_that_is_not_utf8_fails_at_line_zero() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        write(
            root,
            "bruno.json",
            r#"{"version": "1", "name": "u", "type": "collection"}"#,
        );
        fs::write(root.join("bad.bru"), b"meta {\n  name: \xff\xfe\n}\n").expect("write file");
        let errors = plan_files(root).expect_err("not UTF-8");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].path, Path::new("bad.bru"));
        assert_eq!(errors[0].line, 0);
        assert!(!errors[0].message.is_empty());

        fs::write(root.join("bruno.json"), b"{\"name\": \"\xff\"}").expect("write file");
        let errors = plan_files(root).expect_err("bruno.json is not UTF-8");
        assert_eq!(errors[0].path, Path::new("bruno.json"));
        assert_eq!(errors[0].line, 0);
    }

    #[test]
    fn every_failing_file_is_reported_with_its_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path();
        write(
            root,
            "bruno.json",
            r#"{"version": "1", "name": "e", "type": "collection"}"#,
        );
        write(root, "ok.bru", REQUEST);
        write(root, "bad/one.bru", MALFORMED);
        write(root, "two.bru", "nonsense {\n}\n");
        write(root, "collection.bru", "bogus {\n}\n");
        let errors: Vec<FileError> = plan_files(root).expect_err("three files fail");
        let paths: Vec<String> = errors
            .iter()
            .map(|e| e.path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(paths, ["bad/one.bru", "collection.bru", "two.bru"]);
        assert!(
            errors.iter().all(|e| e.line >= 1 && !e.message.is_empty()),
            "{errors:?}"
        );

        let bare = tempfile::tempdir().expect("temp dir");
        write(bare.path(), "a.bru", REQUEST);
        let missing = plan_files(bare.path()).expect_err("bruno.json is required");
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert_eq!(missing[0].path, PathBuf::from("bruno.json"));
    }
}
