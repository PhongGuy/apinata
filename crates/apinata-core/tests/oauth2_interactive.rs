//! Authorization code and implicit sign-ins through a fake browser.

use crate::oauth2_support;

use apinata_core::{AuthorizeOptions, CancelHandle, OAuth2Error, resolve};
use base64::Engine as _;
use oauth2_support::*;
use serde_json::json;
use sha2::Digest as _;
use std::sync::Arc;

fn signing_in(browser: &Arc<FakeBrowser>) -> AuthorizeOptions {
    AuthorizeOptions::new().authorizer(browser.clone())
}

#[test]
fn authorize_options_debug_names_the_authorizer_only() {
    let text = format!("{:?}", signing_in(&FakeBrowser::new(Play::Silent)));
    assert!(text.contains("authorizer: true"), "{text}");
}

fn authorization(f: &Fixture) -> Option<String> {
    let r = resolve(f.request("a.yml"), &f.ctx).ok()?;
    r.headers()
        .iter()
        .find(|(k, _)| k == "Authorization")
        .map(|(_, v)| v.clone())
}
async fn posts(server: &wiremock::MockServer) -> usize {
    server.received_requests().await.unwrap_or_default().len()
}

#[tokio::test]
async fn authorization_code_with_pkce_signs_in_through_the_loopback() {
    let server = wiremock::MockServer::start().await;
    on_grant(
        &server,
        "authorization_code",
        token_response(json!({"expires_in": 3600})),
        1,
    )
    .await;
    let f = single(
        &server,
        &authorization_code(&server.uri(), "    pkce: {}\n"),
    );
    let browser = FakeBrowser::new(Play::Code("c0de"));
    f.prepare("a.yml", &signing_in(&browser)).await.unwrap();

    let prompts = browser.prompts();
    assert_eq!(prompts.len(), 1);
    let p = &prompts[0];
    assert!(!p.wants_redirect);
    assert!(
        p.callback.starts_with("http://127.0.0.1:") && p.callback.ends_with("/callback"),
        "{}",
        p.callback
    );
    assert!(
        p.url.starts_with(&format!(
            "{}/authorize?response_type=code&client_id=app&",
            server.uri()
        )),
        "{}",
        p.url
    );
    assert_eq!(
        query_param(&p.url, "redirect_uri").as_deref(),
        Some(p.callback.as_str())
    );
    assert_eq!(
        query_param(&p.url, "code_challenge_method").as_deref(),
        Some("S256")
    );
    assert_eq!(query_param(&p.url, "state").map(|s| s.len()), Some(32));

    let form = forms(&server, "authorization_code").await.remove(0);
    let keys: Vec<&str> = form.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "grant_type",
            "code",
            "redirect_uri",
            "client_id",
            "client_secret",
            "code_verifier"
        ]
    );
    assert_eq!(form[1].1, "c0de");
    assert_eq!(form[2].1, p.callback);
    let digest = sha2::Sha256::digest(form[5].1.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    assert_eq!(query_param(&p.url, "code_challenge"), Some(challenge));

    assert_eq!(
        authorization(&f).as_deref(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    f.prepare("a.yml", &signing_in(&browser)).await.unwrap();
    assert_eq!(browser.prompts().len(), 1, "a cached token signed in again");
}

#[tokio::test]
async fn a_wrong_state_is_state_mismatch_and_exchanges_nothing() {
    let server = wiremock::MockServer::start().await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let err = f
        .prepare(
            "a.yml",
            &signing_in(&FakeBrowser::new(Play::WrongState("c0de"))),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, OAuth2Error::StateMismatch), "{err:?}");
    assert_eq!(posts(&server).await, 0);
}

#[tokio::test]
async fn an_error_callback_is_authorization_denied() {
    let server = wiremock::MockServer::start().await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let err = f
        .prepare(
            "a.yml",
            &signing_in(&FakeBrowser::new(Play::Deny("access_denied"))),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, OAuth2Error::AuthorizationDenied { ref error, .. } if error == "access_denied"),
        "{err:?}"
    );
    assert_eq!(posts(&server).await, 0);
}

#[tokio::test]
async fn a_cancelled_sign_in_is_cancelled() {
    let server = wiremock::MockServer::start().await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let err = f
        .prepare("a.yml", &signing_in(&FakeBrowser::new(Play::Cancel)))
        .await
        .unwrap_err();
    assert!(matches!(err, OAuth2Error::Cancelled), "{err:?}");
    assert_eq!(posts(&server).await, 0);
}

#[tokio::test]
async fn cancelling_the_send_closes_the_listener() {
    let server = wiremock::MockServer::start().await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let browser = FakeBrowser::new(Play::Silent);
    let cancel = CancelHandle::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        stop.cancel();
    });
    let err = executor()
        .prepare_auth(f.request("a.yml"), &f.ctx, &signing_in(&browser), &cancel)
        .await
        .unwrap_err();
    assert!(matches!(err, OAuth2Error::Cancelled), "{err:?}");
    let callback = url::Url::parse(&browser.prompts()[0].callback).unwrap();
    assert!(std::net::TcpStream::connect(("127.0.0.1", callback.port().unwrap())).is_err());
}

#[tokio::test]
async fn a_non_loopback_callback_takes_the_pasted_code() {
    let server = wiremock::MockServer::start().await;
    on_grant(&server, "authorization_code", token_response(json!({})), 1).await;
    let f = single(
        &server,
        &authorization_code(
            &server.uri(),
            "    callbackUrl: https://app.example.test/cb\n",
        ),
    );
    let browser = FakeBrowser::new(Play::Paste(
        "https://app.example.test/cb?code=c0de&state={state}",
    ));
    f.prepare("a.yml", &signing_in(&browser)).await.unwrap();
    let p = &browser.prompts()[0];
    assert!(p.wants_redirect);
    assert_eq!(p.callback, "https://app.example.test/cb");
    let form = forms(&server, "authorization_code").await.remove(0);
    assert!(form.contains(&(
        "redirect_uri".to_owned(),
        "https://app.example.test/cb".to_owned()
    )));
    assert!(
        !form.iter().any(|(k, _)| k == "code_verifier"),
        "pkce is off"
    );
    assert_eq!(
        authorization(&f).as_deref(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
}

#[tokio::test]
async fn implicit_takes_the_token_from_the_pasted_fragment() {
    let server = wiremock::MockServer::start().await;
    let auth = format!(
        "    type: oauth2\n    flow: implicit\n    authorizationUrl: \"{}/authorize\"\n    callbackUrl: https://app.example.test/cb\n    credentials:\n      clientId: app\n    tokenConfig:\n      id: imp\n      placement:\n        header: Bearer\n",
        server.uri()
    );
    let f = single(&server, &auth);
    let browser = FakeBrowser::new(Play::Paste(
        "https://app.example.test/cb#access_token=imp-tok&token_type=bearer&expires_in=3600&state={state}",
    ));
    f.prepare("a.yml", &signing_in(&browser)).await.unwrap();
    assert!(browser.prompts()[0].url.contains("response_type=token"));
    assert_eq!(authorization(&f).as_deref(), Some("Bearer imp-tok"));
    assert_eq!(posts(&server).await, 0);
}

#[tokio::test]
async fn implicit_without_a_callback_url_is_missing_field() {
    let server = wiremock::MockServer::start().await;
    let auth = format!(
        "    type: oauth2\n    flow: implicit\n    authorizationUrl: \"{}/authorize\"\n    credentials:\n      clientId: app\n",
        server.uri()
    );
    let browser = FakeBrowser::new(Play::Cancel);
    let err = single(&server, &auth)
        .prepare("a.yml", &signing_in(&browser))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            OAuth2Error::MissingField {
                field: "callbackUrl"
            }
        ),
        "{err:?}"
    );
    assert!(browser.prompts().is_empty());
}

#[tokio::test]
async fn concurrent_callers_share_one_sign_in() {
    let server = wiremock::MockServer::start().await;
    on_grant(&server, "authorization_code", token_response(json!({})), 1).await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let browser = FakeBrowser::new(Play::Code("c0de"));
    let how = signing_in(&browser);
    let (a, b) = tokio::join!(f.prepare("a.yml", &how), f.prepare("a.yml", &how));
    a.unwrap();
    b.unwrap();
    assert_eq!(browser.prompts().len(), 1);
}

#[tokio::test]
async fn a_busy_fixed_port_fails_before_the_browser_opens() {
    let server = wiremock::MockServer::start().await;
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let f = single(
        &server,
        &authorization_code(
            &server.uri(),
            &format!("    callbackUrl: \"http://127.0.0.1:{port}/cb\"\n"),
        ),
    );
    let browser = FakeBrowser::new(Play::Code("c0de"));
    let err = f.prepare("a.yml", &signing_in(&browser)).await.unwrap_err();
    assert!(
        matches!(err, OAuth2Error::CallbackBind { ref address, .. } if *address == format!("127.0.0.1:{port}")),
        "{err:?}"
    );
    assert!(browser.prompts().is_empty());
}

#[tokio::test]
async fn a_provider_error_never_quotes_the_code() {
    let server = wiremock::MockServer::start().await;
    let refusal = wiremock::ResponseTemplate::new(400).set_body_json(
        json!({"error": "invalid_grant", "error_description": "code code-7f3a-SECRET was used"}),
    );
    on_grant(&server, "authorization_code", refusal, 1).await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let err = f
        .prepare(
            "a.yml",
            &signing_in(&FakeBrowser::new(Play::Code("code-7f3a-SECRET"))),
        )
        .await
        .unwrap_err();
    for text in [err.to_string(), format!("{err:?}")] {
        assert!(!text.contains("code-7f3a-SECRET"), "{text}");
        assert!(text.contains("[redacted]"), "{text}");
    }
}

/// Waits until `browser` has shown its first prompt, then returns it.
async fn first_prompt(browser: &FakeBrowser) -> apinata_core::AuthorizationPrompt {
    loop {
        if let Some(p) = browser.prompts().into_iter().next() {
            return p;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Answers `prompt`'s loopback callback with `code` and `state` from a blocking browser thread.
async fn answer(prompt: &apinata_core::AuthorizationPrompt, code: &str, state: Option<&str>) {
    let state = state
        .map(str::to_owned)
        .or_else(|| query_param(&prompt.url, "state"))
        .unwrap_or_default();
    let mut url = url::Url::parse(&prompt.callback).unwrap();
    url.query_pairs_mut()
        .append_pair("code", code)
        .append_pair("state", &state);
    let url = String::from(url);
    tokio::task::spawn_blocking(move || browser_get(&url))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_caller_queued_behind_a_sign_in_can_be_cancelled() {
    use std::time::Duration;
    let server = wiremock::MockServer::start().await;
    on_grant(&server, "authorization_code", token_response(json!({})), 1).await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let browser = FakeBrowser::new(Play::Silent);
    let how = signing_in(&browser);
    let first = f.prepare("a.yml", &how);
    let rest = async {
        let prompt = first_prompt(&browser).await;
        let cancel = CancelHandle::new();
        let stop = cancel.clone();
        let exec = executor();
        let queued = exec.prepare_auth(f.request("a.yml"), &f.ctx, &how, &cancel);
        let cancelling = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            stop.cancel();
        };
        let (queued, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(queued, cancelling)
        })
        .await
        .expect("the queued caller ignored its cancel");
        assert!(matches!(queued, Err(OAuth2Error::Cancelled)), "{queued:?}");
        answer(&prompt, "c0de", None).await;
    };
    let (first, ()) = tokio::join!(first, rest);
    first.unwrap();
    assert_eq!(browser.prompts().len(), 1);
}

#[tokio::test]
async fn a_caller_cancelled_by_its_own_handle_leaves_the_queue_signing_in() {
    use std::time::Duration;
    let server = wiremock::MockServer::start().await;
    on_grant(&server, "authorization_code", token_response(json!({})), 1).await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let silent = FakeBrowser::new(Play::Silent);
    let cancel = CancelHandle::new();
    let stop = cancel.clone();
    let exec = executor();
    let silent_how = signing_in(&silent);
    let first = exec.prepare_auth(f.request("a.yml"), &f.ctx, &silent_how, &cancel);
    let answering = FakeBrowser::new(Play::Code("c0de"));
    let answering_how = signing_in(&answering);
    let second = async {
        first_prompt(&silent).await;
        let queued = f.prepare("a.yml", &answering_how);
        let cancelling = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            stop.cancel();
        };
        let (queued, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(queued, cancelling)
        })
        .await
        .expect("the queued caller never signed in");
        queued
    };
    let (a, b) = tokio::join!(first, second);
    assert!(matches!(a, Err(OAuth2Error::Cancelled)), "{a:?}");
    b.unwrap();
    assert_eq!(silent.prompts().len(), 1);
    assert_eq!(answering.prompts().len(), 1);
    assert!(authorization(&f).is_some());
}

#[tokio::test]
async fn concurrent_callers_share_a_failed_sign_in() {
    use std::time::Duration;
    let server = wiremock::MockServer::start().await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let browser = FakeBrowser::new(Play::Silent);
    let how = signing_in(&browser);
    let first = f.prepare("a.yml", &how);
    let second = async {
        first_prompt(&browser).await;
        tokio::time::timeout(Duration::from_secs(5), f.prepare("a.yml", &how))
            .await
            .expect("the queued caller signed in again")
    };
    let forge = async {
        let prompt = first_prompt(&browser).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        answer(&prompt, "c0de", Some("forged")).await;
    };
    let (a, b, ()) = tokio::join!(first, second, forge);
    assert!(matches!(a, Err(OAuth2Error::StateMismatch)), "{a:?}");
    assert!(matches!(b, Err(OAuth2Error::StateMismatch)), "{b:?}");
    assert_eq!(browser.prompts().len(), 1);
    assert_eq!(posts(&server).await, 0);

    // A later send that did not overlap the failed one signs in again.
    let retry = FakeBrowser::new(Play::Cancel);
    let err = f.prepare("a.yml", &signing_in(&retry)).await.unwrap_err();
    assert!(matches!(err, OAuth2Error::Cancelled), "{err:?}");
    assert_eq!(retry.prompts().len(), 1);
}

#[tokio::test]
async fn a_caller_with_a_cached_token_ignores_a_failed_refetch_it_queued_behind() {
    use std::time::Duration;
    let server = wiremock::MockServer::start().await;
    on_grant(&server, "authorization_code", token_response(json!({})), 1).await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    f.prepare("a.yml", &signing_in(&FakeBrowser::new(Play::Code("c0de"))))
        .await
        .unwrap();

    let browser = FakeBrowser::new(Play::Silent);
    let refetching = signing_in(&browser).refetch(true);
    let first = f.prepare("a.yml", &refetching);
    let second = async {
        first_prompt(&browser).await;
        tokio::time::timeout(
            Duration::from_secs(5),
            f.prepare("a.yml", &signing_in(&browser)),
        )
        .await
        .expect("the queued caller never finished")
    };
    let forge = async {
        let prompt = first_prompt(&browser).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        answer(&prompt, "c0de", Some("forged")).await;
    };
    let (a, b, ()) = tokio::join!(first, second, forge);
    assert!(matches!(a, Err(OAuth2Error::StateMismatch)), "{a:?}");
    b.expect("the cached token still serves the queued caller");
    assert_eq!(browser.prompts().len(), 1);
}

/// A browser whose user declines the sign-in once `decline` is notified; counts its prompts.
#[derive(Default)]
struct Declining {
    prompts: std::sync::atomic::AtomicUsize,
    decline: Arc<tokio::sync::Notify>,
}

impl apinata_core::Authorizer for Declining {
    fn authorize(
        &self,
        _: apinata_core::AuthorizationPrompt,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Option<String>, apinata_core::AuthorizeCancelled>,
                > + Send,
        >,
    > {
        self.prompts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let decline = Arc::clone(&self.decline);
        Box::pin(async move {
            decline.notified().await;
            Err(apinata_core::AuthorizeCancelled)
        })
    }
}

#[tokio::test]
async fn a_declined_sign_in_is_shared_with_the_callers_queued_behind_it() {
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    let server = wiremock::MockServer::start().await;
    let f = single(&server, &authorization_code(&server.uri(), ""));
    let browser = Arc::new(Declining::default());
    let how = AuthorizeOptions::new().authorizer(browser.clone());
    let prompted = || async {
        while browser.prompts.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let first = f.prepare("a.yml", &how);
    let second = async {
        prompted().await;
        tokio::time::timeout(Duration::from_secs(5), f.prepare("a.yml", &how))
            .await
            .expect("the queued caller signed in again")
    };
    let decline = async {
        // Nothing public shows the second caller queued, so give it ample time to get there.
        prompted().await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        browser.decline.notify_one();
    };
    let (a, b, ()) = tokio::join!(first, second, decline);
    assert!(matches!(a, Err(OAuth2Error::Cancelled)), "{a:?}");
    assert!(matches!(b, Err(OAuth2Error::Cancelled)), "{b:?}");
    assert_eq!(
        browser.prompts.load(Ordering::SeqCst),
        1,
        "one sign-in, declined once"
    );
    assert_eq!(posts(&server).await, 0);
}
