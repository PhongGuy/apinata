//! Helpers for the OAuth 2.0 engine tests: a collection whose requests use oauth2, a context
//! over one token cache, and a mock provider on 127.0.0.1.
#![allow(dead_code)] // each test binary uses a different subset

use apinata_core::{
    AuthorizationPrompt, AuthorizeCancelled, AuthorizeOptions, Authorizer, CancelHandle,
    Collection, ContextOptions, ExecOptions, Executor, ItemPath, OAuth2Error, RequestFile,
    TokenCache, VarContext,
};
use serde_json::{Value, json};
use std::fs;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
pub const TOKEN: &str = "tok-4f1c-SECRET";
pub const REFRESH: &str = "ref-9b2e-SECRET";
pub const CLIENT_SECRET: &str = "cs-77aa-SECRET";

/// A client-credentials auth block (indented under `auth:`) for `provider`'s `/token`, token id
/// `main`, Bearer header placement; `extra` lines are appended.
pub fn client_credentials(provider: &str, extra: &str) -> String {
    format!(
        "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{provider}/token\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n      placement: body\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n{extra}"
    )
}

/// The `settings` lines of an auth block.
pub fn settings(auto_fetch: bool, auto_refresh: bool) -> String {
    format!(
        "    settings:\n      autoFetchToken: {auto_fetch}\n      autoRefreshToken: {auto_refresh}\n"
    )
}

/// A GET request file at `url` whose `http.auth` is `auth`.
pub fn request(name: &str, seq: u32, url: &str, auth: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: {seq}\nhttp:\n  method: GET\n  url: \"{url}\"\n  auth:\n{auth}"
    )
}

/// A collection on disk, its context and the token cache the context uses.
pub struct Fixture {
    pub dir: TempDir,
    pub collection: Collection,
    pub ctx: VarContext,
    pub tokens: TokenCache,
}

impl Fixture {
    /// `files` with an in-memory cache and no environment.
    pub fn new(files: &[(&str, &str)]) -> Fixture {
        Fixture::with(files, ContextOptions::new(), TokenCache::in_memory())
    }

    pub fn with(files: &[(&str, &str)], options: ContextOptions, tokens: TokenCache) -> Fixture {
        let dir = TempDir::new().expect("tempdir");
        for (rel, text) in files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(path, text).expect("write");
        }
        let collection = Collection::open(dir.path()).expect("opens");
        assert!(
            collection.load_errors().is_empty(),
            "{:?}",
            collection.load_errors()
        );
        let ctx =
            VarContext::new(&collection, options.oauth2_tokens(tokens.clone())).expect("context");
        Fixture {
            dir,
            collection,
            ctx,
            tokens,
        }
    }

    pub fn request(&self, rel: &str) -> &RequestFile {
        self.collection
            .request(&ItemPath::new(rel).expect("item path"))
            .expect("request")
    }

    /// `prepare_auth` on `rel` with `how`, on a default executor.
    pub async fn prepare(&self, rel: &str, how: &AuthorizeOptions) -> Result<(), OAuth2Error> {
        executor()
            .prepare_auth(self.request(rel), &self.ctx, how, &CancelHandle::new())
            .await
    }
}

/// A collection on `server` whose requests share one client-credentials block, each with its
/// own `extra` lines (settings).
pub fn fixture(server: &MockServer, requests: &[(&str, &str)]) -> Fixture {
    let texts: Vec<(String, String)> = requests
        .iter()
        .enumerate()
        .map(|(i, (rel, extra))| {
            let url = format!("{}/r", server.uri());
            (
                (*rel).to_owned(),
                request(
                    rel,
                    i as u32 + 1,
                    &url,
                    &client_credentials(&server.uri(), extra),
                ),
            )
        })
        .collect();
    let mut files = vec![("opencollection.yml", ROOT)];
    files.extend(texts.iter().map(|(r, t)| (r.as_str(), t.as_str())));
    Fixture::new(&files)
}

pub fn executor() -> Executor {
    Executor::new(ExecOptions::new()).expect("client")
}

/// A token response: `access_token` TOKEN and `token_type` Bearer, plus `extra`'s fields.
pub fn token_response(extra: Value) -> ResponseTemplate {
    let mut body = json!({"access_token": TOKEN, "token_type": "Bearer"});
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        body.extend(extra.clone());
    }
    ResponseTemplate::new(200).set_body_json(body)
}

/// Answers POSTs to `/token` whose form carries `grant_type=<grant>`, expecting `times` of them.
pub async fn on_grant(server: &MockServer, grant: &str, response: ResponseTemplate, times: u64) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains(format!("grant_type={grant}")))
        .respond_with(response)
        .expect(times)
        .mount(server)
        .await;
}

/// The decoded form of each POST the provider received with `grant_type=<grant>`, in order.
pub async fn forms(server: &MockServer, grant: &str) -> Vec<Vec<(String, String)>> {
    server
        .received_requests()
        .await
        .expect("recording")
        .iter()
        .map(|r| {
            url::form_urlencoded::parse(&r.body)
                .into_owned()
                .collect::<Vec<_>>()
        })
        .filter(|f| f.iter().any(|(k, v)| k == "grant_type" && v == grant))
        .collect()
}

/// How the fake browser answers a prompt.
#[derive(Clone, Copy, Debug)]
pub enum Play {
    /// GET the loopback callback with this code and the prompt's state.
    Code(&'static str),
    /// GET the callback with this code and another state.
    WrongState(&'static str),
    /// GET the callback with `error=<this>` and the prompt's state.
    Deny(&'static str),
    /// Never answer.
    Silent,
    /// The user cancels.
    Cancel,
    /// Return this URL as the paste, `{state}` replaced by the prompt's state.
    Paste(&'static str),
}

/// An `Authorizer` that plays the browser and records every prompt.
pub struct FakeBrowser {
    play: Play,
    prompts: Mutex<Vec<AuthorizationPrompt>>,
}

impl FakeBrowser {
    pub fn new(play: Play) -> Arc<FakeBrowser> {
        Arc::new(FakeBrowser {
            play,
            prompts: Mutex::new(Vec::new()),
        })
    }
    pub fn prompts(&self) -> Vec<AuthorizationPrompt> {
        self.prompts.lock().unwrap().clone()
    }
}

/// The decoded value of query parameter `name` in `url`.
pub fn query_param(url: &str, name: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// A blocking GET of an `http` URL over a plain TCP connection; the raw response.
pub fn browser_get(url: &str) -> String {
    use std::io::{Read, Write};
    let u = url::Url::parse(url).expect("a URL");
    let addr = (
        u.host_str().expect("a host").trim_matches(['[', ']']),
        u.port().expect("a port"),
    );
    let mut s = std::net::TcpStream::connect(addr).expect("connect");
    let target = match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => u.path().to_owned(),
    };
    write!(
        s,
        "GET {target} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).expect("response");
    out
}

async fn visit(callback: String, query: Vec<(&'static str, String)>) {
    let mut url = url::Url::parse(&callback).expect("callback");
    url.query_pairs_mut().extend_pairs(query);
    let url = String::from(url);
    tokio::task::spawn_blocking(move || browser_get(&url))
        .await
        .expect("browser thread");
}

impl Authorizer for FakeBrowser {
    fn authorize(
        &self,
        prompt: AuthorizationPrompt,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, AuthorizeCancelled>> + Send>> {
        self.prompts.lock().unwrap().push(prompt.clone());
        let state = query_param(&prompt.url, "state").unwrap_or_default();
        let callback = prompt.callback.clone();
        let play = self.play;
        Box::pin(async move {
            match play {
                Play::Code(code) => {
                    visit(callback, vec![("code", code.to_owned()), ("state", state)]).await
                }
                Play::WrongState(code) => {
                    visit(
                        callback,
                        vec![("code", code.to_owned()), ("state", "forged".to_owned())],
                    )
                    .await
                }
                Play::Deny(error) => {
                    visit(
                        callback,
                        vec![("error", error.to_owned()), ("state", state)],
                    )
                    .await
                }
                Play::Silent => std::future::pending::<()>().await,
                Play::Cancel => return Err(AuthorizeCancelled),
                Play::Paste(url) => return Ok(Some(url.replace("{state}", &state))),
            }
            Ok(None)
        })
    }
}

/// An authorization-code auth block for `provider`: token id `main`, Bearer header; `extra` appended.
pub fn authorization_code(provider: &str, extra: &str) -> String {
    format!(
        "    type: oauth2\n    flow: authorization_code\n    authorizationUrl: \"{provider}/authorize\"\n    accessTokenUrl: \"{provider}/token\"\n    credentials:\n      clientId: app\n      clientSecret: {CLIENT_SECRET}\n    scope: read\n    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n{extra}"
    )
}

/// One request `a.yml` on `server` with `auth`.
pub fn single(server: &MockServer, auth: &str) -> Fixture {
    let text = request("a", 1, &format!("{}/r", server.uri()), auth);
    Fixture::new(&[("opencollection.yml", ROOT), ("a.yml", &text)])
}
