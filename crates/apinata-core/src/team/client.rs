//! The team server's JSON API over HTTP: one reqwest client per server, bounded in time and
//! size, that never follows a redirect, so the bearer token only ever reaches this server.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER,
    USER_AGENT,
};
use reqwest::{Client, Method, RequestBuilder, Response, redirect};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use url::Url;

use super::{
    APINATA_KIND, Me, NewWorkspace, ServerInfo, ServerUrl, SignIn, Team, TeamError, TeamToken,
    User, Workspace, redact_token,
};

/// A boxed future from a `TeamApi` call.
pub type TeamFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, TeamError>> + Send + 'a>>;

/// The name every token this client asks for carries on the server.
pub const TOKEN_NAME: &str = "Apinata";

/// The largest answer body read, in bytes.
const BODY_CAP: usize = 1024 * 1024;

/// The team server calls the app and the CLI make.
pub trait TeamApi: Send + Sync {
    /// The server this talks to.
    fn server(&self) -> &ServerUrl;
    /// `GET /api/v1/server`: what the server offers before sign-in.
    fn server_info(&self) -> TeamFuture<'_, ServerInfo>;
    /// `POST /api/v1/session`: a password sign-in that mints a token named `TOKEN_NAME`.
    fn sign_in<'a>(&'a self, username: &'a str, password: &'a str) -> TeamFuture<'a, SignIn>;
    /// `GET /api/v1/me`: the user, their commit identity and their workspaces.
    fn me<'a>(&'a self, token: &'a TeamToken) -> TeamFuture<'a, Me>;
    /// `GET /api/v1/teams`: the teams on the server.
    fn teams<'a>(&'a self, token: &'a TeamToken) -> TeamFuture<'a, Vec<Team>>;
    /// `POST /api/v1/teams/{team}/workspaces`: a new seeded Apinata workspace on `main`.
    fn create_workspace<'a>(
        &'a self,
        token: &'a TeamToken,
        new: &'a NewWorkspace,
    ) -> TeamFuture<'a, Workspace>;
    /// `DELETE /api/v1/tokens/{id}`: revokes a token.
    fn revoke<'a>(&'a self, token: &'a TeamToken, token_id: &'a str) -> TeamFuture<'a, ()>;
}

/// `TeamApi` over reqwest against one server.
#[non_exhaustive]
pub struct HttpTeamClient {
    server: ServerUrl,
    http: Client,
}

impl HttpTeamClient {
    /// A client for `server`: no redirects, no proxy, 10 s to connect and 30 s per call.
    pub fn new(server: ServerUrl) -> Result<HttpTeamClient, TeamError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("apinata/", env!("CARGO_PKG_VERSION"))),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        // Uncompressed, so the body cap counts the bytes on the wire.
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let http = Client::builder()
            .default_headers(headers)
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .redirect(redirect::Policy::none())
            .referer(false)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| TeamError::Offline(e.to_string()))?;
        Ok(HttpTeamClient { server, http })
    }

    /// `{server}/api/v1/<segments>`, each segment percent-encoded, with no empty segment from a
    /// trailing slash on the server address.
    fn url(&self, segments: &[&str]) -> Result<Url, TeamError> {
        let mut url = Url::parse(self.server.as_str())
            .map_err(|_| TeamError::BadServerUrl(self.server.to_string()))?;
        let base: Vec<String> = url
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        url.path_segments_mut()
            .map_err(|_| TeamError::BadServerUrl(self.server.to_string()))?
            .clear()
            .extend(&base)
            .extend(["api", "v1"])
            .extend(segments);
        Ok(url)
    }

    fn request(
        &self,
        method: Method,
        segments: &[&str],
        token: Option<&TeamToken>,
    ) -> Result<RequestBuilder, TeamError> {
        let mut request = self.http.request(method, self.url(segments)?);
        if let Some(token) = token {
            let mut value = HeaderValue::from_str(&format!("Bearer {}", token.expose()))
                .map_err(|_| TeamError::Protocol("the session token cannot be sent".into()))?;
            value.set_sensitive(true);
            request = request.header(AUTHORIZATION, value);
        }
        Ok(request)
    }

    /// Sends `request` and returns the 2xx body; any other status maps through its error body.
    async fn send(request: RequestBuilder) -> Result<Vec<u8>, TeamError> {
        let response = request.send().await.map_err(transport)?;
        let status = response.status();
        if status.is_success() {
            return read_capped(response).await;
        }
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        // An unreadable or oversized error body still reports the status.
        let body = read_capped(response).await.unwrap_or_default();
        Err(TeamError::from_response(
            status.as_u16(),
            retry_after,
            &body,
        ))
    }

    async fn call<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, TeamError> {
        parse(&Self::send(request).await?)
    }
}

/// A JSON request body; reqwest is built without its `json` feature.
fn with_json(request: RequestBuilder, body: &serde_json::Value) -> RequestBuilder {
    request
        .header(CONTENT_TYPE, "application/json")
        .body(body.to_string())
}

fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, TeamError> {
    serde_json::from_slice(body).map_err(|e| TeamError::Protocol(redact_token(&e.to_string())))
}

/// A send or body error: a timeout is `Timeout`, anything else means the server is unreachable.
fn transport(e: reqwest::Error) -> TeamError {
    if e.is_timeout() {
        TeamError::Timeout
    } else {
        TeamError::Offline(e.to_string())
    }
}

/// The whole body, refused as soon as it is known to exceed `BODY_CAP`.
async fn read_capped(mut response: Response) -> Result<Vec<u8>, TeamError> {
    let too_big = || TeamError::Protocol(format!("the answer is larger than {BODY_CAP} bytes"));
    if response
        .content_length()
        .is_some_and(|n| n > BODY_CAP as u64)
    {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if body.len() + chunk.len() > BODY_CAP {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct ServerInfoWire {
    version: String,
    api: u32,
    auth: AuthWire,
}

#[derive(Deserialize)]
struct AuthWire {
    password: bool,
}

#[derive(Deserialize)]
struct SignInWire {
    token: String,
    token_meta: TokenMetaWire,
    user: User,
}

#[derive(Deserialize)]
struct TokenMetaWire {
    id: String,
}

impl TeamApi for HttpTeamClient {
    fn server(&self) -> &ServerUrl {
        &self.server
    }

    fn server_info(&self) -> TeamFuture<'_, ServerInfo> {
        Box::pin(async move {
            let wire: ServerInfoWire =
                Self::call(self.request(Method::GET, &["server"], None)?).await?;
            Ok(ServerInfo {
                version: wire.version,
                api: wire.api,
                password_sign_in: wire.auth.password,
            })
        })
    }

    fn sign_in<'a>(&'a self, username: &'a str, password: &'a str) -> TeamFuture<'a, SignIn> {
        Box::pin(async move {
            let body =
                json!({"username": username, "password": password, "token_name": TOKEN_NAME});
            let request = with_json(self.request(Method::POST, &["session"], None)?, &body);
            let wire: SignInWire = Self::call(request).await?;
            Ok(SignIn {
                token: TeamToken::parse(&wire.token)?,
                token_id: wire.token_meta.id,
                user: wire.user,
            })
        })
    }

    fn me<'a>(&'a self, token: &'a TeamToken) -> TeamFuture<'a, Me> {
        Box::pin(async move { Self::call(self.request(Method::GET, &["me"], Some(token))?).await })
    }

    fn teams<'a>(&'a self, token: &'a TeamToken) -> TeamFuture<'a, Vec<Team>> {
        Box::pin(
            async move { Self::call(self.request(Method::GET, &["teams"], Some(token))?).await },
        )
    }

    fn create_workspace<'a>(
        &'a self,
        token: &'a TeamToken,
        new: &'a NewWorkspace,
    ) -> TeamFuture<'a, Workspace> {
        Box::pin(async move {
            let body = json!({"slug": new.slug, "name": new.name, "kind": APINATA_KIND,
                              "default_branch": "main", "seed": true});
            let at = ["teams", new.team.as_str(), "workspaces"];
            let request = with_json(self.request(Method::POST, &at, Some(token))?, &body);
            Self::call(request).await
        })
    }

    fn revoke<'a>(&'a self, token: &'a TeamToken, token_id: &'a str) -> TeamFuture<'a, ()> {
        Box::pin(async move {
            let request = self.request(Method::DELETE, &["tokens", token_id], Some(token))?;
            Self::send(request).await.map(drop)
        })
    }
}
