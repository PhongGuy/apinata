//! HAR captures: entries, folders per host, credentials, headers and bodies.

use apinata_core::import::{
    CollectionFormat, CollectionOptions, DocumentError, FormatError, ImportBundle, ImportNote,
    ImportedCollection, ImportedFolder, ImportedItem, ImportedRequestItem, NoteKind,
    parse_collections, write_collection,
};
use apinata_core::{
    AuthSpec, BodySpec, EnvVariable, KeyValue, MemoryStore, MultipartField, Param, ParamKind,
    VarValue,
};

fn parse_with(input: &str, options: &CollectionOptions) -> Result<Vec<ImportBundle>, FormatError> {
    parse_collections(Some(CollectionFormat::Har), input.as_bytes(), options)
}

fn one(input: &str) -> ImportBundle {
    let mut bundles = parse_with(input, &CollectionOptions::default()).expect("parses");
    assert_eq!(bundles.len(), 1);
    bundles.remove(0)
}

/// A HAR 1.2 log holding `entries`, each already JSON.
fn log(entries: &[String]) -> String {
    format!(
        r#"{{"log":{{"version":"1.2","creator":{{"name":"WebInspector","version":"1"}},"pages":[],"entries":[{}]}}}}"#,
        entries.join(",")
    )
}

/// An entry whose request has `method`, `url` and the `extra` members (each starting with a comma).
fn entry(method: &str, url: &str, extra: &str) -> String {
    format!(
        r#"{{"startedDateTime":"2026-10-10T00:00:00Z","time":1,"request":{{"method":"{method}","url":"{url}","httpVersion":"HTTP/2","headersSize":-1,"bodySize":0{extra}}},"response":{{"status":200}},"cache":{{}},"timings":{{"wait":1}}}}"#
    )
}

fn folder<'a>(c: &'a ImportedCollection, name: &str) -> &'a ImportedFolder {
    c.items
        .iter()
        .find_map(|i| match i {
            ImportedItem::Folder(f) if f.name == name => Some(f),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no folder {name}"))
}

fn request<'a>(f: &'a ImportedFolder, name: &str) -> &'a ImportedRequestItem {
    f.items
        .iter()
        .find_map(|i| match i {
            ImportedItem::Request(r) if r.name == name => Some(r),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no request {name}"))
}

/// The only request of a bundle with one folder holding one request.
fn only(b: &ImportBundle) -> &ImportedRequestItem {
    let [ImportedItem::Folder(f)] = &b.collection.items[..] else {
        panic!("one folder")
    };
    let [ImportedItem::Request(r)] = &f.items[..] else {
        panic!("one request")
    };
    r
}

/// Every note: the collection's, the unwritten ones, then each item's depth first.
fn all_notes(c: &ImportedCollection) -> Vec<&ImportNote> {
    fn walk<'a>(items: &'a [ImportedItem], out: &mut Vec<&'a ImportNote>) {
        for item in items {
            match item {
                ImportedItem::Folder(f) => {
                    out.extend(&f.notes);
                    walk(&f.items, out);
                }
                ImportedItem::Request(r) => out.extend(&r.notes),
                _ => unreachable!("only folders and requests exist"),
            }
        }
    }
    let mut out: Vec<&ImportNote> = c.notes.iter().chain(&c.unwritten).collect();
    walk(&c.items, &mut out);
    out
}

fn sources(notes: &[&ImportNote], kind: NoteKind) -> Vec<String> {
    notes
        .iter()
        .filter(|n| n.kind == kind)
        .map(|n| n.source.clone())
        .collect()
}

#[test]
fn a_capture_is_one_collection_with_a_folder_per_host() {
    let b = one(&log(&[
        entry("GET", "https://api.shop.test/users?page=2", ""),
        entry("post", "http://localhost:8080/login", ""),
        entry("GET", "https://API.shop.test:443/users/7", ""),
        entry("DELETE", "https://api.shop.test:8443/x", ""),
    ]));
    let c = &b.collection;
    assert_eq!(c.name, "HAR capture");
    assert_eq!(c.auth, AuthSpec::None);
    assert!(b.environments.is_empty());
    let folders: Vec<(&str, u64)> = c
        .items
        .iter()
        .map(|i| match i {
            ImportedItem::Folder(f) => (f.name.as_str(), f.seq),
            _ => panic!("only folders at the root"),
        })
        .collect();
    assert_eq!(
        folders,
        [
            ("api.shop.test", 1),
            ("localhost:8080", 2),
            ("api.shop.test:8443", 3)
        ]
    );
    let api = folder(c, "api.shop.test");
    let names: Vec<(&str, u64)> = api
        .items
        .iter()
        .map(|i| match i {
            ImportedItem::Request(r) => (r.name.as_str(), r.seq),
            _ => panic!("only requests in a host folder"),
        })
        .collect();
    assert_eq!(names, [("GET /users", 1), ("GET /users/7", 2)]);
    assert_eq!(
        request(api, "GET /users/7").request.url,
        "https://API.shop.test:443/users/7"
    );
    let login = request(folder(c, "localhost:8080"), "POST /login");
    assert_eq!(login.request.method, "POST");
    assert_eq!(login.request.auth, AuthSpec::None);
}

#[test]
fn the_url_is_kept_and_params_come_from_its_query_undecoded() {
    let b = one(&log(&[entry(
        "GET",
        "https://x.test/s?page=2&q=a%20b&flag",
        r#","queryString":[{"name":"other","value":"1"}]"#,
    )]));
    let r = &only(&b).request;
    assert_eq!(r.url, "https://x.test/s?page=2&q=a%20b&flag");
    let want = [
        Param::new("page", "2", ParamKind::Query),
        Param::new("q", "a%20b", ParamKind::Query),
        Param::new("flag", "", ParamKind::Query),
    ];
    assert_eq!(r.params, want);
    assert!(only(&b).notes.is_empty(), "{:?}", only(&b).notes);
}

#[test]
fn names_come_from_the_entry_comment_then_the_request_comment() {
    let b = one(&log(&[
        r#"{"comment":"Load cart","request":{"method":"GET","url":"https://x.test/cart","comment":"no"}}"#.to_owned(),
        entry("GET", "https://x.test/a", r#","comment":"From request""#),
        r#"{"comment":"  ","request":{"method":"get","url":"https://x.test/?q=1"}}"#.to_owned(),
    ]));
    let f = folder(&b.collection, "x.test");
    let names: Vec<&str> = f
        .items
        .iter()
        .map(|i| match i {
            ImportedItem::Request(r) => r.name.as_str(),
            _ => panic!("request"),
        })
        .collect();
    assert_eq!(names, ["Load cart", "From request", "GET /"]);
}

#[test]
fn entries_that_are_not_http_requests_are_unwritten() {
    let b = one(&log(&[
        r#"{"request":{"method":"GET","url":"data:text/plain,hi"}}"#.to_owned(),
        r#"{"request":{"method":"GET","url":"wss://x.test/live"}}"#.to_owned(),
        r#"{"request":{"method":"POST","url":""}}"#.to_owned(),
        r#"{"response":{}}"#.to_owned(),
        r#"{"request":"GET /"}"#.to_owned(),
        r#"{"request":{"method":"GET","url":"https://x.test/h","headers":"Bearer s3cr3t-hdr-91"}}"#.to_owned(),
        r#"{"request":{"method":"GET","url":"https://x.test/i","headers":[{"name":"A","value":"1"},"Bearer s3cr3t-hdr-92"]}}"#.to_owned(),
        entry("GET", "https://x.test/ok", ""),
    ]));
    let c = &b.collection;
    let unwritten: Vec<&ImportNote> = c.unwritten.iter().collect();
    let want: Vec<String> = (0..7).map(|i| format!("entries[{i}]")).collect();
    assert_eq!(sources(&unwritten, NoteKind::Dropped), want);
    let details: Vec<&str> = unwritten.iter().map(|n| n.detail.as_str()).collect();
    assert_eq!(details[..3], ["entry is not an http or https request"; 3]);
    assert_eq!(
        details[3..],
        ["entry request is not a valid HAR request"; 4]
    );
    assert!(
        all_notes(c).iter().all(|n| !n.detail.contains("s3cr3t")),
        "a serde error leaked a value"
    );
    assert_eq!(request(folder(c, "x.test"), "GET /ok").seq, 1);
}

#[test]
fn a_log_without_entries_is_invalid_and_only_version_1_reads() {
    let options = CollectionOptions::default();
    for input in [
        r#"{"log":{}}"#,
        r#"{"log":{"entries":{}}}"#,
        r#"{"method":"GET","url":"https://x.test"}"#,
    ] {
        let e = parse_with(input, &options).unwrap_err();
        assert!(
            matches!(&e, FormatError::Har(DocumentError::Invalid { message }) if message == "log.entries is missing"),
            "{input}: {e:?}"
        );
    }
    let e = parse_with(r#"{"log":{"version":"2.0","entries":[]}}"#, &options).unwrap_err();
    assert!(
        matches!(&e, FormatError::Har(DocumentError::Unsupported { found }) if found == "HAR version 2.0"),
        "{e:?}"
    );
    assert_eq!(e.to_string(), "HAR: unsupported HAR version 2.0");
    for head in [r#""version":"1.1","#, r#""version":"","#, ""] {
        let b = one(&format!(r#"{{"log":{{{head}"entries":[]}}}}"#));
        assert!(b.collection.items.is_empty());
    }
}

#[test]
fn the_name_option_names_the_collection() {
    let mut options = CollectionOptions::default();
    options.name = Some("Checkout flow".into());
    let b = parse_with(&log(&[]), &options).expect("parses");
    assert_eq!(b[0].collection.name, "Checkout flow");
}

fn header(name: &str, value: &str) -> String {
    format!(r#"{{"name":"{name}","value":"{value}"}}"#)
}

fn with_headers(method: &str, url: &str, headers: &[String]) -> String {
    entry(
        method,
        url,
        &format!(r#","headers":[{}]"#, headers.join(",")),
    )
}

fn secrets(names: &[&str]) -> Vec<EnvVariable> {
    names.iter().map(|n| EnvVariable::secret(*n)).collect()
}

/// Every text a bundle reaches: its `Debug`, its notes, the write report's notes and every written file.
fn reached(b: &ImportBundle) -> Vec<String> {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let (c, report) = write_collection(
        dir.path(),
        &b.collection,
        &b.environments,
        &MemoryStore::new(),
    )
    .expect("writes");
    let mut out = vec![format!("{b:?}")];
    out.extend(all_notes(&b.collection).iter().map(|n| n.detail.clone()));
    out.extend(report.notes.iter().map(|n| n.detail.clone()));
    let mut stack = vec![c.root().to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(std::fs::read_to_string(&path).expect("utf-8"));
            }
        }
    }
    out
}

#[test]
fn basic_is_split_at_the_first_colon_into_shared_secret_references() {
    let b = one(&log(&[
        // ann:s3cr3t:pw-91, ann:other-pw, bob:s3cr3t:pw-91
        with_headers(
            "GET",
            "https://api.shop.test/a",
            &[header("Authorization", "Basic YW5uOnMzY3IzdDpwdy05MQ==")],
        ),
        with_headers(
            "GET",
            "https://api.shop.test/b",
            &[header("authorization", "basic YW5uOm90aGVyLXB3")],
        ),
        with_headers(
            "GET",
            "https://api.shop.test/c",
            &[header("Authorization", "Basic Ym9iOnMzY3IzdDpwdy05MQ==")],
        ),
    ]));
    let f = folder(&b.collection, "api.shop.test");
    let basic = |u: &str, p: &str| AuthSpec::Basic {
        username: format!("{{{{{u}}}}}"),
        password: format!("{{{{{p}}}}}"),
    };
    let a = request(f, "GET /a");
    assert_eq!(
        a.request.auth,
        basic("api_shop_test_username", "api_shop_test_password")
    );
    assert_eq!(
        request(f, "GET /b").request.auth,
        basic("api_shop_test_username", "api_shop_test_password_2")
    );
    assert_eq!(
        request(f, "GET /c").request.auth,
        basic("api_shop_test_username_2", "api_shop_test_password")
    );
    assert!(
        a.request.headers.is_empty(),
        "the header is replaced by the auth"
    );
    assert!(a.notes.iter().any(|n| n.kind == NoteKind::Converted
        && n.source == "api.shop.test / GET /a / request.headers[0]"
        && n.detail.contains("api_shop_test_password")));
    let [env] = &b.environments[..] else {
        panic!("one environment")
    };
    assert_eq!(env.name, "Capture");
    let want = [
        "api_shop_test_username",
        "api_shop_test_password",
        "api_shop_test_password_2",
        "api_shop_test_username_2",
    ];
    assert_eq!(env.variables, secrets(&want));
}

#[test]
fn bearer_shares_tokens_and_other_schemes_stay_headers() {
    let b = one(&log(&[
        with_headers(
            "GET",
            "https://api.shop.test/a",
            &[header("Authorization", "Bearer s3cr3t-tok-91")],
        ),
        with_headers(
            "GET",
            "https://api.shop.test/b",
            &[header("authorization", "bearer s3cr3t-tok-91")],
        ),
        with_headers(
            "GET",
            "https://api.shop.test/c",
            &[header("Authorization", "Bearer other-tok")],
        ),
        with_headers(
            "GET",
            "https://api.shop.test/d",
            &[header(
                "Authorization",
                "Digest username=\\\"ann\\\", response=\\\"s3cr3t-dig-91\\\"",
            )],
        ),
        with_headers(
            "GET",
            "https://api.shop.test/e",
            &[header("Authorization", "Basic !!!")],
        ),
        with_headers(
            "GET",
            "https://api.shop.test/f",
            &[header("Authorization", "Basic bm9jb2xvbg==")],
        ),
        with_headers(
            "GET",
            "http://127.0.0.1:3000/g",
            &[header("Authorization", "Bearer s3cr3t-tok-91")],
        ),
    ]));
    let c = &b.collection;
    let f = folder(c, "api.shop.test");
    let bearer = |t: &str| AuthSpec::Bearer {
        token: format!("{{{{{t}}}}}"),
    };
    assert_eq!(
        request(f, "GET /a").request.auth,
        bearer("api_shop_test_token")
    );
    assert_eq!(
        request(f, "GET /b").request.auth,
        bearer("api_shop_test_token")
    );
    assert_eq!(
        request(f, "GET /c").request.auth,
        bearer("api_shop_test_token_2")
    );
    for (name, var) in [
        ("GET /d", "authorization"),
        ("GET /e", "authorization_2"),
        ("GET /f", "authorization_3"),
    ] {
        let r = &request(f, name).request;
        assert_eq!(r.auth, AuthSpec::None, "{name}");
        assert_eq!(
            r.headers,
            [KeyValue::new(
                "Authorization",
                format!("{{{{api_shop_test_{var}}}}}")
            )],
            "{name}"
        );
    }
    assert_eq!(
        request(folder(c, "127.0.0.1:3000"), "GET /g").request.auth,
        bearer("127_0_0_1_token")
    );
    let want = [
        "api_shop_test_token",
        "api_shop_test_token_2",
        "api_shop_test_authorization",
        "api_shop_test_authorization_2",
        "api_shop_test_authorization_3",
        "127_0_0_1_token",
    ];
    assert_eq!(b.environments[0].variables, secrets(&want));
}

#[test]
fn proxy_and_cookie_values_become_header_references() {
    let cookies =
        r#","cookies":[{"name":"sid","value":"s3cr3t-sid-91"},{"name":"theme","value":"dark"}]"#;
    let b = one(&log(&[
        entry(
            "GET",
            "https://My-Shop.test/a",
            &format!(
                r#",{}{cookies}"#,
                r#""headers":[{"name":"Proxy-Authorization","value":"Basic cHJveHk6czNjcjN0LXByb3h5LTkx"},{"name":"Cookie","value":"sid=s3cr3t-sid-91; theme=dark"}]"#
            ),
        ),
        entry("GET", "https://www.shop.test/b", cookies),
    ]));
    let c = &b.collection;
    let a = &request(folder(c, "my-shop.test"), "GET /a").request;
    assert_eq!(
        a.headers,
        [
            KeyValue::new(
                "Proxy-Authorization",
                "{{my_shop_test_proxy_authorization}}"
            ),
            KeyValue::new("Cookie", "{{my_shop_test_cookie}}"),
        ]
    );
    let w = &request(folder(c, "www.shop.test"), "GET /b").request;
    assert_eq!(
        w.headers,
        [KeyValue::new("Cookie", "{{www_shop_test_cookie}}")]
    );
    let want = [
        "my_shop_test_proxy_authorization",
        "my_shop_test_cookie",
        "www_shop_test_cookie",
    ];
    assert_eq!(b.environments[0].variables, secrets(&want));
}

#[test]
fn no_credentials_mean_no_capture_environment() {
    let b = one(&log(&[with_headers(
        "GET",
        "https://x.test/a",
        &[header("Accept", "*/*")],
    )]));
    assert!(b.environments.is_empty());
    assert_eq!(only(&b).request.auth, AuthSpec::None);
}

#[test]
fn captured_values_never_reach_the_bundle_or_the_disk() {
    let b = one(&log(&[
        with_headers(
            "GET",
            "https://x.test/a",
            &[
                header("Authorization", "Basic YW5uOnMzY3IzdDpwdy05MQ=="),
                header("Cookie", "sid=s3cr3t-sid-91"),
            ],
        ),
        with_headers(
            "GET",
            "https://x.test/b",
            &[header("Authorization", "Bearer s3cr3t-tok-91")],
        ),
    ]));
    for v in &b.environments[0].variables {
        assert!(v.value == VarValue::Text(String::new()), "{}", v.name);
    }
    let texts = reached(&b);
    for secret in ["YW5uOnMzY3IzdDpwdy05MQ==", "s3cr3t"] {
        assert!(texts.iter().all(|t| !t.contains(secret)), "{secret} leaked");
    }
}

#[test]
fn userinfo_leaves_the_url_and_becomes_basic_secrets() {
    let b = one(&log(&[
        entry("GET", "https://ann:s3cr3t-pw-91@API.shop.test/a?x=1", ""),
        // ann:s3cr3t-pw-91 again, so the header shares the userinfo's variables.
        with_headers(
            "GET",
            "https://api.shop.test/b",
            &[header("Authorization", "basic YW5uOnMzY3IzdC1wdy05MQ==")],
        ),
        entry("GET", "https://bob%40corp@api.shop.test/c", ""),
        with_headers(
            "GET",
            "https://eve:s3cr3t-pw-92@api.shop.test/d",
            &[header("Authorization", "Bearer s3cr3t-tok-91")],
        ),
    ]));
    let f = folder(&b.collection, "api.shop.test");
    let basic = |u: &str, p: &str| AuthSpec::Basic {
        username: u.to_owned(),
        password: p.to_owned(),
    };
    let a = request(f, "GET /a");
    assert_eq!(a.request.url, "https://API.shop.test/a?x=1");
    assert_eq!(
        a.request.auth,
        basic("{{api_shop_test_username}}", "{{api_shop_test_password}}")
    );
    assert!(
        a.notes.iter().any(|n| n.kind == NoteKind::Converted
            && n.source == "api.shop.test / GET /a / request.url"
            && n.detail.contains("api_shop_test_password")),
        "{:?}",
        a.notes
    );
    assert_eq!(request(f, "GET /b").request.auth, a.request.auth);
    let c = &request(f, "GET /c").request;
    assert_eq!(
        (c.url.as_str(), &c.auth),
        (
            "https://api.shop.test/c",
            &basic("{{api_shop_test_username_2}}", "")
        )
    );
    let d = &request(f, "GET /d").request;
    let token = AuthSpec::Bearer {
        token: "{{api_shop_test_token}}".into(),
    };
    assert_eq!(
        (d.url.as_str(), &d.auth),
        ("https://api.shop.test/d", &token)
    );
    let want = [
        "api_shop_test_username",
        "api_shop_test_password",
        "api_shop_test_username_2",
        "api_shop_test_username_3",
        "api_shop_test_password_2",
        "api_shop_test_token",
    ];
    assert_eq!(b.environments[0].variables, secrets(&want));
}

#[test]
fn credential_like_query_values_become_secret_references() {
    let b = one(&log(&[
        entry(
            "GET",
            "https://maps.test/geo?q=oslo&API_KEY=s3cr3t-key-91&Sig=s3cr3t-sig-91&page=2&token=&keyword=k",
            "",
        ),
        entry(
            "GET",
            "https://maps.test/tiles?api_key=s3cr3t-key-91&key=s3cr3t-key-92",
            "",
        ),
    ]));
    let f = folder(&b.collection, "maps.test");
    let geo = request(f, "GET /geo");
    let url = "https://maps.test/geo?q=oslo&API_KEY={{maps_test_api_key}}&Sig={{maps_test_sig}}&page=2&token=&keyword=k";
    assert_eq!(geo.request.url, url);
    let want = [
        Param::new("q", "oslo", ParamKind::Query),
        Param::new("API_KEY", "{{maps_test_api_key}}", ParamKind::Query),
        Param::new("Sig", "{{maps_test_sig}}", ParamKind::Query),
        Param::new("page", "2", ParamKind::Query),
        Param::new("token", "", ParamKind::Query),
        Param::new("keyword", "k", ParamKind::Query),
    ];
    assert_eq!(geo.request.params, want);
    let converted = sources(&geo.notes.iter().collect::<Vec<_>>(), NoteKind::Converted);
    assert_eq!(converted, ["maps.test / GET /geo / request.url"; 2]);
    let tiles = &request(f, "GET /tiles").request;
    assert_eq!(
        tiles.url,
        "https://maps.test/tiles?api_key={{maps_test_api_key}}&key={{maps_test_key}}"
    );
    assert_eq!(
        b.environments[0].variables,
        secrets(&["maps_test_api_key", "maps_test_sig", "maps_test_key"])
    );
}

#[test]
fn url_credentials_never_reach_params_notes_debug_or_disk() {
    let b = one(&log(&[
        entry(
            "POST",
            "https://ann:s3cr3t-pw-91@x.test/a?client_secret=s3cr3t-cs-91&n=1",
            r#","queryString":[{"name":"client_secret","value":"s3cr3t-cs-91"}]"#,
        ),
        entry("GET", "http://x.test:8080/b?Access_Token=s3cr3t-at-91", ""),
    ]));
    let mut texts = reached(&b);
    for item in &b.collection.items {
        let ImportedItem::Folder(f) = item else {
            panic!("folder")
        };
        for r in &f.items {
            let ImportedItem::Request(r) = r else {
                panic!("request")
            };
            texts.push(r.request.url.clone());
            texts.extend(
                r.request
                    .params
                    .iter()
                    .map(|p| format!("{}={}", p.name, p.value)),
            );
        }
    }
    assert!(texts.iter().any(|t| t.contains("{{x_test_access_token}}")));
    for secret in ["s3cr3t", "ann:"] {
        assert!(texts.iter().all(|t| !t.contains(secret)), "{secret} leaked");
    }
}

#[test]
fn userinfo_is_cut_after_any_run_of_slashes_or_backslashes() {
    let b = one(&log(&[
        entry("GET", "https:ann:s3cr3t-pw-91@x.test/a", ""),
        entry("GET", "https:/ann:s3cr3t-pw-91@x.test/b", ""),
        entry("GET", r"https:\\\\ann:s3cr3t-pw-91@x.test\\c", ""),
    ]));
    let f = folder(&b.collection, "x.test");
    let basic = AuthSpec::Basic {
        username: "{{x_test_username}}".into(),
        password: "{{x_test_password}}".into(),
    };
    let urls = ["https:x.test/a", "https:/x.test/b", r"https:\\x.test\c"];
    for (name, url) in ["GET /a", "GET /b", "GET /c"].into_iter().zip(urls) {
        let r = &request(f, name).request;
        assert_eq!((r.url.as_str(), &r.auth), (url, &basic), "{name}");
    }
    let texts = reached(&b);
    assert!(
        texts
            .iter()
            .all(|t| !t.contains("s3cr3t") && !t.contains("ann:"))
    );
}

#[test]
fn a_url_whose_userinfo_cannot_be_cut_is_unwritten() {
    let b = one(&log(&[entry(
        "GET",
        r"https:/\t/ann:s3cr3t-pw-91@x.test/a",
        "",
    )]));
    let c = &b.collection;
    assert!(c.items.is_empty());
    let details: Vec<&str> = c.unwritten.iter().map(|n| n.detail.as_str()).collect();
    assert_eq!(details, ["entry URL credentials could not be removed"]);
    assert!(b.environments.is_empty());
    assert!(reached(&b).iter().all(|t| !t.contains("s3cr3t")));
}

#[test]
fn empty_userinfo_is_cut_and_gives_no_auth() {
    let b = one(&log(&[
        entry("GET", "https://@x.test/a", ""),
        entry("GET", "https://:@x.test/b", ""),
    ]));
    let f = folder(&b.collection, "x.test");
    for (name, url) in [
        ("GET /a", "https://x.test/a"),
        ("GET /b", "https://x.test/b"),
    ] {
        let r = &request(f, name).request;
        assert_eq!((r.url.as_str(), &r.auth), (url, &AuthSpec::None), "{name}");
    }
    assert!(b.environments.is_empty());
}

#[test]
fn transport_and_pseudo_headers_are_filtered_silently() {
    let names = [
        ":authority",
        ":method",
        ":path",
        ":scheme",
        "Host",
        "content-length",
        "CONNECTION",
        "Keep-Alive",
        "Transfer-Encoding",
        "Upgrade",
        "accept-encoding",
        "User-Agent",
        "accept",
        "X-Req",
    ];
    let headers: Vec<String> = names.iter().map(|n| header(n, "v")).collect();
    let b = one(&log(&[with_headers("GET", "https://x.test/a", &headers)]));
    let r = only(&b);
    let kept: Vec<&str> = r.request.headers.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(kept, ["User-Agent", "accept", "X-Req"]);
    assert!(r.notes.is_empty(), "{:?}", r.notes);
}

#[test]
fn an_exact_repeat_is_dropped_with_a_note() {
    let b = one(&log(&[with_headers(
        "POST",
        "https://x.test/a",
        &[
            header("Content-Type", "text/plain"),
            header("Accept", "*/*"),
            header(":path", "/a"),
            header("content-type", "text/plain"),
            header("Content-Type", "text/csv"),
        ],
    )]));
    let r = only(&b);
    assert_eq!(
        r.request.headers,
        [
            KeyValue::new("Content-Type", "text/plain"),
            KeyValue::new("Accept", "*/*"),
            KeyValue::new("Content-Type", "text/csv"),
        ]
    );
    let notes: Vec<(&str, NoteKind)> = r
        .notes
        .iter()
        .map(|n| (n.source.as_str(), n.kind))
        .collect();
    assert_eq!(
        notes,
        [("x.test / POST /a / request.headers[3]", NoteKind::Dropped)]
    );
}

#[test]
fn filtering_runs_before_credentials_so_dropped_headers_capture_nothing() {
    let b = one(&log(&[with_headers(
        "GET",
        "https://x.test/a",
        &[
            header(":authority", "x.test"),
            header("Authorization", "Bearer s3cr3t-tok-91"),
            header("authorization", "Bearer s3cr3t-tok-91"),
        ],
    )]));
    assert_eq!(
        only(&b).request.auth,
        AuthSpec::Bearer {
            token: "{{x_test_token}}".into()
        }
    );
    assert!(only(&b).request.headers.is_empty());
    assert_eq!(b.environments[0].variables, secrets(&["x_test_token"]));
}

#[test]
fn headers_are_copied_and_unknown_members_noted_but_custom_ones_ignored() {
    let input = r#"{"zzRoot":1,"log":{"version":"1.2","_exporter":"x","zzLog":1,"comment":"c","entries":[
        {"_initiator":{"type":"script"},"_priority":"High","zzEntry":1,"pageref":"p","serverIPAddress":"1.2.3.4",
         "request":{"method":"GET","url":"https://x.test/a","_private":1,"zzRequest":1,"httpVersion":"h2",
           "headersSize":-1,"bodySize":0,"queryString":[],"cookies":[],
           "headers":[{"name":"Accept","value":"*/*"},{"name":"X-Trace","value":"1","comment":"c","zzHeader":1}]}}]}}"#;
    let b = one(input);
    let r = only(&b);
    assert_eq!(
        r.request.headers,
        [
            KeyValue::new("Accept", "*/*"),
            KeyValue::new("X-Trace", "1")
        ]
    );
    let mut dropped = sources(&all_notes(&b.collection), NoteKind::Dropped);
    dropped.sort();
    assert_eq!(
        dropped,
        [
            "collection / log.zzLog",
            "collection / zzRoot",
            "x.test / GET /a / request.headers[1].zzHeader",
            "x.test / GET /a / request.zzRequest",
            "x.test / GET /a / zzEntry",
        ]
    );
}

/// The body and notes of one `POST https://x.test/b` whose `postData` is `post_data`.
fn body_of(post_data: &str) -> (BodySpec, Vec<ImportNote>) {
    let extra = if post_data.is_empty() {
        String::new()
    } else {
        format!(r#","postData":{post_data}"#)
    };
    let b = one(&log(&[entry("POST", "https://x.test/b", &extra)]));
    let r = only(&b);
    (r.request.body.clone(), r.notes.clone())
}

fn kv(pairs: &[(&str, &str)]) -> Vec<KeyValue> {
    pairs.iter().map(|(k, v)| KeyValue::new(*k, *v)).collect()
}

#[test]
fn text_bodies_follow_their_mime_type() {
    let cases = [
        (
            r#"{"mimeType":"application/json; charset=utf-8","text":"{\"a\":1}"}"#,
            BodySpec::Json("{\"a\":1}".into()),
        ),
        (
            r#"{"mimeType":"application/vnd.api+json","text":"{}"}"#,
            BodySpec::Json("{}".into()),
        ),
        (
            r#"{"mimeType":"text/xml","text":"<a/>"}"#,
            BodySpec::Xml("<a/>".into()),
        ),
        (
            r#"{"mimeType":"application/soap+xml","text":"<a/>"}"#,
            BodySpec::Xml("<a/>".into()),
        ),
        (
            r#"{"mimeType":"Text/CSV","text":"a,b"}"#,
            BodySpec::Text("a,b".into()),
        ),
        (
            r#"{"mimeType":"","text":"hello"}"#,
            BodySpec::Text("hello".into()),
        ),
        (
            r#"{"text":"hello","params":[]}"#,
            BodySpec::Text("hello".into()),
        ),
        (r#"{"mimeType":"text/plain","text":""}"#, BodySpec::None),
        ("{}", BodySpec::None),
        ("", BodySpec::None),
    ];
    for (post_data, want) in cases {
        let (got, notes) = body_of(post_data);
        assert_eq!(got, want, "{post_data}");
        assert!(notes.is_empty(), "{post_data}: {notes:?}");
    }
    let (got, notes) = body_of(r#"{"mimeType":"application/octet-stream","text":"abc"}"#);
    assert_eq!(got, BodySpec::Text("abc".into()));
    assert!(
        matches!(&notes[..], [n] if n.kind == NoteKind::Converted
        && n.source == "x.test / POST /b / request.postData"
        && n.detail.contains("application/octet-stream")),
        "{notes:?}"
    );
}

#[test]
fn urlencoded_text_is_decoded_and_wins_over_params() {
    let (got, _) = body_of(
        r#"{"mimeType":"application/x-www-form-urlencoded","text":"a=1&b=x%20y+z",
        "params":[{"name":"a","value":"1"},{"name":"b","value":"x%20y+z"}]}"#,
    );
    assert_eq!(
        got,
        BodySpec::FormUrlEncoded(kv(&[("a", "1"), ("b", "x y z")]))
    );
    let (got, _) = body_of(
        r#"{"mimeType":"application/x-www-form-urlencoded","text":"","params":[{"name":"q","value":"a%20b"}]}"#,
    );
    assert_eq!(got, BodySpec::FormUrlEncoded(kv(&[("q", "a%20b")])));
}

#[test]
fn multipart_comes_from_params_and_files_need_input() {
    let (got, notes) = body_of(
        r#"{"mimeType":"multipart/form-data; boundary=B","text":"--B\r\n(binary)\r\n--B--",
        "params":[{"name":"title","value":"Hi"},
                  {"name":"doc","fileName":"a.pdf","contentType":"application/pdf"},
                  {"name":"img","value":"(binary)"}]}"#,
    );
    let mut doc = MultipartField::files("doc", vec![String::new()]);
    doc.content_type = "application/pdf".into();
    let want = vec![
        MultipartField::text("title", "Hi"),
        doc,
        MultipartField::files("img", vec![String::new()]),
    ];
    assert_eq!(got, BodySpec::Multipart(want));
    let needs = sources(&notes.iter().collect::<Vec<_>>(), NoteKind::NeedsInput);
    assert_eq!(
        needs,
        [
            "x.test / POST /b / request.postData.params[1]",
            "x.test / POST /b / request.postData.params[2]"
        ]
    );
}

#[test]
fn a_multipart_body_without_params_is_dropped_with_a_note() {
    let (got, notes) = body_of(
        r#"{"mimeType":"multipart/form-data; boundary=B","text":"--B\r\nx\r\n--B--","params":[]}"#,
    );
    assert_eq!(got, BodySpec::None);
    let dropped = sources(&notes.iter().collect::<Vec<_>>(), NoteKind::Dropped);
    assert_eq!(dropped, ["x.test / POST /b / request.postData"]);
}

#[test]
fn unknown_post_data_and_param_members_are_noted() {
    let (_, notes) = body_of(
        r#"{"mimeType":"application/x-www-form-urlencoded","comment":"c","zzBody":1,
        "params":[{"name":"a","value":"1","comment":"c","zzParam":1}]}"#,
    );
    let dropped = sources(&notes.iter().collect::<Vec<_>>(), NoteKind::Dropped);
    assert_eq!(
        dropped,
        [
            "x.test / POST /b / request.postData.zzBody",
            "x.test / POST /b / request.postData.params[0].zzParam"
        ]
    );
}

#[test]
fn api_key_style_headers_become_secret_references() {
    let b = one(&log(&[with_headers(
        "GET",
        "https://api.shop.test/a",
        &[
            header("X-Api-Key", "s3cr3t-key-91"),
            header("PRIVATE-TOKEN", "s3cr3t-pat-91"),
            header("X-CSRF-Token", "s3cr3t-csrf-91"),
            header("x-api-key-id", "public-id"),
            header("Accept", "*/*"),
        ],
    )]));
    let r = only(&b);
    assert_eq!(
        r.request.headers,
        [
            KeyValue::new("X-Api-Key", "{{api_shop_test_x_api_key}}"),
            KeyValue::new("PRIVATE-TOKEN", "{{api_shop_test_private_token}}"),
            KeyValue::new("X-CSRF-Token", "{{api_shop_test_x_csrf_token}}"),
            KeyValue::new("x-api-key-id", "public-id"),
            KeyValue::new("Accept", "*/*"),
        ]
    );
    assert_eq!(
        b.environments[0].variables,
        secrets(&[
            "api_shop_test_x_api_key",
            "api_shop_test_private_token",
            "api_shop_test_x_csrf_token"
        ])
    );
    let converted = sources(&r.notes.iter().collect::<Vec<_>>(), NoteKind::Converted);
    assert_eq!(
        converted,
        [
            "api.shop.test / GET /a / request.headers[0]",
            "api.shop.test / GET /a / request.headers[1]",
            "api.shop.test / GET /a / request.headers[2]",
        ]
    );
    assert!(r.notes.iter().all(|n| n.detail.contains("Capture")));
    assert!(reached(&b).iter().all(|t| !t.contains("s3cr3t")));
}

#[test]
fn empty_credential_headers_stay_as_captured_and_make_no_variable() {
    let b = one(&log(&[entry(
        "GET",
        "https://x.test/a",
        r#","headers":[{"name":"Cookie","value":""},{"name":"Proxy-Authorization","value":" "},{"name":"Authorization","value":""},{"name":"X-Api-Key","value":""}],"cookies":[{"name":"sid","value":"s3cr3t-sid-91"}]"#,
    )]));
    let r = only(&b);
    assert_eq!(
        r.request.headers,
        [
            KeyValue::new("Cookie", ""),
            KeyValue::new("Proxy-Authorization", " "),
            KeyValue::new("Authorization", ""),
            KeyValue::new("X-Api-Key", ""),
        ]
    );
    assert!(b.environments.is_empty());
    assert!(r.notes.is_empty(), "{:?}", r.notes);
}

#[test]
fn more_credential_query_names_are_replaced_and_lookalikes_kept() {
    let b = one(&log(&[entry(
        "GET",
        "https://id.test/cb?refresh_token=s3cr3t-rt-91&ID_TOKEN=s3cr3t-it-91&client-secret=s3cr3t-cs-91&code=c1&state=st&client_id=app&token_type=bearer&email=a%40b.test",
        "",
    )]));
    let r = only(&b);
    assert_eq!(
        r.request.url,
        "https://id.test/cb?refresh_token={{id_test_refresh_token}}&ID_TOKEN={{id_test_id_token}}&client-secret={{id_test_client_secret}}&code=c1&state=st&client_id=app&token_type=bearer&email=a%40b.test"
    );
    assert_eq!(
        b.environments[0].variables,
        secrets(&[
            "id_test_refresh_token",
            "id_test_id_token",
            "id_test_client_secret"
        ])
    );
}

#[test]
fn an_uncut_url_leaves_no_capture_variables() {
    let b = one(&log(&[
        entry(
            "GET",
            r"https:/\t/ann:s3cr3t-pw-91@x.test/a?token=s3cr3t-tok-91",
            "",
        ),
        entry("GET", "https://y.test/b", ""),
    ]));
    let details: Vec<&str> = b
        .collection
        .unwritten
        .iter()
        .map(|n| n.detail.as_str())
        .collect();
    assert_eq!(details, ["entry URL credentials could not be removed"]);
    assert!(b.environments.is_empty(), "{:?}", b.environments);
    assert!(reached(&b).iter().all(|t| !t.contains("s3cr3t")));
}

#[test]
fn a_null_version_reads_as_missing() {
    let b = one(r#"{"log":{"version":null,"entries":[]}}"#);
    assert!(b.collection.items.is_empty());
}

#[test]
fn encoded_or_tab_split_query_names_are_still_credentials() {
    let b = one(&log(&[entry(
        "GET",
        r"https://x.test/a?%74oken=s3cr3t-tok-91&to\tken=s3cr3t-tok-92&%6Bey=s3cr3t-key-91&n=1",
        "",
    )]));
    let r = only(&b);
    assert_eq!(
        r.request.url,
        "https://x.test/a?%74oken={{x_test_token}}&to\tken={{x_test_token_2}}&%6Bey={{x_test_key}}&n=1"
    );
    let mut texts = reached(&b);
    texts.extend(
        r.request
            .params
            .iter()
            .map(|p| format!("{}={}", p.name, p.value)),
    );
    assert!(texts.iter().all(|t| !t.contains("s3cr3t")));
}

fn converted_at(notes: &[ImportNote]) -> Vec<&str> {
    notes
        .iter()
        .filter(|n| n.kind == NoteKind::Converted)
        .map(|n| n.source.as_str())
        .collect()
}

#[test]
fn credential_form_fields_become_secret_references() {
    let (got, notes) = body_of(
        r#"{"mimeType":"application/x-www-form-urlencoded","text":"username=ann&password=s3cr3t-pw-91&grant_type=password&Client_Secret=s3cr3t-cs-91&key=k1"}"#,
    );
    assert_eq!(
        got,
        BodySpec::FormUrlEncoded(kv(&[
            ("username", "ann"),
            ("password", "{{x_test_password}}"),
            ("grant_type", "password"),
            ("Client_Secret", "{{x_test_client_secret}}"),
            ("key", "k1"),
        ]))
    );
    assert_eq!(
        converted_at(&notes),
        ["x.test / POST /b / request.postData.text"; 2]
    );
    let (got, notes) = body_of(
        r#"{"mimeType":"application/x-www-form-urlencoded","text":"","params":[{"name":"q","value":"a"},{"name":"Refresh-Token","value":"s3cr3t-rt-91"},{"name":"key","value":"k"}]}"#,
    );
    assert_eq!(
        got,
        BodySpec::FormUrlEncoded(kv(&[
            ("q", "a"),
            ("Refresh-Token", "{{x_test_refresh_token}}"),
            ("key", "k"),
        ]))
    );
    assert_eq!(
        converted_at(&notes),
        ["x.test / POST /b / request.postData.params[1]"]
    );
}

#[test]
fn credential_multipart_text_fields_become_references() {
    let (got, notes) = body_of(
        r#"{"mimeType":"multipart/form-data; boundary=B","params":[{"name":"token","value":"s3cr3t-tok-91"},{"name":"note","value":"hi"},{"name":"password","fileName":"p.txt","value":"(binary)"}]}"#,
    );
    let BodySpec::Multipart(fields) = got else {
        panic!("multipart: {got:?}")
    };
    assert_eq!(fields[0], MultipartField::text("token", "{{x_test_token}}"));
    assert_eq!(fields[1], MultipartField::text("note", "hi"));
    assert_eq!(
        fields[2],
        MultipartField::files("password", vec![String::new()])
    );
    assert_eq!(
        converted_at(&notes),
        ["x.test / POST /b / request.postData.params[0]"]
    );
}

#[test]
fn credential_json_string_members_become_references_at_any_depth() {
    let text = r#"{"user":"ann","password" : "s3cr3t-pw-91","nested":{"refresh_token":"s3cr3t-rt-91","token":7,"Token":null},"note":"password","list":[{"secret":"s3cr3t-se-91"}],"esc":"a\"password\":\"x","key":"color"}"#;
    let post = serde_json::json!({"mimeType": "application/json", "text": text}).to_string();
    let (got, notes) = body_of(&post);
    let want = r#"{"user":"ann","password" : "{{x_test_password}}","nested":{"refresh_token":"{{x_test_refresh_token}}","token":{{x_test_token}},"Token":null},"note":"password","list":[{"secret":"{{x_test_secret}}"}],"esc":"a\"password\":\"x","key":"color"}"#;
    assert_eq!(got, BodySpec::Json(want.into()));
    assert_eq!(
        converted_at(&notes),
        ["x.test / POST /b / request.postData.text"; 4]
    );
    let (got, _) = body_of(r#"{"mimeType":"text/plain","text":"password=s3cr3t"}"#);
    assert_eq!(got, BodySpec::Text("password=s3cr3t".into()));
}

#[test]
fn hardened_credentials_never_reach_the_bundle_or_the_disk() {
    let json = serde_json::json!({"mimeType": "application/json", "text": r#"{"client_secret":"s3cr3t-js-91"}"#}).to_string();
    let b = one(&log(&[
        with_headers(
            "GET",
            "https://x.test/a?refresh_token=s3cr3t-rt-91",
            &[
                header("X-Api-Key", "s3cr3t-key-91"),
                header("X-Vault-Token", "s3cr3t-vt-91"),
            ],
        ),
        entry(
            "POST",
            "https://x.test/b",
            r#","postData":{"mimeType":"application/x-www-form-urlencoded","text":"password=s3cr3t-pw-91"}"#,
        ),
        entry(
            "POST",
            "https://x.test/c",
            &format!(r#","postData":{json}"#),
        ),
    ]));
    assert_eq!(
        b.environments[0].variables,
        secrets(&[
            "x_test_refresh_token",
            "x_test_x_api_key",
            "x_test_x_vault_token",
            "x_test_password",
            "x_test_client_secret"
        ])
    );
    assert!(
        reached(&b).iter().all(|t| !t.contains("s3cr3t")),
        "a captured value leaked"
    );
}

fn json_entry(path: &str, text: &str) -> String {
    let post = serde_json::json!({"mimeType": "application/json", "text": text});
    entry(
        "POST",
        &format!("https://x.test/{path}"),
        &format!(r#","postData":{post}"#),
    )
}

#[test]
fn credential_json_numbers_and_composites_become_references() {
    let text = r#"{"pin":1,"token":9191919,"secret":{"k":"s3cr3t-in-91","n":[1,2]},"password":[],"api_key":true,"refresh_token":null,"client_secret":["s3cr3t-cs-91"],"Token":{ }}"#;
    let b = one(&log(&[json_entry("b", text)]));
    let r = only(&b);
    let want = r#"{"pin":1,"token":{{x_test_token}},"secret":{{x_test_secret}},"password":[],"api_key":true,"refresh_token":null,"client_secret":{{x_test_client_secret}},"Token":{ }}"#;
    assert_eq!(r.request.body, BodySpec::Json(want.into()));
    assert_eq!(
        converted_at(&r.notes),
        ["x.test / POST /b / request.postData.text"; 3]
    );
    assert_eq!(
        b.environments[0].variables,
        secrets(&["x_test_token", "x_test_secret", "x_test_client_secret"])
    );
    for leak in ["s3cr3t", "9191919"] {
        assert!(
            reached(&b).iter().all(|t| !t.contains(leak)),
            "{leak} leaked"
        );
    }
}

#[test]
fn a_json_body_cut_off_inside_a_credential_is_dropped() {
    for text in [
        r#"{"user":"ann","password":"s3cr3t-pw-91"#,
        r#"{"user":"ann","token":{"v":"s3cr3t-tok-91""#,
        r#"{"token":"s3cr3t-tok-92","password":"#,
    ] {
        let b = one(&log(&[json_entry("b", text)]));
        let r = only(&b);
        assert_eq!(r.request.body, BodySpec::None, "{text}");
        let dropped = sources(&r.notes.iter().collect::<Vec<_>>(), NoteKind::Dropped);
        assert_eq!(
            dropped,
            ["x.test / POST /b / request.postData.text"],
            "{text}"
        );
        assert!(b.environments.is_empty(), "{text}: {:?}", b.environments);
        assert!(reached(&b).iter().all(|t| !t.contains("s3cr3t")), "{text}");
    }
    // A body cut off outside any credential value is kept, its credentials scrubbed.
    let b = one(&log(&[json_entry(
        "b",
        r#"{"token":"s3cr3t-tok-93","note":"unfini"#,
    )]));
    assert_eq!(
        only(&b).request.body,
        BodySpec::Json(r#"{"token":"{{x_test_token}}","note":"unfini"#.into())
    );
}

#[test]
fn signed_url_bracketed_and_plus_spaced_names_never_reach_the_disk() {
    let b = one(&log(&[
        entry(
            "GET",
            "https://b.s3.test/o?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIDs3cr3t91%2F20261010%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20261010T000000Z&X-Amz-Expires=900&X-Amz-Security-Token=s3cr3t-st-91&X-Amz-SignedHeaders=host&X-Amz-Signature=s3cr3t-sg-91",
            "",
        ),
        entry(
            "GET",
            "https://g.test/o?X-Goog-Credential=s3cr3t-gc-91&X-Goog-Signature=s3cr3t-gs-91&auth[token]=s3cr3t-at-91&access+token=s3cr3t-pt-91&AWSAccessKeyId=s3cr3t-ak-91",
            "",
        ),
        entry(
            "POST",
            "https://x.test/b",
            r#","postData":{"mimeType":"application/x-www-form-urlencoded","text":"user[name]=ann&user[password]=s3cr3t-pw-91&client[secret]=s3cr3t-cs-91&password[0]=s3cr3t-p0-91&password[hint]=pet"}"#,
        ),
        entry(
            "POST",
            "https://x.test/m",
            r#","postData":{"mimeType":"multipart/form-data; boundary=B","params":[{"name":"session[token]","value":"s3cr3t-mt-91"}]}"#,
        ),
    ]));
    let s3 = folder(&b.collection, "b.s3.test");
    assert_eq!(
        request(s3, "GET /o").request.url,
        "https://b.s3.test/o?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={{b_s3_test_x_amz_credential}}&X-Amz-Date=20261010T000000Z&X-Amz-Expires=900&X-Amz-Security-Token={{b_s3_test_x_amz_security_token}}&X-Amz-SignedHeaders=host&X-Amz-Signature={{b_s3_test_x_amz_signature}}"
    );
    let g = folder(&b.collection, "g.test");
    assert_eq!(
        request(g, "GET /o").request.url,
        "https://g.test/o?X-Goog-Credential={{g_test_x_goog_credential}}&X-Goog-Signature={{g_test_x_goog_signature}}&auth[token]={{g_test_token}}&access+token={{g_test_access_token}}&AWSAccessKeyId={{g_test_awsaccesskeyid}}"
    );
    let x = folder(&b.collection, "x.test");
    assert_eq!(
        request(x, "POST /b").request.body,
        BodySpec::FormUrlEncoded(kv(&[
            ("user[name]", "ann"),
            ("user[password]", "{{x_test_password}}"),
            ("client[secret]", "{{x_test_secret}}"),
            ("password[0]", "{{x_test_password_2}}"),
            ("password[hint]", "pet"),
        ]))
    );
    assert_eq!(
        b.environments[0].variables,
        secrets(&[
            "b_s3_test_x_amz_credential",
            "b_s3_test_x_amz_security_token",
            "b_s3_test_x_amz_signature",
            "g_test_x_goog_credential",
            "g_test_x_goog_signature",
            "g_test_token",
            "g_test_access_token",
            "g_test_awsaccesskeyid",
            "x_test_password",
            "x_test_secret",
            "x_test_password_2",
            "x_test_token",
        ])
    );
    assert!(reached(&b).iter().all(|t| !t.contains("s3cr3t")));
}

#[test]
fn fragment_pairs_are_scanned_like_a_query() {
    let b = one(&log(&[
        entry(
            "GET",
            "https://app.test/cb#access_token=s3cr3t-at-91&token_type=bearer&id_token=s3cr3t-it-91&state=st",
            "",
        ),
        entry(
            "GET",
            "https://app.test/cb2?x=1#a=1&refresh_token=s3cr3t-rt-91",
            "",
        ),
        entry(
            "GET",
            "https://app.test/cb3#/done?token=s3cr3t-ft-91&n=2",
            "",
        ),
    ]));
    let f = folder(&b.collection, "app.test");
    assert_eq!(
        request(f, "GET /cb").request.url,
        "https://app.test/cb#access_token={{app_test_access_token}}&token_type=bearer&id_token={{app_test_id_token}}&state=st"
    );
    let cb2 = &request(f, "GET /cb2").request;
    assert_eq!(
        cb2.url,
        "https://app.test/cb2?x=1#a=1&refresh_token={{app_test_refresh_token}}"
    );
    assert_eq!(cb2.params, [Param::new("x", "1", ParamKind::Query)]);
    assert_eq!(
        request(f, "GET /cb3").request.url,
        "https://app.test/cb3#/done?token={{app_test_token}}&n=2"
    );
    assert_eq!(
        b.environments[0].variables,
        secrets(&[
            "app_test_access_token",
            "app_test_id_token",
            "app_test_refresh_token",
            "app_test_token"
        ])
    );
    assert!(reached(&b).iter().all(|t| !t.contains("s3cr3t")));
}

#[test]
fn a_repeated_hash_in_the_fragment_hides_no_pair() {
    let b = one(&log(&[entry(
        "GET",
        "https://app.test/a8#a=1#access_token=s3cr3t-at-92",
        "",
    )]));
    let f = folder(&b.collection, "app.test");
    assert_eq!(
        request(f, "GET /a8").request.url,
        "https://app.test/a8#a=1#access_token={{app_test_access_token}}"
    );
    assert!(reached(&b).iter().all(|t| !t.contains("s3cr3t")));
}

#[test]
fn referer_urls_are_scrubbed_like_the_request_url() {
    let b = one(&log(&[
        entry(
            "GET",
            "https://app.test/cb?access_token=s3cr3t-at-91&x=1",
            "",
        ),
        with_headers(
            "GET",
            "https://api.test/me",
            &[
                header(
                    "Referer",
                    "https://app.test/cb?access_token=s3cr3t-at-91&x=1",
                ),
                header("Origin", "https://app.test"),
            ],
        ),
        with_headers(
            "GET",
            "https://api.test/you",
            &[header(
                "referer",
                "https://ann:s3cr3t-pw-91@app.test/p#id_token=s3cr3t-it-91",
            )],
        ),
        with_headers(
            "GET",
            "https://api.test/them",
            &[
                header("Referer", r"https:/\t/ann:s3cr3t-pw-92@app.test/q"),
                header("Accept", "*/*"),
            ],
        ),
        with_headers(
            "GET",
            "https://api.test/rel",
            &[header("Referer", "/cb?token=s3cr3t-rel-91")],
        ),
    ]));
    let f = folder(&b.collection, "api.test");
    let me = request(f, "GET /me");
    assert_eq!(
        me.request.headers,
        [
            KeyValue::new(
                "Referer",
                "https://app.test/cb?access_token={{app_test_access_token}}&x=1"
            ),
            KeyValue::new("Origin", "https://app.test"),
        ]
    );
    let you = request(f, "GET /you");
    assert_eq!(
        you.request.headers,
        [KeyValue::new(
            "referer",
            "https://app.test/p#id_token={{app_test_id_token}}"
        )]
    );
    let you_notes: Vec<&ImportNote> = you.notes.iter().collect();
    assert_eq!(
        sources(&you_notes, NoteKind::Dropped),
        ["api.test / GET /you / request.headers[0]"]
    );
    assert_eq!(
        sources(&you_notes, NoteKind::Converted),
        ["api.test / GET /you / request.headers[0]"]
    );
    let them = request(f, "GET /them");
    assert_eq!(them.request.headers, [KeyValue::new("Accept", "*/*")]);
    assert_eq!(
        sources(&them.notes.iter().collect::<Vec<_>>(), NoteKind::Dropped),
        ["api.test / GET /them / request.headers[0]"]
    );
    assert_eq!(
        request(f, "GET /rel").request.headers,
        [KeyValue::new("Referer", "/cb?token={{api_test_token}}")]
    );
    assert_eq!(
        b.environments[0].variables,
        secrets(&[
            "app_test_access_token",
            "app_test_id_token",
            "api_test_token"
        ])
    );
    for leak in ["s3cr3t", "ann:"] {
        assert!(
            reached(&b).iter().all(|t| !t.contains(leak)),
            "{leak} leaked"
        );
    }
}

#[test]
fn a_scheme_relative_referer_loses_its_userinfo() {
    let b = one(&log(&[
        with_headers(
            "GET",
            "https://api.test/a",
            &[header(
                "Referer",
                "//ann:s3cr3t-pw-93@app.test/p?token=s3cr3t-t-93",
            )],
        ),
        with_headers(
            "GET",
            "https://api.test/b",
            &[header("Referer", "/cb?token=s3cr3t-t-94:x@y")],
        ),
    ]));
    let f = folder(&b.collection, "api.test");
    assert_eq!(
        request(f, "GET /a").request.headers,
        [KeyValue::new(
            "Referer",
            "//app.test/p?token={{app_test_token}}"
        )]
    );
    assert_eq!(
        request(f, "GET /b").request.headers,
        [KeyValue::new("Referer", "/cb?token={{api_test_token}}")]
    );
    for leak in ["s3cr3t", "ann"] {
        assert!(
            reached(&b).iter().all(|t| !t.contains(leak)),
            "{leak} leaked"
        );
    }
}

#[test]
fn unparseable_referers_still_lose_their_userinfo() {
    let b = one(&log(&[
        with_headers(
            "GET",
            "https://api.test/a",
            &[header("Referer", "https://u:s3cr3t-pw-95@h.test:99999/")],
        ),
        with_headers(
            "GET",
            "https://api.test/b",
            &[header("Referer", "https://u:s3cr3t-pw-96@exa mple.test/")],
        ),
        with_headers(
            "GET",
            "https://api.test/c",
            &[
                header("Referer", r"https:/\t/u:s3cr3t-pw-97@h.test:99999/"),
                header("Accept", "*/*"),
            ],
        ),
        with_headers(
            "GET",
            "https://api.test/d",
            &[header("Referer", "/@alice/p")],
        ),
    ]));
    let f = folder(&b.collection, "api.test");
    assert_eq!(
        request(f, "GET /a").request.headers,
        [KeyValue::new("Referer", "https://h.test:99999/")]
    );
    assert_eq!(
        request(f, "GET /b").request.headers,
        [KeyValue::new("Referer", "https://exa mple.test/")]
    );
    let c = request(f, "GET /c");
    assert_eq!(c.request.headers, [KeyValue::new("Accept", "*/*")]);
    assert_eq!(
        sources(&c.notes.iter().collect::<Vec<_>>(), NoteKind::Dropped),
        ["api.test / GET /c / request.headers[0]"]
    );
    let d = request(f, "GET /d");
    assert_eq!(d.request.headers, [KeyValue::new("Referer", "/@alice/p")]);
    assert!(d.notes.is_empty());
    assert!(reached(&b).iter().all(|t| !t.contains("s3cr3t")));
}
