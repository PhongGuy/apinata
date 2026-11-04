use crate::support;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_app::app::AppState;
use apinata_app::last_response::ResponseStore;
use apinata_app::send_model::{Restored, SendState};
use apinata_app::settings::RunScripts;
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::TestAppContext;
use support::*;
use wiremock::ResponseTemplate;

fn kept_result() -> RequestResult {
    RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(203).with_text_body("old")),
        Vec::new(),
    )
}

#[gpui_kit::test]
fn a_send_keeps_its_response(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("hello"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    cx.update(|cx| apinata_app::last_response::install(store.clone(), cx));
    let model = sender(cx, &ws);
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    wait_real(cx, |_| store.load(&root, &ip("a.yml")).is_some());
    let got = store.load(&root, &ip("a.yml")).unwrap();
    assert_eq!(got.result.response.as_ref().unwrap().text(), "hello");
    assert!(got.saved_at <= SystemTime::now());
    assert!(model.read_with(cx, |m, _| m.restored().is_none()));
}

#[gpui_kit::test]
fn an_opened_request_shows_its_kept_response_until_the_next_send(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("new"));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    let saved_at = UNIX_EPOCH + Duration::from_secs(1_760_000_000);
    store
        .save(&root, &ip("a.yml"), &kept_result(), saved_at)
        .unwrap();
    cx.update(|cx| apinata_app::last_response::install(store.clone(), cx));
    let (_win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.restored().is_some()));
    assert_eq!(
        model.read_with(cx, |m, _| m.restored()),
        Some(Restored {
            saved_at,
            body_dropped: false,
            body_masked: false,
            masked_headers: Vec::new(),
            header_digests: None,
        })
    );
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => assert_eq!(r.response.as_ref().unwrap().status(), 203),
        _ => panic!("not shown"),
    });
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    assert!(model.read_with(cx, |m, _| m.restored().is_none()));
    finished(cx, &model);
    wait_real(cx, |_| {
        store
            .load(&root, &ip("a.yml"))
            .is_some_and(|s| s.saved_at > saved_at)
    });
    assert!(model.read_with(cx, |m, _| m.restored().is_none()));
}

#[gpui_kit::test]
fn restore_is_ignored_once_a_send_started(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("live"));
    let col = collection_for(&server.uri(), "");
    let (_state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    let restored = Restored {
        saved_at: UNIX_EPOCH,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    model.update(cx, |m, cx| m.restore(Arc::new(kept_result()), restored, cx));
    assert!(model.read_with(cx, |m, _| m.is_sending() && m.restored().is_none()));
    let result = finished(cx, &model);
    assert_eq!(result.response.as_ref().unwrap().text(), "live");
}

#[gpui_kit::test]
fn with_remembering_off_a_send_writes_nothing(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200).set_body_string("x"));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |v| v.remember_responses = false));
    });
    let store = cx.update(|cx| cx.global::<ResponseStore>().clone());
    assert_eq!(store.dir(), s.files.path().join("responses"));
    let model = sender(cx, &s.ws);
    let r = s.root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, &model);
    cx.run_until_parked();
    assert!(store.load(&s.root, &ip("a.yml")).is_none());
    assert!(!store.dir().exists());
}

#[gpui_kit::test]
fn a_kept_script_test_never_quotes_a_secret_the_script_read(cx: &mut TestAppContext) {
    const SECRET: &str = "S3cret-tok-77";
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let code = "const t = bru.getProcessEnv('TOK'); test('named ' + t, function() { expect(t).to.equal('nope'); });";
    let a = format!(
        "{}runtime:\n  scripts:\n    - type: tests\n      code: \"{code}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let env = format!("TOK={SECRET}\n");
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a), (".env", &env)]);
    let root = col.path().to_path_buf();
    let s = started_with(cx, &col, |p| allow_scripts(p, &root));
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    // The live result quotes the secret, so only the kept copy is scrubbed.
    assert!(result.tests.iter().any(|t| t.name.contains(SECRET)));
    let store = ResponseStore::new(s.files.path().join("responses"));
    wait_real(cx, |_| store.load(&s.root, &ip("a.yml")).is_some());
    let kept = store.load(&s.root, &ip("a.yml")).unwrap();
    assert_eq!(kept.result.tests.len(), 1);
    let dir = s.files.path().join("responses");
    for f in std::fs::read_dir(&dir).unwrap() {
        let text = std::fs::read_to_string(f.unwrap().path()).unwrap_or_default();
        assert!(!text.contains(SECRET), "{text}");
    }
}

// Each file of a store directory with its contents, sorted by name.
fn store_files(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let text = std::fs::read_to_string(e.path()).unwrap();
            (e.file_name().to_string_lossy().into_owned(), text)
        })
        .collect();
    files.sort();
    files
}

// Remembering off still shows what was kept, but must not touch the store: opening a
// request over a version 1 store reads it as it is instead of migrating it.
#[gpui_kit::test]
fn with_remembering_off_opening_a_request_writes_nothing(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:9", "");
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |v| v.remember_responses = false));
    });
    let dir = cx.update(|cx| cx.global::<ResponseStore>().dir().to_path_buf());
    std::fs::create_dir_all(&dir).unwrap();
    let index = serde_json::json!({
        "version": 1,
        "entries": [{
            "root": s.root.to_string_lossy(),
            "item": "a.yml",
            "id": "00000000000000aa"
        }]
    });
    let entry = serde_json::json!({
        "version": 1, "saved_at_ms": 5, "name": "a", "outcome": "passed",
        "status": 203, "status_text": "OK", "url": "http://x/ok", "headers": [],
        "time_ms": 42.0, "size": 3, "truncated": false, "body_dropped": false,
        "text": "old", "has_body_file": false, "assertions": []
    });
    std::fs::write(dir.join("index.json"), index.to_string()).unwrap();
    std::fs::write(dir.join("00000000000000aa.json"), entry.to_string()).unwrap();
    let before = store_files(&dir);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let editor = s.shell.read_with(cx, |sh, cx| {
        sh.tab_area()
            .read(cx)
            .editor(tab)
            .expect("an editor")
            .clone()
    });
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.restored().is_some()));
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => assert_eq!(r.response.as_ref().unwrap().status(), 203),
        _ => panic!("the kept response is not shown"),
    });
    assert_eq!(
        store_files(&dir),
        before,
        "opening with remembering off rewrote the store"
    );
}

// A version 1 store of one send of `a.yml` under `root`, as plan 5r wrote it.
fn seed_v1(dir: &std::path::Path, root: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let index = serde_json::json!({
        "version": 1,
        "entries": [{ "root": root.to_string_lossy(), "item": "a.yml", "id": "00000000000000aa" }]
    });
    let entry = serde_json::json!({
        "version": 1, "saved_at_ms": 5, "name": "a", "outcome": "passed",
        "status": 203, "status_text": "OK", "url": "http://x/ok", "headers": [],
        "time_ms": 42.0, "size": 3, "truncated": false, "body_dropped": false,
        "text": "old", "has_body_file": false, "assertions": []
    });
    std::fs::write(dir.join("index.json"), index.to_string()).unwrap();
    std::fs::write(dir.join("00000000000000aa.json"), entry.to_string()).unwrap();
}

// Picking a kept send from History with remembering off shows it but leaves a version 1
// store exactly as it was.
#[gpui_kit::test]
fn with_remembering_off_picking_a_kept_send_writes_nothing(cx: &mut TestAppContext) {
    let col = collection_for("http://127.0.0.1:9", "");
    let s = started(cx, &col);
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| p.update(cx, |v| v.remember_responses = false));
    });
    let dir = cx.update(|cx| cx.global::<ResponseStore>().dir().to_path_buf());
    seed_v1(&dir, &s.root);
    let before = store_files(&dir);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let editor = s.shell.read_with(cx, |sh, cx| {
        sh.tab_area()
            .read(cx)
            .editor(tab)
            .expect("an editor")
            .clone()
    });
    let (model, pane) = editor.read_with(cx, |e, _| (e.send_model().clone(), e.response().clone()));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.restored().is_some()));
    pane.update(cx, |p, cx| p.show_history(0, cx));
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.viewing() == Some(0)));
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => assert_eq!(r.response.as_ref().unwrap().status(), 203),
        _ => panic!("the picked send is not shown"),
    });
    assert_eq!(
        store_files(&dir),
        before,
        "picking a send with remembering off rewrote the store"
    );
}

// A secret the request carried in its URL must not reach any file of the store: the index's
// send list and each send's json both name the URL.
fn kept_without(cx: &mut TestAppContext, s: &Started, secret: &str) {
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.response.is_some(), "no response: {:?}", result.error);
    let store = ResponseStore::new(s.files.path().join("responses"));
    wait_real(cx, |_| store.load(&s.root, &ip("a.yml")).is_some());
    let leaks: Vec<String> = store_files(store.dir())
        .into_iter()
        .filter(|(_, text)| text.contains(secret))
        .map(|(name, text)| format!("{name}: {text}"))
        .collect();
    assert!(leaks.is_empty(), "the secret is on disk in {leaks:#?}");
}

const URL_SECRET: &str = "Url-S3cret-9";

#[gpui_kit::test]
fn a_secret_variable_in_the_query_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let a = request("a", &format!("{}/ok?token={{{{token}}}}", server.uri()));
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), URL_SECRET).unwrap();
    let s = started_with_store(cx, &col, Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    kept_without(cx, &s, URL_SECRET);
}

#[gpui_kit::test]
fn an_api_key_in_the_query_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let auth = format!(
        "  auth:\n    type: apikey\n    key: api_key\n    value: {URL_SECRET}\n    placement: query\n"
    );
    let col = collection_for(&server.uri(), &auth);
    let s = started(cx, &col);
    kept_without(cx, &s, URL_SECRET);
}

#[gpui_kit::test]
fn an_oauth1_signature_in_the_query_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let auth = format!(
        "  auth:\n    type: oauth1\n    consumerKey: ck\n    consumerSecret: {URL_SECRET}\n    signatureMethod: PLAINTEXT\n    placement: query\n"
    );
    let col = collection_for(&server.uri(), &auth);
    let s = started(cx, &col);
    kept_without(cx, &s, URL_SECRET);
}

#[gpui_kit::test]
fn a_password_in_the_url_userinfo_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let uri = server
        .uri()
        .replace("http://", &format!("http://user:{URL_SECRET}@"));
    let col = collection_for(&uri, "");
    let s = started(cx, &col);
    kept_without(cx, &s, URL_SECRET);
}

#[gpui_kit::test]
fn a_kept_url_keeps_its_plain_query(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, ResponseTemplate::new(200));
    let auth = format!(
        "  auth:\n    type: apikey\n    key: api_key\n    value: {URL_SECRET}\n    placement: query\n"
    );
    let a = format!(
        "{}{auth}",
        request("a", &format!("{}/ok?page=2", server.uri()))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let s = started(cx, &col);
    kept_without(cx, &s, URL_SECRET);
    let store = ResponseStore::new(s.files.path().join("responses"));
    let index = store_files(store.dir())
        .into_iter()
        .find(|(name, _)| name == "index.json")
        .map(|(_, text)| text)
        .expect("an index");
    assert!(index.contains("page=2"), "plain query lost: {index}");
    assert!(
        index.contains("api_key=[redacted]"),
        "no placeholder: {index}"
    );
}

#[gpui_kit::test]
fn an_oauth2_token_in_the_query_is_not_kept(cx: &mut TestAppContext) {
    const TOKEN: &str = "Tok-S3cret-77";
    http_ready(cx);
    // The token endpoint and the request share one server, the token route ranked first.
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    let body = format!(r#"{{"access_token":"{TOKEN}","token_type":"Bearer","expires_in":3600}}"#);
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .with_priority(1)
            .mount(&server),
    );
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    let uri = server.uri();
    let a = format!(
        "{}  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{uri}/token\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      placement:\n        query: access_token\n",
        request("a", &format!("{uri}/ok?q={{{{token}}}}"))
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), URL_SECRET).unwrap();
    let s = started_with_store(cx, &col, Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    // With scripts off the result reports no sensitive values, so only the app's own list scrubs.
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| {
            p.update(cx, |s| s.run_scripts = RunScripts::Never)
        });
    });
    cx.run_until_parked();
    kept_without(cx, &s, TOKEN);
    // The token really went out in the query, so its absence on disk is a scrub, not a no-op.
    let received = handle
        .block_on(server.received_requests())
        .unwrap_or_default();
    assert!(
        received
            .iter()
            .any(|r| r.url.path() == "/ok" && r.url.as_str().contains(TOKEN)),
        "the token was not sent in the query"
    );
    let store = ResponseStore::new(s.files.path().join("responses"));
    let leaks: Vec<String> = store_files(store.dir())
        .into_iter()
        .filter(|(_, text)| text.contains(URL_SECRET))
        .map(|(name, _)| name)
        .collect();
    assert!(leaks.is_empty(), "the secret is on disk in {leaks:?}");
}

const ECHO_SECRET: &str = "Echo-S3cret-42";

// A server that answers every request by echoing its Authorization header back, once as a
// response header and once inside a JSON body, as httpbin-style endpoints do.
fn echo_server(cx: &mut TestAppContext) -> wiremock::MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(|req: &wiremock::Request| {
                let auth = req
                    .headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                ResponseTemplate::new(200)
                    .insert_header("x-echo-authorization", auth.as_str())
                    .set_body_raw(
                        format!(r#"{{"headers":{{"authorization":"{auth}"}}}}"#),
                        "application/json",
                    )
            })
            .mount(&server),
    );
    server
}

// An assertion that fails on the echoed header, so its kept `actual` quotes the secret.
const ECHO_ASSERTION: &str = "runtime:\n  assertions:\n    - expression: res.body.headers.authorization\n      operator: eq\n      value: \"nope\"\n";

// Sends once, checks the live response still shows the echoed secret, then that no store file
// holds it.
fn echoed_but_not_kept(cx: &mut TestAppContext, s: &Started, secret: &str) {
    let model = sender(cx, &s.ws);
    assert!(model.update(cx, |m, cx| m.send(&s.root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    let response = result.response.as_ref().expect("a response");
    assert!(
        response.text().contains(secret),
        "the live body lost the secret: {}",
        response.text()
    );
    assert!(
        response.headers().iter().any(|(_, v)| v.contains(secret)),
        "the live headers lost the secret"
    );
    let store = ResponseStore::new(s.files.path().join("responses"));
    wait_real(cx, |_| store.load(&s.root, &ip("a.yml")).is_some());
    let leaks: Vec<String> = store_files(store.dir())
        .into_iter()
        .filter(|(_, text)| text.contains(secret))
        .map(|(name, text)| format!("{name}: {text}"))
        .collect();
    assert!(leaks.is_empty(), "the secret is on disk in {leaks:#?}");
}

#[gpui_kit::test]
fn an_echoed_bearer_token_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = echo_server(cx);
    let extra = format!("  auth:\n    type: bearer\n    token: {ECHO_SECRET}\n{ECHO_ASSERTION}");
    let col = collection_for(&server.uri(), &extra);
    let s = started(cx, &col);
    echoed_but_not_kept(cx, &s, ECHO_SECRET);
}

#[gpui_kit::test]
fn an_echoed_secret_variable_header_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = echo_server(cx);
    let a = format!(
        "{}  headers:\n    - name: Authorization\n      value: \"Key {{{{token}}}}\"\n{ECHO_ASSERTION}",
        request("a", &format!("{}/ok", server.uri()))
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), ECHO_SECRET).unwrap();
    let s = started_with_store(cx, &col, Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    echoed_but_not_kept(cx, &s, ECHO_SECRET);
}

#[gpui_kit::test]
fn an_echoed_oauth2_token_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    // The token route ranks first; every other request is echoed.
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = echo_server(cx);
    let body =
        format!(r#"{{"access_token":"{ECHO_SECRET}","token_type":"Bearer","expires_in":3600}}"#);
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .with_priority(1)
            .mount(&server),
    );
    let uri = server.uri();
    let extra = format!(
        "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{uri}/token\"\n    credentials:\n      clientId: app\n{ECHO_ASSERTION}"
    );
    let col = collection_for(&uri, &extra);
    let s = started(cx, &col);
    // With scripts off the result reports no sensitive values, so only the app's own list scrubs.
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| {
            p.update(cx, |s| s.run_scripts = RunScripts::Never)
        });
    });
    cx.run_until_parked();
    echoed_but_not_kept(cx, &s, ECHO_SECRET);
}

// A response echoing an environment secret the request never references is kept with the
// secret as `[redacted]`, as a saved example would replace it. Scripts are off, so the
// expression scope's sensitive values cannot cover it; only the app's own list does.
#[gpui_kit::test]
fn an_echoed_unreferenced_environment_secret_is_not_kept(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = echo_server(cx);
    let a = format!(
        "{}  headers:\n    - name: Authorization\n      value: \"Key {ECHO_SECRET}\"\n",
        request("a", &format!("{}/ok", server.uri()))
    );
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets = apinata_core::MemoryStore::new();
    apinata_core::SecretStore::set(&secrets, &c.secret_key("dev", "token"), ECHO_SECRET).unwrap();
    let s = started_with_store(cx, &col, Arc::new(secrets));
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.update(|cx| {
        let settings = cx.global::<AppState>().settings.clone();
        settings.update(cx, |p, cx| {
            p.update(cx, |s| s.run_scripts = RunScripts::Never)
        });
    });
    cx.run_until_parked();
    echoed_but_not_kept(cx, &s, ECHO_SECRET);
    let store = ResponseStore::new(s.files.path().join("responses"));
    let kept = store.load(&s.root, &ip("a.yml")).expect("kept");
    let kept = kept.result.response.expect("a response");
    assert!(kept.text().contains("Key [redacted]"), "{}", kept.text());
}
