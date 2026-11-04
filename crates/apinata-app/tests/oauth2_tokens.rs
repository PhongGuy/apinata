//! The app sends with the shared encrypted token cache.

use crate::support;

use apinata_core::Persistence;
use gpui_kit::TestAppContext;
use support::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "tok-app-5e1d-SECRET";

fn provider(cx: &mut TestAppContext) -> MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    let body = format!(r#"{{"access_token":"{TOKEN}","token_type":"Bearer","expires_in":3600}}"#);
    handle.block_on(
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server),
    );
    handle.block_on(
        Mock::given(method("GET"))
            .and(path("/r"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server),
    );
    server
}

#[gpui_kit::test]
fn a_send_keeps_its_token_in_the_encrypted_file(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = provider(cx);
    let uri = server.uri();
    let file = format!(
        "{}  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{uri}/token\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      placement:\n        header: Bearer\n",
        request("a", &format!("{uri}/r"))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let s = started(cx, &col);
    let model = sender(cx, &s.ws);
    let root = s.root.clone();
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);

    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let received = handle
        .block_on(server.received_requests())
        .unwrap_or_default();
    let sent = received
        .iter()
        .find(|r| r.url.path() == "/r")
        .expect("the request");
    let auth = sent
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok());
    assert_eq!(auth, Some(format!("Bearer {TOKEN}").as_str()));

    let tokens = cx.update(apinata_app::oauth2_tokens::shared);
    assert_eq!(tokens.persistence(), Persistence::Encrypted);
    let file = apinata_app::persist::Paths::in_dir(s.files.path()).oauth2_tokens;
    let bytes = std::fs::read(&file).expect("the token file");
    assert!(!String::from_utf8_lossy(&bytes).contains(TOKEN));
}

#[gpui_kit::test]
fn volatile_secrets_leave_the_token_file_untouched(cx: &mut TestAppContext) {
    http_ready(cx);
    cx.update(|cx| cx.set_global(apinata_app::app::VolatileSecrets));
    let server = provider(cx);
    let uri = server.uri();
    let file = format!(
        "{}  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{uri}/token\"\n    credentials:\n      clientId: app\n    tokenConfig:\n      placement:\n        header: Bearer\n",
        request("a", &format!("{uri}/r"))
    );
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let seeded = b"tokens written by another process".to_vec();
    let s = started_with(cx, &col, |paths| {
        std::fs::write(&paths.oauth2_tokens, b"tokens written by another process").unwrap();
    });
    let model = sender(cx, &s.ws);
    let root = s.root.clone();
    assert!(model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    assert!(result.error.is_none(), "{:?}", result.error);

    let tokens = cx.update(apinata_app::oauth2_tokens::shared);
    assert!(matches!(
        tokens.persistence(),
        Persistence::MemoryOnly { .. }
    ));
    let path = apinata_app::persist::Paths::in_dir(s.files.path()).oauth2_tokens;
    assert_eq!(std::fs::read(path).unwrap(), seeded);
}
