//! Request snapshots: a request's mapping as JSON, literal auth credentials blanked.

use crate::tree_support;

use std::path::Path;

use apinata_core::{Collection, DraftSnapshot, Folder, ItemPath, TreeError};
use tree_support::{ROOT, folder, ip, open};

fn http(name: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x\n{extra}"
    )
}

fn round_trips(snap: &DraftSnapshot) {
    let back = DraftSnapshot::from_json(&snap.to_json()).expect("parses");
    assert!(back == *snap, "{}", snap.to_json());
}

// The snapshot's auth mapping, after checking it survives a JSON round trip.
fn auth_after(auth: &str) -> serde_json::Value {
    let (_dir, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &http("r", &format!("  auth:\n{auth}"))),
    ]);
    let snap = c.request_snapshot(&ip("r.yml")).expect("a request");
    round_trips(&snap);
    let json: serde_json::Value = serde_json::from_str(&snap.to_json()).expect("json");
    json["http"]["auth"].clone()
}

#[test]
fn literal_credentials_are_blanked_and_everything_else_is_kept() {
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "    type: ntlm\n    username: u\n    password: lit\n    domain: CORP\n",
            &[("/username", "u"), ("/password", ""), ("/domain", "CORP")],
        ),
        (
            "    type: ntlm\n    username: u\n    password: \"{{pw}}\"\n    domain: CORP\n",
            &[("/password", "{{pw}}")],
        ),
        (
            "    type: basic\n    username: u\n    password: lit\n",
            &[("/username", "u"), ("/password", "")],
        ),
        (
            "    type: basic\n    username: u\n    password: 1234\n",
            &[("/password", "")],
        ),
        (
            "    type: basic\n    username: u\n    password: \"{{pw}}\"\n",
            &[("/password", "{{pw}}")],
        ),
        ("    type: bearer\n    token: lit\n", &[("/token", "")]),
        (
            "    type: bearer\n    token: \"{{tok}}\"\n",
            &[("/token", "{{tok}}")],
        ),
        (
            "    type: bearer\n    token: \"Bearer {{t}}\"\n",
            &[("/token", "")],
        ),
        (
            "    type: bearer\n    token: \"{{a}}{{b}}\"\n",
            &[("/token", "")],
        ),
        (
            "    type: apikey\n    key: X-Key\n    value: lit\n    placement: header\n",
            &[("/key", "X-Key"), ("/value", ""), ("/placement", "header")],
        ),
        (
            "    type: digest\n    username: u\n    password: lit\n",
            &[("/username", "u"), ("/password", "")],
        ),
        (
            "    type: wsse\n    username: u\n    password: lit\n",
            &[("/password", "")],
        ),
        (
            "    type: awsv4\n    accessKeyId: AK\n    secretAccessKey: lit\n    sessionToken: lit\n    region: eu\n",
            &[
                ("/accessKeyId", "AK"),
                ("/secretAccessKey", ""),
                ("/sessionToken", ""),
                ("/region", "eu"),
            ],
        ),
        (
            "    type: oauth1\n    consumerKey: ck\n    consumerSecret: lit\n    accessToken: lit\n    accessTokenSecret: lit\n    privateKey: lit\n",
            &[
                ("/consumerKey", "ck"),
                ("/consumerSecret", ""),
                ("/accessToken", ""),
                ("/accessTokenSecret", ""),
                ("/privateKey", ""),
            ],
        ),
        (
            "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://x/t\n    credentials:\n      clientId: id\n      clientSecret: lit\n",
            &[
                ("/accessTokenUrl", "http://x/t"),
                ("/credentials/clientId", "id"),
                ("/credentials/clientSecret", ""),
            ],
        ),
        (
            "    type: oauth2\n    flow: password\n    accessTokenUrl: http://x/t\n    resourceOwner:\n      username: u\n      password: lit\n",
            &[
                ("/resourceOwner/username", "u"),
                ("/resourceOwner/password", ""),
            ],
        ),
        (
            "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://x/t\n    additionalParameters:\n      accessTokenRequest:\n        - name: aud\n          placement: body\n          value: lit\n        - name: tenant\n          placement: header\n          value: \"{{tenant}}\"\n",
            &[
                ("/additionalParameters/accessTokenRequest/0/name", "aud"),
                ("/additionalParameters/accessTokenRequest/0/value", ""),
                (
                    "/additionalParameters/accessTokenRequest/1/value",
                    "{{tenant}}",
                ),
            ],
        ),
        (
            "    type: akamai-edgegrid\n    accessToken: lit\n    clientToken: lit\n    clientSecret: lit\n    baseURL: http://x\n",
            &[
                ("/accessToken", ""),
                ("/clientToken", ""),
                ("/clientSecret", ""),
                ("/baseURL", "http://x"),
            ],
        ),
    ];
    for (auth, want) in cases {
        let got = auth_after(auth);
        for (pointer, value) in *want {
            assert_eq!(
                got.pointer(pointer).and_then(|v| v.as_str()),
                Some(*value),
                "{pointer} in\n{auth}"
            );
        }
    }
}

#[test]
fn a_graphql_requests_credentials_are_blanked_too() {
    let text = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: http://x\n  body:\n    query: \"{ a }\"\n  auth:\n    type: bearer\n    token: lit\n";
    let (_dir, c) = open(&[("opencollection.yml", ROOT), ("g.yml", text)]);
    let snap = c.request_snapshot(&ip("g.yml")).expect("a request");
    round_trips(&snap);
    let json: serde_json::Value = serde_json::from_str(&snap.to_json()).unwrap();
    assert_eq!(json["graphql"]["auth"]["token"], "");
    assert_eq!(json["graphql"]["body"]["query"], "{ a }");
}

#[test]
fn every_body_kind_and_plain_scalar_survives_a_json_round_trip() {
    let bodies = [
        "  body:\n    type: json\n    data: '{\"a\": 1}'\n",
        "  body:\n    type: text\n    data: plain\n",
        "  body:\n    type: xml\n    data: <a/>\n",
        "  body:\n    type: form-urlencoded\n    data:\n      - name: a\n        value: '1'\n        disabled: true\n",
        "  body:\n    type: multipart-form\n    data:\n      - name: a\n        type: text\n        value: x\n      - name: f\n        type: file\n        value:\n          - /tmp/f.bin\n",
        "  body:\n    type: file\n    data:\n      - filePath: /tmp/f.bin\n        contentType: application/octet-stream\n        selected: true\n",
    ];
    let tail = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"201\"\nsettings:\n  timeout: 0\n  followRedirects: true\n  maxRedirects: 5\n  ratio: 1.5\ndocs: ~\n";
    for body in bodies {
        let text = format!("{}{body}{tail}", http("r", ""));
        let (_dir, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
        round_trips(&c.request_snapshot(&ip("r.yml")).expect("a request"));
    }
}

fn request_items(c: &Collection) -> Vec<ItemPath> {
    fn walk(c: &Collection, f: &Folder, out: &mut Vec<ItemPath>) {
        out.extend(f.requests().iter().filter_map(|r| c.item_path(r.path())));
        for sub in f.folders() {
            walk(c, sub, out);
        }
    }
    let mut out = Vec::new();
    walk(c, c.tree(), &mut out);
    out
}

#[test]
fn every_vendored_request_survives_a_json_round_trip() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots = [
        base.join("../../examples/demo"),
        base.join("tests/fixtures/bruno/cli-docs"),
        base.join("tests/fixtures/bruno/bruno-tests"),
        base.join("tests/fixtures/bruno/env-inheritance"),
    ];
    let mut seen = 0;
    for root in roots {
        let c = Collection::open(&root).expect("opens");
        for item in request_items(&c) {
            let snap = c.request_snapshot(&item).expect("a request");
            let back = DraftSnapshot::from_json(&snap.to_json())
                .unwrap_or_else(|e| panic!("{} {}: {e}", root.display(), item.as_str()));
            assert!(back == snap, "{} {} changed", root.display(), item.as_str());
            seen += 1;
        }
    }
    assert_eq!(seen, 28, "every request file of the four collections");
}

#[test]
fn only_requests_have_a_snapshot() {
    let (_dir, c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("users", 1)),
    ]);
    assert!(c.request_snapshot(&ip("users")).is_none());
    assert!(c.request_snapshot(&ip("gone.yml")).is_none());
}

#[test]
fn map_strings_rewrites_values_and_never_keys() {
    let (_dir, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &http("r", ""))]);
    let mut snap = c.request_snapshot(&ip("r.yml")).unwrap();
    snap.map_strings(|s| s.to_uppercase());
    let json: serde_json::Value = serde_json::from_str(&snap.to_json()).unwrap();
    assert_eq!(json["http"]["url"], "HTTP://X");
    assert_eq!(json["http"]["method"], "GET");
    assert!(json.get("HTTP").is_none());
}

#[test]
fn from_json_refuses_anything_but_a_request_mapping() {
    for bad in [
        "not json",
        "null",
        "[]",
        r#"{"info":{"type":"http"}}"#,
        r#"{"info":{"type":"graphql"},"http":{}}"#,
        r#"{"info":{"type":"websocket"},"http":{}}"#,
    ] {
        assert!(DraftSnapshot::from_json(bad).is_err(), "{bad}");
    }
    assert!(
        DraftSnapshot::from_json(r#"{"info":{"name":"d","type":"http"},"http":{"url":"x"}}"#)
            .is_ok()
    );
}

#[test]
fn a_draft_from_a_snapshot_holds_its_mapping_unsaved() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("users", 1)),
        (
            "r.yml",
            &http("r", "  body:\n    type: text\n    data: hi\n"),
        ),
    ]);
    let snap = c.request_snapshot(&ip("r.yml")).unwrap();
    let draft = c.create_draft_from(&ip("users"), &snap).expect("created");
    assert!(draft.is_draft(), "{draft:?}");
    assert_eq!(draft.parent(), Some(ip("users")));
    assert!(c.request_snapshot(&draft).unwrap() == snap);
    assert!(!c.request(&draft).unwrap().is_dirty());
    assert!(
        !dir.path().join(draft.as_str()).exists(),
        "nothing is written"
    );
    assert!(matches!(
        c.create_draft_from(&ip("nowhere"), &snap),
        Err(TreeError::NotFound { .. })
    ));
}

#[test]
fn a_draft_from_a_snapshot_takes_a_free_name_when_its_own_is_held() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let first = c.create_draft(&ItemPath::root()).unwrap();
    let snap = c.request_snapshot(&first).unwrap();
    let second = c.create_draft_from(&ItemPath::root(), &snap).unwrap();
    assert_ne!(first, second);
    assert_eq!(c.request(&first).unwrap().name(), "Untitled request");
    assert_eq!(c.request(&second).unwrap().name(), "Untitled request 2");
}

// A secret equal to a format token (a method, a body mode, the request type) must not rewrite
// the token, or the snapshot stops parsing and the request cannot be reopened.
#[test]
fn map_strings_leaves_method_and_type_tokens_alone() {
    let (_dir, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &http(
                "r",
                "  body:\n    type: json\n    data: '{\"a\":\"json\"}'\n",
            ),
        ),
    ]);
    let mut snap = c.request_snapshot(&ip("r.yml")).unwrap();
    snap.map_strings(|s| {
        ["GET", "json", "http"]
            .iter()
            .fold(s.to_owned(), |t, secret| t.replace(secret, "[redacted]"))
    });
    let back = DraftSnapshot::from_json(&snap.to_json()).expect("still parses");
    let json: serde_json::Value = serde_json::from_str(&back.to_json()).unwrap();
    assert_eq!(json["info"]["type"], "http");
    assert_eq!(json["http"]["method"], "GET");
    assert_eq!(json["http"]["body"]["type"], "json");
    assert_eq!(json["http"]["url"], "[redacted]://x");
    assert_eq!(json["http"]["body"]["data"], "{\"a\":\"[redacted]\"}");
}

// The snapshot's url and whether it blanked anything, for a request sent to `url`.
fn url_after(url: &str) -> (String, bool) {
    let text = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n  params:\n    - name: q\n      value: a:b@c\n"
    );
    let (_dir, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let (snap, blanked) = c
        .request(&ip("r.yml"))
        .expect("a request")
        .snapshot_and_blanks();
    round_trips(&snap);
    let json: serde_json::Value = serde_json::from_str(&snap.to_json()).expect("json");
    assert_eq!(
        json["http"]["params"][0]["value"], "a:b@c",
        "other fields kept"
    );
    (
        json["http"]["url"].as_str().expect("a url").to_owned(),
        blanked,
    )
}

#[test]
fn a_literal_url_password_is_blanked_and_flagged() {
    assert_eq!(
        url_after("http://u:hunter2pw@h.test/p?x=1#f"),
        ("http://u@h.test/p?x=1#f".to_owned(), true)
    );
}

#[test]
fn a_lone_placeholder_url_password_is_kept_unflagged() {
    let url = "http://u:{{pw}}@h.test/p";
    assert_eq!(url_after(url), (url.to_owned(), false));
}

#[test]
fn a_url_without_a_password_is_unchanged_and_unflagged() {
    for url in ["http://u@h.test/p", "http://u:@h.test/p", "http://h.test/p"] {
        assert_eq!(url_after(url), (url.to_owned(), false), "{url}");
    }
}

#[test]
fn a_password_with_an_empty_user_drops_the_whole_userinfo() {
    assert_eq!(
        url_after("http://:pw@h.test/p"),
        ("http://h.test/p".to_owned(), true)
    );
}

#[test]
fn a_placeholder_scheme_or_no_scheme_still_finds_the_password() {
    assert_eq!(
        url_after("{{proto}}://u:p@h/x"),
        ("{{proto}}://u@h/x".to_owned(), true)
    );
    assert_eq!(url_after("u:p@h/x"), ("u@h/x".to_owned(), true));
}

#[test]
fn an_at_sign_past_the_authority_is_not_userinfo() {
    for url in [
        "http://h.test/a:b@c",
        "http://h.test?a:b@c",
        "http://h.test#a:b@c",
    ] {
        assert_eq!(url_after(url), (url.to_owned(), false), "{url}");
    }
}
