//! HAR 1.2 export of one sent request and its response.

use apinata_core::export::{HarExchange, har};
use apinata_core::import::{CollectionFormat, CollectionOptions, ImportedItem, parse_collections};
use apinata_core::{Response, derived_secrets};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime};

fn exchange() -> HarExchange {
    let mut x = HarExchange::new("POST", "https://api.test/items?page=2&q=a%20b");
    x.started = SystemTime::UNIX_EPOCH + Duration::from_secs(1_760_000_000);
    x.request_headers = vec![
        ("content-type".into(), "application/json".into()),
        ("cookie".into(), "a=1; b=2".into()),
    ];
    x.request_body = br#"{"n":1}"#.to_vec();
    x.status = 201;
    x.status_text = "Created".into();
    x.response_headers = vec![
        ("content-type".into(), "application/json".into()),
        ("set-cookie".into(), "sid=xyz; Path=/".into()),
        ("location".into(), "/items/9".into()),
    ];
    x.response_body = br#"{"id":9}"#.to_vec();
    x.time = Duration::from_millis(42);
    x
}

fn parsed(x: &HarExchange) -> Value {
    serde_json::from_str(&har(x)).expect("valid JSON")
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is a string"))
}

/// The members HAR 1.2 requires, checked by name and JSON type.
fn assert_har_12(doc: &Value) {
    let log = &doc["log"];
    assert_eq!(log["version"], "1.2");
    assert!(log["creator"]["name"].is_string() && log["creator"]["version"].is_string());
    let entries = log["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert!(
        e["startedDateTime"]
            .as_str()
            .is_some_and(|s| s.contains('T'))
    );
    assert!(e["time"].is_number() && e["cache"].is_object());
    for key in ["method", "url", "httpVersion"] {
        assert!(e["request"][key].is_string(), "request.{key}");
    }
    for key in ["cookies", "headers", "queryString"] {
        assert!(e["request"][key].is_array(), "request.{key}");
    }
    assert!(e["request"]["headersSize"].is_number() && e["request"]["bodySize"].is_number());
    assert!(e["response"]["status"].is_number());
    for key in ["statusText", "httpVersion", "redirectURL"] {
        assert!(e["response"][key].is_string(), "response.{key}");
    }
    for key in ["cookies", "headers"] {
        assert!(e["response"][key].is_array(), "response.{key}");
    }
    assert!(e["response"]["content"]["size"].is_number());
    assert!(e["response"]["content"]["mimeType"].is_string());
    for key in ["send", "wait", "receive"] {
        assert!(e["timings"][key].is_number(), "timings.{key}");
    }
}

#[test]
fn a_json_exchange_has_the_har_12_shape() {
    let doc = parsed(&exchange());
    assert_har_12(&doc);
    let e = &doc["log"]["entries"][0];
    assert_eq!(e["request"]["method"], "POST");
    assert_eq!(
        e["request"]["queryString"],
        json!([{"name":"page","value":"2"},{"name":"q","value":"a b"}])
    );
    assert_eq!(
        e["request"]["cookies"],
        json!([{"name":"a","value":"1"},{"name":"b","value":"2"}])
    );
    assert_eq!(e["request"]["postData"]["mimeType"], "application/json");
    assert_eq!(e["request"]["postData"]["text"], r#"{"n":1}"#);
    assert_eq!(e["response"]["status"], 201);
    assert_eq!(e["response"]["statusText"], "Created");
    assert_eq!(e["response"]["redirectURL"], "/items/9");
    assert_eq!(e["response"]["content"]["text"], r#"{"id":9}"#);
    assert_eq!(e["response"]["content"]["size"], 8);
    assert_eq!(e["response"]["content"]["mimeType"], "application/json");
    assert!(e["response"]["content"].get("encoding").is_none());
    assert_eq!(
        e["response"]["cookies"],
        json!([{"name":"sid","value":"xyz"}])
    );
    assert_eq!(e["startedDateTime"], "2025-10-09T08:53:20.000Z");
}

#[test]
fn only_the_measured_total_is_a_timing() {
    let e = &parsed(&exchange())["log"]["entries"][0];
    assert_eq!(e["time"], 42.0);
    for phase in ["blocked", "dns", "connect", "ssl"] {
        assert_eq!(e["timings"][phase], -1, "{phase} is unknown");
    }
    // HAR requires these three; the measured total is all wait.
    assert_eq!(e["timings"]["send"], 0);
    assert_eq!(e["timings"]["wait"], 42.0);
    assert_eq!(e["timings"]["receive"], 0);
}

#[test]
fn a_binary_body_is_base64() {
    let mut x = exchange();
    x.response_headers = vec![("content-type".into(), "image/png".into())];
    x.response_body = vec![0x89, b'P', b'N', b'G', 0, 0xff, 0xfe];
    let doc = parsed(&x);
    assert_har_12(&doc);
    let content = &doc["log"]["entries"][0]["response"]["content"];
    assert_eq!(content["encoding"], "base64");
    assert_eq!(content["text"], "iVBORwD//g==");
    assert_eq!(content["size"], 7);
}

#[test]
fn a_truncated_body_says_so_in_the_comment() {
    let mut x = exchange();
    x.response_truncated = true;
    let e = &parsed(&x)["log"]["entries"][0];
    let comment = str_of(e, "comment");
    assert!(comment.contains("truncated"), "{comment}");
    assert_eq!(e["response"]["content"]["text"], r#"{"id":9}"#);
    assert!(
        parsed(&exchange())["log"]["entries"][0]
            .get("comment")
            .is_none()
    );
}

#[test]
fn secrets_are_replaced_in_every_field() {
    let mut x = exchange();
    x.url = "https://api.test/items?key=hunter2hunter2".into();
    x.request_headers
        .push(("authorization".into(), "Bearer hunter2hunter2".into()));
    x.request_body = br#"{"pw":"hunter2hunter2"}"#.to_vec();
    x.response_headers
        .push(("x-echo".into(), "hunter2hunter2".into()));
    x.response_body = br#"{"token":"hunter2hunter2"}"#.to_vec();
    let secrets = derived_secrets(&["hunter2hunter2".to_owned()]);
    let text = har(&x.redacted(&secrets));
    assert!(!text.contains("hunter2hunter2"), "{text}");
    assert!(text.contains("api.test/items"));
}

#[test]
fn server_issued_cookies_are_masked_even_when_no_secret_is_known() {
    let mut x = exchange();
    x.request_headers = vec![("cookie".into(), "sid=abc123".into())];
    x.response_headers = vec![("set-cookie".into(), "sid=abc123; Path=/".into())];
    let text = har(&x.redacted(&[]));
    assert!(!text.contains("abc123"), "{text}");
    let doc: Value = serde_json::from_str(&text).unwrap();
    let response = &doc["log"]["entries"][0]["response"];
    assert_eq!(response["headers"][0]["value"], "sid=[redacted]");
    assert_eq!(response["cookies"][0]["name"], "sid");
    assert_eq!(response["cookies"][0]["value"], "[redacted]");
    // The whole Cookie header is hidden, so the request keeps no cookie entries.
    let request = &doc["log"]["entries"][0]["request"];
    assert_eq!(request["headers"][0]["value"], "[redacted]");
    assert_eq!(request["cookies"].as_array().unwrap().len(), 0, "{text}");
}

#[test]
fn the_file_round_trips_through_the_importer() {
    let text = har(&exchange());
    let bundles = parse_collections(
        Some(CollectionFormat::Har),
        text.as_bytes(),
        &CollectionOptions::default(),
    )
    .expect("our own importer reads it");
    let [ImportedItem::Folder(folder)] = &bundles[0].collection.items[..] else {
        panic!("one folder per host");
    };
    let [ImportedItem::Request(item)] = &folder.items[..] else {
        panic!("one request");
    };
    assert_eq!(item.request.method, "POST");
    assert!(item.request.url.starts_with("https://api.test/items"));
}

#[tokio::test]
async fn a_sent_response_becomes_an_exchange() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;
    let res: Response = {
        let url = format!("{}/ping", server.uri());
        crate::common::executor()
            .send(&crate::common::resolved_one("POST", &url, "", ""))
            .await
            .expect("sent")
    };
    let doc = parsed(&HarExchange::from_response(&res, SystemTime::now()));
    assert_har_12(&doc);
    let e = &doc["log"]["entries"][0];
    assert_eq!(e["request"]["method"], "POST");
    assert!(str_of(&e["request"], "url").ends_with("/ping"));
    assert_eq!(e["response"]["content"]["text"], "pong");
}

#[test]
fn secrets_are_replaced_in_cookies_query_redirects_and_derived_shapes() {
    let digest = r#"Digest username="bob", response="9f3a77c1d2e44b60""#;
    let basic = "Basic dXNlcjpwYXNzd29yZDEyMw==";
    let bearer = "Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig-4242";
    let mut x = exchange();
    x.url = "https://api.test/x?sig=cookie-Secret-11".into();
    x.request_headers = vec![
        ("cookie".into(), "sid=cookie-Secret-11".into()),
        ("authorization".into(), basic.into()),
        ("proxy-authorization".into(), bearer.into()),
        ("x-digest".into(), digest.into()),
    ];
    x.response_headers = vec![
        ("set-cookie".into(), "sid=cookie-Secret-11; Path=/".into()),
        ("location".into(), "/next?t=cookie-Secret-11".into()),
    ];
    let secrets = derived_secrets(&[
        "cookie-Secret-11".into(),
        basic.into(),
        bearer.into(),
        digest.into(),
    ]);
    let text = har(&x.redacted(&secrets));
    for leak in [
        "cookie-Secret-11",
        "dXNlcjpwYXNzd29yZDEyMw",
        "eyJhbGci",
        "9f3a77c1d2e44b60",
    ] {
        assert!(!text.contains(leak), "{leak} leaked: {text}");
    }
}

#[test]
fn a_non_utf8_request_body_is_redacted_through_a_lossy_copy() {
    let mut x = exchange();
    x.request_body = [b"pw=hunter2hunter2&raw=".as_slice(), &[0xff, 0xfe]].concat();
    let secrets = derived_secrets(&["hunter2hunter2".to_owned()]);
    assert!(!har(&x.redacted(&secrets)).contains("hunter2hunter2"));
}

#[test]
fn a_binary_response_body_holding_a_secret_is_reported() {
    let mut x = exchange();
    x.response_body = [b"token=hunter2hunter2".as_slice(), &[0xff, 0x00]].concat();
    let secrets = derived_secrets(&["hunter2hunter2".to_owned()]);
    assert!(x.binary_body_holds_secret(&secrets));
    x.response_body = vec![0xff, 0x00, 1, 2];
    assert!(!x.binary_body_holds_secret(&secrets));
    assert!(
        !exchange().binary_body_holds_secret(&secrets),
        "text is redacted instead"
    );
}

#[test]
fn the_response_body_size_is_the_kept_length() {
    let e = &parsed(&exchange())["log"]["entries"][0];
    assert_eq!(e["response"]["bodySize"], 8);
}

#[test]
fn bytes_hold_secret_reads_raw_bytes_through_a_lossy_copy() {
    let secrets = derived_secrets(&["hunter2hunter2".to_owned()]);
    let holding = [b"\x00\xfftoken=hunter2hunter2\x01".as_slice()].concat();
    assert!(apinata_core::bytes_hold_secret(&holding, &secrets));
    assert!(apinata_core::bytes_hold_secret(b"hunter2hunter2", &secrets));
    assert!(!apinata_core::bytes_hold_secret(
        &[0xff, 0x00, 1, 2],
        &secrets
    ));
    assert!(!apinata_core::bytes_hold_secret(b"", &secrets));
}

fn utf16(text: &str, little: bool) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(|u| {
            if little {
                u.to_le_bytes()
            } else {
                u.to_be_bytes()
            }
        })
        .collect()
}

#[test]
fn bytes_hold_secret_finds_a_secret_stored_as_utf16_either_way_round() {
    let secrets = derived_secrets(&["hunter2hunter2".to_owned()]);
    for little in [true, false] {
        let body = [
            b"\xff\xfe".as_slice(),
            &utf16("hunter2hunter2", little),
            b"\x00\x01",
        ]
        .concat();
        assert!(
            apinata_core::bytes_hold_secret(&body, &secrets),
            "little {little}"
        );
        // Odd alignment: one stray byte before the encoded secret.
        let odd = [b"\x07".as_slice(), &utf16("hunter2hunter2", little)].concat();
        assert!(
            apinata_core::bytes_hold_secret(&odd, &secrets),
            "odd, little {little}"
        );
        // A non-ASCII secret is encoded as UTF-16, not as its UTF-8 bytes widened.
        let accented = derived_secrets(&["pässwörd-9".to_owned()]);
        assert!(apinata_core::bytes_hold_secret(
            &utf16("pässwörd-9", little),
            &accented
        ));
    }
}

#[test]
fn bytes_hold_secret_finds_a_secret_outside_the_basic_plane_in_utf16() {
    let value = "key-\u{1F511}-42";
    let secrets = derived_secrets(&[value.to_owned()]);
    for little in [true, false] {
        let encoded = utf16(value, little);
        // The emoji takes a surrogate pair: two code units, four bytes.
        assert_eq!(encoded.len(), (value.chars().count() + 1) * 2);
        let body = [b"\x00\xff".as_slice(), &encoded, b"\x01"].concat();
        assert!(
            apinata_core::bytes_hold_secret(&body, &secrets),
            "little {little}"
        );
    }
}

#[test]
fn bytes_hold_secret_ignores_an_ascii_near_miss_in_utf16() {
    let secrets = derived_secrets(&["hunter2hunter2".to_owned()]);
    for little in [true, false] {
        let near = utf16("hunter2hunter3", little);
        assert!(
            !apinata_core::bytes_hold_secret(&near, &secrets),
            "little {little}"
        );
        // The secret's bytes spread with a non-zero filler are not UTF-16 of it.
        let spread: Vec<u8> = b"hunter2hunter2".iter().flat_map(|b| [*b, b'x']).collect();
        assert!(!apinata_core::bytes_hold_secret(&spread, &secrets));
    }
}

#[test]
fn an_unknown_server_address_leaves_serveripaddress_out() {
    let doc = parsed(&exchange());
    assert!(doc["log"]["entries"][0].get("serverIPAddress").is_none());
}

#[test]
fn a_server_address_is_written_as_the_ip_alone() {
    let mut x = exchange();
    x.server_ip = Some("::1".into());
    let doc = parsed(&x);
    assert_eq!(doc["log"]["entries"][0]["serverIPAddress"], "::1");
}

#[tokio::test]
async fn a_direct_send_exports_the_servers_ip_without_its_port() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let url = format!("{}/ping", server.uri());
    let res = crate::common::executor()
        .send(&crate::common::resolved_one("GET", &url, "", ""))
        .await
        .expect("sent");
    let doc = parsed(&HarExchange::from_response(&res, SystemTime::now()));
    assert_eq!(doc["log"]["entries"][0]["serverIPAddress"], "127.0.0.1");
}

// Through a proxy the peer is the proxy, not the server HAR asks for, so the member is omitted.
#[tokio::test]
async fn a_send_through_a_proxy_exports_no_server_address() {
    use crate::network_support::{custom_proxy, http_proxy, resolve_in, root_with};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&origin)
        .await;
    let proxy = http_proxy(None).await;
    let r = resolve_in(
        &root_with(&custom_proxy(
            "http",
            "127.0.0.1",
            &proxy.port.to_string(),
            None,
            "",
        )),
        &format!("{}/ping", origin.uri()),
        apinata_core::ContextOptions::new(),
    );
    let res = crate::common::executor()
        .send(r.request())
        .await
        .expect("sent");
    assert!(res.via_proxy());
    let doc = parsed(&HarExchange::from_response(&res, SystemTime::now()));
    assert!(doc["log"]["entries"][0].get("serverIPAddress").is_none());
}

#[tokio::test]
async fn a_send_through_a_host_override_exports_the_mapped_address_and_names_it() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let root = "opencollection: \"1.0.0\"\ninfo:\n  name: t\nextensions:\n  apinata:\n    hostOverrides:\n      - host: fake.test\n        address: 127.0.0.1\n";
    let url = format!("http://fake.test:{}/ping", server.address().port());
    let req = crate::network_support::resolve_in(root, &url, apinata_core::ContextOptions::new());
    let res = crate::common::executor()
        .send(req.request())
        .await
        .expect("sent");
    let doc = parsed(&HarExchange::from_response(&res, SystemTime::now()));
    let entry = &doc["log"]["entries"][0];
    assert_eq!(entry["serverIPAddress"], "127.0.0.1");
    assert_eq!(entry["_hostOverride"], "fake.test -> 127.0.0.1");
    // A send DNS answered carries no such member.
    assert!(
        parsed(&exchange())["log"]["entries"][0]
            .get("_hostOverride")
            .is_none()
    );
}

#[test]
fn a_retried_send_names_its_attempt_and_the_most_there_could_be() {
    let mut x = exchange();
    x.attempt = Some((2, 3));
    let entry = &parsed(&x)["log"]["entries"][0];
    assert_eq!(entry["_attempt"], 2, "{entry}");
    assert_eq!(entry["_attempts"], 3, "{entry}");
}

#[test]
fn a_send_that_was_not_retrying_writes_no_attempt_members() {
    let doc = parsed(&exchange());
    let entry = &doc["log"]["entries"][0];
    assert!(entry.get("_attempt").is_none() && entry.get("_attempts").is_none());
}

#[test]
fn an_attempt_member_does_not_stop_the_importer() {
    let mut x = exchange();
    x.attempt = Some((1, 2));
    let bundles = parse_collections(
        Some(CollectionFormat::Har),
        har(&x).as_bytes(),
        &CollectionOptions::default(),
    )
    .expect("the importer ignores custom members");
    assert_eq!(bundles.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logged_retries_export_their_attempt_numbers() {
    use crate::script_support::{ROOT, on, write_tree};
    use crate::session_log_support::entries;
    use apinata_core::{
        CancelHandle, Collection, ContextOptions, ExecOptions, Executor, SessionLog, VarContext,
        run,
    };
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&s)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let req = format!(
        "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{}\"\n",
        s.uri()
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &req)]);
    let log = SessionLog::new();
    let c = Collection::open(dir.path()).expect("opens");
    let plan = c.plan(dir.path()).expect("plan");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let exec = std::sync::Arc::new(
        Executor::new(ExecOptions::new().session_log(log.clone())).expect("client"),
    );
    run(
        plan,
        ctx,
        exec,
        on().retries(1).retry_delay(Duration::from_millis(5)),
        CancelHandle::new(),
        Box::new(|_| {}),
    )
    .await;
    let docs: Vec<Value> = entries(&log)
        .iter()
        .map(|e| parsed(&HarExchange::from_network_entry(e).expect("a hop")))
        .collect();
    let attempts: Vec<(Value, Value)> = docs
        .iter()
        .map(|d| {
            let e = &d["log"]["entries"][0];
            (e["_attempt"].clone(), e["_attempts"].clone())
        })
        .collect();
    assert_eq!(attempts, [(json!(1), json!(2)), (json!(2), json!(2))]);
}

#[tokio::test]
async fn a_logged_binary_body_exports_as_its_exact_bytes() {
    use crate::session_log_support::{logged, only};
    use apinata_core::ExecOptions;
    use base64::Engine as _;
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let body = vec![0x89, b'P', b'N', b'G', 0x00, 0xff, 0xfe, 0x10, 0x00];
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(body.clone()),
        )
        .mount(&server)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    let url = format!("{}/img", server.uri());
    exec.send(&crate::common::resolved_one("GET", &url, "", ""))
        .await
        .expect("sent");
    let doc = parsed(&HarExchange::from_network_entry(&only(&log)).expect("answered"));
    let response = &doc["log"]["entries"][0]["response"];
    assert_eq!(response["statusText"], "OK");
    assert_eq!(response["content"]["encoding"], "base64");
    assert_eq!(response["content"]["size"], body.len());
    let text = str_of(&response["content"], "text");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap();
    assert_eq!(decoded, body);
}

#[test]
fn a_url_userinfo_password_is_dropped_even_when_no_secret_is_known() {
    let x = HarExchange::new("GET", "http://user:pa55word-xyz@api.test/items");
    let text = har(&x.redacted(&[]));
    assert!(!text.contains("pa55word-xyz"), "{text}");
    assert!(text.contains("api.test/items"), "{text}");
}

#[test]
fn a_set_cookie_without_an_equals_sign_is_hidden_whole() {
    let mut x = exchange();
    x.response_headers = vec![("set-cookie".into(), "sekrit-token; Path=/".into())];
    let text = har(&x.redacted(&[]));
    assert!(!text.contains("sekrit"), "{text}");
}

#[test]
fn a_redirect_location_with_userinfo_is_dropped_from_redirecturl_and_the_headers() {
    let mut x = exchange();
    x.status = 302;
    x.response_headers = vec![(
        "location".into(),
        "http://joe:hop-pa55-xyz@127.0.0.1/next".into(),
    )];
    let text = har(&x.redacted(&[]));
    assert!(!text.contains("hop-pa55-xyz"), "{text}");
    let doc: Value = serde_json::from_str(&text).unwrap();
    let response = &doc["log"]["entries"][0]["response"];
    assert_eq!(response["redirectURL"], "http://127.0.0.1/next", "{text}");
    assert_eq!(
        response["headers"][0]["value"], "http://127.0.0.1/next",
        "{text}"
    );
}

#[test]
fn a_url_at_sign_in_the_path_is_not_userinfo() {
    for url in [
        "http://127.0.0.1:8080/r/joe:pw@127.0.0.1:8080/next",
        "http://127.0.0.1:8080/a//b@c",
    ] {
        let doc = parsed(&HarExchange::new("GET", url).redacted(&[]));
        assert_eq!(doc["log"]["entries"][0]["request"]["url"], url);
    }
}

#[test]
fn a_response_without_its_request_has_no_sent_exchange() {
    use apinata_core::export::{HarRefusal, sent_exchange};
    let res = Response::new(200).with_text_body("kept");
    let refusal = sent_exchange(&res, SystemTime::now(), &[]).unwrap_err();
    assert_eq!(refusal, HarRefusal::NoRequest);
}

#[tokio::test]
async fn a_sent_exchange_starts_its_response_time_before_the_end_and_refuses_a_binary_secret() {
    use apinata_core::export::{HarRefusal, sent_exchange};
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let body = [b"t=hunter2hunter2".as_slice(), &[0xff, 0x00]].concat();
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    let url = format!("{}/bin", server.uri());
    let res = crate::common::executor()
        .send(&crate::common::resolved_one("GET", &url, "", ""))
        .await
        .expect("sent");
    let ended = SystemTime::UNIX_EPOCH + Duration::from_secs(1_760_000_000);
    let x = sent_exchange(&res, ended, &[]).expect("no secret known");
    assert_eq!(x.started + res.response_time(), ended);
    let secrets = derived_secrets(&["hunter2hunter2".to_owned()]);
    let refusal = sent_exchange(&res, ended, &secrets).unwrap_err();
    assert_eq!(refusal, HarRefusal::BinarySecret);
}

#[test]
fn several_exchanges_are_one_log_with_an_entry_each() {
    use apinata_core::export::har_log;
    let mut second = exchange();
    second.url = "https://api.test/second".into();
    let doc: Value = serde_json::from_str(&har_log(&[exchange(), second])).unwrap();
    let entries = doc["log"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1]["request"]["url"], "https://api.test/second");
    assert_eq!(har_log(&[exchange()]), har(&exchange()));
}

fn wide(value: &str, big: bool) -> Vec<u8> {
    value
        .encode_utf16()
        .flat_map(|u| {
            if big {
                u.to_be_bytes()
            } else {
                u.to_le_bytes()
            }
        })
        .collect()
}

fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The response body a HAR document holds, decoded from base64 when it is.
fn content_bytes(doc: &Value) -> Vec<u8> {
    use base64::Engine as _;
    let content = &doc["log"]["entries"][0]["response"]["content"];
    let text = content["text"].as_str().unwrap_or_default();
    if content["encoding"] == "base64" {
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .unwrap()
    } else {
        text.as_bytes().to_vec()
    }
}

#[test]
fn a_utf16_secret_in_either_body_is_redacted_in_place() {
    let secret = "hunter2hunter2";
    let both = [wide(secret, false), wide(secret, true)].concat();
    let mut x = exchange();
    x.request_body = [b"pw=".as_slice(), &both].concat();
    x.response_body = [b"echo:".as_slice(), &both].concat();
    assert!(
        std::str::from_utf8(&x.response_body).is_ok(),
        "ASCII as UTF-16 is valid UTF-8"
    );
    let secrets = derived_secrets(&[secret.to_owned()]);
    assert!(!x.binary_body_holds_secret(&secrets), "redactable in place");
    let doc = parsed(&x.redacted(&secrets));
    let body = content_bytes(&doc);
    let posted = str_of(&doc["log"]["entries"][0]["request"]["postData"], "text").as_bytes();
    for big in [false, true] {
        let leak = wide(secret, big);
        assert!(!holds(&body, &leak), "response body, big endian {big}");
        assert!(!holds(posted, &leak), "request body, big endian {big}");
        let marker = wide("[redacted]", big);
        assert!(holds(&body, &marker), "response body, big endian {big}");
    }
}

#[test]
fn bodies_past_the_budget_are_cut_with_a_comment() {
    let mut x = exchange();
    x.response_body = "abcdef".repeat(10).into_bytes();
    x.request_body = "0123456789".repeat(10).into_bytes();
    let doc = parsed(&x.limited(16, false));
    let e = &doc["log"]["entries"][0];
    assert_eq!(e["response"]["content"]["text"], "abcdefabcdefabcd");
    assert_eq!(e["response"]["content"]["size"], 60);
    assert_eq!(e["request"]["postData"]["text"], "0123456789012345");
    assert_eq!(e["request"]["bodySize"], 100);
    let comment = str_of(e, "comment");
    assert!(comment.contains("16 bytes"), "{comment}");
}

#[test]
fn a_binary_body_gives_its_type_and_size_unless_the_budget_is_raised() {
    let mut x = exchange();
    x.response_headers = vec![("content-type".into(), "image/png".into())];
    x.response_body = vec![0x89, b'P', b'N', b'G', 0xff, 0x00];
    let doc = parsed(&x.clone().limited(1024, false));
    let e = &doc["log"]["entries"][0];
    assert!(e["response"]["content"].get("text").is_none(), "{e}");
    assert_eq!(e["response"]["content"]["size"], 6);
    assert_eq!(e["response"]["content"]["mimeType"], "image/png");
    assert!(str_of(e, "comment").contains("binary"), "{e}");
    let doc = parsed(&x.limited(4, true));
    assert_eq!(content_bytes(&doc), [0x89, b'P', b'N', b'G']);
}

#[test]
fn the_required_timings_are_not_negative_and_sum_to_the_time() {
    let doc = parsed(&exchange());
    let e = &doc["log"]["entries"][0];
    for phase in ["send", "wait", "receive"] {
        assert!(e["timings"][phase].as_f64().unwrap() >= 0.0, "{phase}");
    }
    let sum: f64 = ["blocked", "dns", "connect", "send", "wait", "receive"]
        .iter()
        .map(|p| e["timings"][*p].as_f64().unwrap())
        .filter(|v| *v >= 0.0)
        .sum();
    assert_eq!(sum, e["time"].as_f64().unwrap());
}
