//! One request to the token endpoint, and how its answer is read.

use super::OAuth2Error;
use super::config::TokenConfig;
use crate::cancel::CancelHandle;
use crate::context::VarContext;
use crate::edit::{OAuth2ParamPlacement, OAuth2Stage};
use crate::execute::{ExecError, Executor, SendTag};
use crate::request::RequestFile;
use crate::resolve::{ResolvedRequest, token_request};
use crate::secret_names::log_values;
use crate::session_log::SendSource;
use base64::Engine as _;
use serde_json::{Map, Value};
use url::Url;

/// Which grant a token request carries.
pub(crate) enum Grant<'a> {
    ClientCredentials,
    Password,
    Refresh {
        refresh_token: &'a str,
    },
    AuthorizationCode {
        code: &'a str,
        redirect_uri: &'a str,
        verifier: Option<&'a str>,
    },
}

/// The longest provider text kept, in characters.
const MAX_TEXT: usize = 300;

/// POST one token request and return the stored response object.
pub(crate) async fn request_token(
    exec: &Executor,
    cfg: &TokenConfig,
    grant: Grant<'_>,
    req: &RequestFile,
    ctx: &VarContext,
    cancel: &CancelHandle,
    tag: &SendTag,
) -> Result<Map<String, Value>, OAuth2Error> {
    let (resolved, secrets) = build(cfg, &grant, req, ctx)?;
    // The entry hides the grant's secrets and the tokens the answer issues; the values of the
    // request the token is for are computed only when a log will record the entry.
    let mut extra = Vec::new();
    if exec.session_log().is_some() {
        extra = secrets.clone();
        extra.extend(log_values(req, ctx));
    }
    let tag = SendTag {
        source: SendSource::OAuth2,
        extra,
        hold: tag.hold.clone(),
        response_tokens: true,
        stream: None,
        attempt: None,
    };
    let response = exec
        .send_as(&resolved, cancel, &tag)
        .await
        .map_err(|e| match e {
            ExecError::Cancelled => OAuth2Error::Cancelled,
            other => OAuth2Error::Exec(other),
        })?;

    let status = response.status();
    let invalid = |reason| OAuth2Error::InvalidResponse { status, reason };
    if (300..400).contains(&status) {
        return Err(invalid("a redirect"));
    }
    let Ok(body) = serde_json::from_str::<Value>(response.text()) else {
        return Err(invalid("a body that is not JSON"));
    };
    let Value::Object(mut object) = body else {
        return Err(invalid(if (200..300).contains(&status) {
            "no access_token"
        } else {
            "an error status"
        }));
    };
    // An `error` member wins over everything else, even a 200 that also carries a token.
    if let Some(error) = object.get("error").and_then(Value::as_str) {
        let description = object
            .get("error_description")
            .and_then(Value::as_str)
            .map(|d| scrub(d, &secrets));
        return Err(OAuth2Error::Provider {
            status,
            error: scrub(error, &secrets),
            description,
        });
    }
    if !(200..300).contains(&status) {
        return Err(invalid("an error status"));
    }
    if object.get("access_token").is_some_and(Value::is_string) {
        Ok(std::mem::take(&mut object))
    } else {
        Err(invalid("no access_token"))
    }
}

/// The token POST and the values to scrub from its answer: cfg.secrets, the refresh token,
/// the code, the verifier, and with Basic client auth the base64 pair and the full header value.
pub(crate) fn build(
    cfg: &TokenConfig,
    grant: &Grant<'_>,
    req: &RequestFile,
    ctx: &VarContext,
) -> Result<(ResolvedRequest, Vec<String>), OAuth2Error> {
    let refresh = matches!(grant, Grant::Refresh { .. });
    let (template, stage) = if refresh {
        (&cfg.refresh_url, OAuth2Stage::Refresh)
    } else {
        (&cfg.token_url, OAuth2Stage::Token)
    };
    let mut secrets = cfg.secrets.clone();
    let add = |secrets: &mut Vec<String>, value: &str| {
        if !value.is_empty() && !secrets.iter().any(|s| s == value) {
            secrets.push(value.to_owned());
        }
    };
    if let Grant::Refresh { refresh_token } = grant {
        add(&mut secrets, refresh_token);
    }

    let mut headers = vec![
        (
            "content-type".to_owned(),
            "application/x-www-form-urlencoded".to_owned(),
        ),
        ("Accept".to_owned(), "application/json".to_owned()),
    ];
    // Basic client auth carries the raw pair in a header, which is as secret as the pair itself;
    // otherwise the id (and a non-blank secret) travel in the body, after the grant's own fields.
    let mut client = Vec::new();
    if cfg.basic_auth {
        let pair = format!("{}:{}", cfg.client_id, cfg.client_secret);
        let encoded = base64::engine::general_purpose::STANDARD.encode(pair);
        let value = format!("Basic {encoded}");
        add(&mut secrets, &value);
        add(&mut secrets, &encoded);
        headers.push(("Authorization".to_owned(), value));
    } else {
        client.push(("client_id".to_owned(), cfg.client_id.clone()));
        if !cfg.client_secret.trim().is_empty() {
            client.push(("client_secret".to_owned(), cfg.client_secret.clone()));
        }
    }
    let field = |k: &str, v: &str| (k.to_owned(), v.to_owned());
    let scope = (!cfg.scope.is_empty()).then(|| field("scope", &cfg.scope));
    let mut form = Vec::new();
    match grant {
        Grant::ClientCredentials => {
            form.push(field("grant_type", "client_credentials"));
            form.extend(client);
            form.extend(scope);
        }
        Grant::Password => {
            form.push(field("grant_type", "password"));
            form.push(field("username", &cfg.username));
            form.push(field("password", &cfg.password));
            form.extend(client);
            form.extend(scope);
        }
        Grant::Refresh { refresh_token } => {
            form.push(field("grant_type", "refresh_token"));
            form.push(field("refresh_token", refresh_token));
            form.extend(client);
        }
        Grant::AuthorizationCode {
            code,
            redirect_uri,
            verifier,
        } => {
            form.push(field("grant_type", "authorization_code"));
            form.push(field("code", code));
            form.push(field("redirect_uri", redirect_uri));
            form.extend(client);
            form.extend(verifier.map(|v| field("code_verifier", v)));
        }
    }

    let mut url = Url::parse(&crate::resolve::with_scheme(template.clone())).map_err(|_| {
        OAuth2Error::Resolve(crate::ResolveError::InvalidUrl {
            url: "the token URL".to_owned(),
        })
    })?;
    for param in cfg.params.iter().filter(|p| p.stage == stage) {
        match param.placement {
            OAuth2ParamPlacement::Header => headers.push((param.name.clone(), param.value.clone())),
            OAuth2ParamPlacement::Query => {
                url.query_pairs_mut().append_pair(&param.name, &param.value);
            }
            OAuth2ParamPlacement::Body => form.push((param.name.clone(), param.value.clone())),
        }
    }

    let resolved = token_request(req, ctx, template, url, headers, form, &secrets)?;
    // The code and verifier are scrubbed from provider text but never marked sensitive, so a
    // short code cannot redact the token host from the request shown to the user.
    if let Grant::AuthorizationCode { code, verifier, .. } = grant {
        add(&mut secrets, code);
        add(&mut secrets, verifier.unwrap_or_default());
    }
    Ok((resolved, secrets))
}

/// Replace each secret with `[redacted]`, longest first so a secret that contains another is
/// removed whole, then cut the text to a bounded length.
pub(crate) fn scrub(text: &str, secrets: &[String]) -> String {
    let mut ordered: Vec<&String> = secrets.iter().filter(|s| !s.is_empty()).collect();
    ordered.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut out = text.to_owned();
    for secret in ordered {
        out = out.replace(secret.as_str(), "[redacted]");
    }
    out.chars().take(MAX_TEXT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::effective_oauth2;
    use crate::{
        CancelHandle, Collection, ContextOptions, ExecError, ExecOptions, Executor, ItemPath,
        RequestFile, VarContext,
    };
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SECRET: &str = "cs-77aa-SECRET";

    struct Setup {
        _dir: TempDir,
        c: Collection,
        ctx: VarContext,
    }
    impl Setup {
        fn new(auth: &str) -> Setup {
            let dir = TempDir::new().unwrap();
            let request = format!(
                "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://api.test/r\n  auth:\n{auth}"
            );
            std::fs::write(
                dir.path().join("opencollection.yml"),
                "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
            )
            .unwrap();
            std::fs::write(dir.path().join("r.yml"), request).unwrap();
            let c = Collection::open(dir.path()).unwrap();
            let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
            Setup { _dir: dir, c, ctx }
        }
        fn req(&self) -> &RequestFile {
            self.c.request(&ItemPath::new("r.yml").unwrap()).unwrap()
        }
        async fn token(
            &self,
            exec: &Executor,
            grant: Grant<'_>,
        ) -> Result<Map<String, Value>, OAuth2Error> {
            let spec = effective_oauth2(self.req(), &self.ctx).expect("oauth2");
            let cfg = TokenConfig::new(&spec, self.req(), &self.ctx).expect("config");
            request_token(
                exec,
                &cfg,
                grant,
                self.req(),
                &self.ctx,
                &CancelHandle::new(),
                &SendTag::default(),
            )
            .await
        }
    }
    fn exec() -> Executor {
        Executor::new(ExecOptions::new()).unwrap()
    }
    fn cc(server: &MockServer, extra: &str) -> String {
        format!(
            "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n      clientSecret: {SECRET}\n{extra}",
            server.uri()
        )
    }
    fn ok() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "tok", "token_type": "Bearer", "expires_in": 3600}),
        )
    }
    async fn answer(server: &MockServer, at: &str, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path(at))
            .respond_with(response)
            .mount(server)
            .await;
    }
    async fn only_request(server: &MockServer) -> wiremock::Request {
        let mut all = server.received_requests().await.expect("recording");
        assert_eq!(all.len(), 1);
        all.remove(0)
    }
    fn form(r: &wiremock::Request) -> Vec<(String, String)> {
        url::form_urlencoded::parse(&r.body).into_owned().collect()
    }
    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }
    fn header<'a>(r: &'a wiremock::Request, name: &str) -> Option<&'a str> {
        r.headers.get(name).and_then(|v| v.to_str().ok())
    }

    #[tokio::test]
    async fn client_credentials_posts_the_reference_form_in_order() {
        let server = MockServer::start().await;
        answer(&server, "/token", ok()).await;
        let params = "    scope: read write\n    additionalParameters:\n      accessTokenRequest:\n        - name: audience\n          value: https://api.test\n          placement: body\n        - name: tenant\n          value: t1\n          placement: query\n        - name: X-Trace\n          value: \"1\"\n          placement: header\n      refreshTokenRequest:\n        - name: only_on_refresh\n          value: x\n          placement: body\n";
        let token = Setup::new(&cc(&server, params))
            .token(&exec(), Grant::ClientCredentials)
            .await
            .unwrap();
        assert_eq!(token["access_token"], "tok");
        let r = only_request(&server).await;
        assert_eq!(r.method.as_str(), "POST");
        assert_eq!(
            header(&r, "content-type"),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(header(&r, "accept"), Some("application/json"));
        assert_eq!(header(&r, "x-trace"), Some("1"));
        assert_eq!(header(&r, "authorization"), None);
        assert_eq!(r.url.query(), Some("tenant=t1"));
        assert_eq!(
            form(&r),
            pairs(&[
                ("grant_type", "client_credentials"),
                ("client_id", "app"),
                ("client_secret", SECRET),
                ("scope", "read write"),
                ("audience", "https://api.test")
            ])
        );
    }

    #[tokio::test]
    async fn basic_client_auth_moves_the_credentials_to_a_header() {
        let server = MockServer::start().await;
        answer(&server, "/token", ok()).await;
        let auth = format!(
            "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n      clientSecret: \"p:ss wörd\"\n      placement: basic_auth_header\n    scope: read\n",
            server.uri()
        );
        Setup::new(&auth)
            .token(&exec(), Grant::ClientCredentials)
            .await
            .unwrap();
        let r = only_request(&server).await;
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("app:p:ss wörd")
        );
        assert_eq!(header(&r, "authorization"), Some(expected.as_str()));
        assert_eq!(
            form(&r),
            pairs(&[("grant_type", "client_credentials"), ("scope", "read")])
        );
    }

    #[tokio::test]
    async fn the_password_grant_keeps_its_order_and_leaves_a_blank_secret_out() {
        let server = MockServer::start().await;
        answer(&server, "/token", ok()).await;
        let auth = format!(
            "    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: \"{}/token\"\n    credentials:\n      clientId: app\n    resourceOwner:\n      username: ann\n      password: pw\n    scope: read\n",
            server.uri()
        );
        Setup::new(&auth)
            .token(&exec(), Grant::Password)
            .await
            .unwrap();
        assert_eq!(
            form(&only_request(&server).await),
            pairs(&[
                ("grant_type", "password"),
                ("username", "ann"),
                ("password", "pw"),
                ("client_id", "app"),
                ("scope", "read")
            ])
        );
    }

    #[tokio::test]
    async fn refresh_posts_to_the_refresh_url_with_refresh_stage_parameters() {
        let server = MockServer::start().await;
        answer(&server, "/refresh", ok()).await;
        let extra = format!(
            "    refreshTokenUrl: \"{}/refresh\"\n    additionalParameters:\n      accessTokenRequest:\n        - name: audience\n          value: a\n          placement: body\n      refreshTokenRequest:\n        - name: resource\n          value: r\n          placement: body\n",
            server.uri()
        );
        Setup::new(&cc(&server, &extra))
            .token(
                &exec(),
                Grant::Refresh {
                    refresh_token: "ref-1",
                },
            )
            .await
            .unwrap();
        let r = only_request(&server).await;
        assert_eq!(r.url.path(), "/refresh");
        assert_eq!(
            form(&r),
            pairs(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", "ref-1"),
                ("client_id", "app"),
                ("client_secret", SECRET),
                ("resource", "r")
            ])
        );
    }

    #[tokio::test]
    async fn a_provider_error_is_scrubbed_of_secrets_and_cut() {
        let server = MockServer::start().await;
        let echo = format!(
            "client secret {SECRET} and refresh ref-1 are wrong {}",
            "x".repeat(400)
        );
        answer(
            &server,
            "/token",
            ResponseTemplate::new(400)
                .set_body_json(json!({"error": "invalid_grant", "error_description": echo})),
        )
        .await;
        let err = Setup::new(&cc(&server, ""))
            .token(
                &exec(),
                Grant::Refresh {
                    refresh_token: "ref-1",
                },
            )
            .await
            .unwrap_err();
        let OAuth2Error::Provider {
            status,
            error,
            description,
        } = &err
        else {
            panic!("{err:?}")
        };
        assert_eq!((*status, error.as_str()), (400, "invalid_grant"));
        let description = description.as_deref().expect("a description");
        assert!(description.chars().count() <= 300, "{}", description.len());
        assert!(
            description.starts_with("client secret [redacted] and refresh [redacted]"),
            "{description}"
        );
        for shown in [description.to_owned(), err.to_string(), format!("{err:?}")] {
            assert!(
                !shown.contains(SECRET) && !shown.contains("ref-1"),
                "{shown}"
            );
        }
    }

    #[tokio::test]
    async fn anything_but_a_json_token_is_invalid_response() {
        let server = MockServer::start().await;
        Mock::given(path("/final"))
            .respond_with(ok())
            .expect(0)
            .mount(&server)
            .await;
        let cases = [
            (
                "/html",
                ResponseTemplate::new(200).set_body_raw("<html>secret page</html>", "text/html"),
            ),
            (
                "/redirect",
                ResponseTemplate::new(302).insert_header("location", "/final"),
            ),
            (
                "/no-token",
                ResponseTemplate::new(200).set_body_json(json!({"token_type": "Bearer"})),
            ),
        ];
        for (at, response) in cases {
            answer(&server, at, response).await;
            let auth = format!(
                "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{}{at}\"\n    credentials:\n      clientId: app\n",
                server.uri()
            );
            let err = Setup::new(&auth)
                .token(&exec(), Grant::ClientCredentials)
                .await
                .unwrap_err();
            assert!(
                matches!(err, OAuth2Error::InvalidResponse { .. }),
                "{at}: {err:?}"
            );
            assert!(!err.to_string().contains("secret page"), "{err}");
            if at == "/redirect" {
                assert!(
                    matches!(err, OAuth2Error::InvalidResponse { status: 302, .. }),
                    "{err:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn the_host_policy_judges_the_token_host() {
        // Loopback never asks the policy, so the token host is a name that is refused before any I/O.
        let auth = "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://auth.example.test/token\n    credentials:\n      clientId: app\n";
        let exec =
            Executor::new(ExecOptions::new().host_policy(Arc::new(|_: &str| false))).unwrap();
        let err = Setup::new(auth)
            .token(&exec, Grant::ClientCredentials)
            .await
            .unwrap_err();
        assert!(
            matches!(err, OAuth2Error::Exec(ExecError::HostNotAllowed { .. })),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn the_code_exchange_posts_the_reference_form_in_order() {
        let server = MockServer::start().await;
        answer(&server, "/token", ok()).await;
        let auth = format!(
            "    type: oauth2\n    flow: authorization_code\n    authorizationUrl: \"{0}/authorize\"\n    accessTokenUrl: \"{0}/token\"\n    credentials:\n      clientId: app\n      clientSecret: {SECRET}\n    scope: read\n    additionalParameters:\n      accessTokenRequest:\n        - name: audience\n          value: aud\n          placement: body\n",
            server.uri()
        );
        let grant = Grant::AuthorizationCode {
            code: "c0de",
            redirect_uri: "http://127.0.0.1:5/callback",
            verifier: Some("v3rifier"),
        };
        Setup::new(&auth).token(&exec(), grant).await.unwrap();
        let r = only_request(&server).await;
        assert_eq!(
            form(&r),
            pairs(&[
                ("grant_type", "authorization_code"),
                ("code", "c0de"),
                ("redirect_uri", "http://127.0.0.1:5/callback"),
                ("client_id", "app"),
                ("client_secret", SECRET),
                ("code_verifier", "v3rifier"),
                ("audience", "aud")
            ])
        );
    }

    #[test]
    fn the_basic_header_is_sensitive_and_the_code_and_verifier_are_scrubbed() {
        let auth = "    type: oauth2\n    flow: authorization_code\n    authorizationUrl: https://id.test/authorize\n    accessTokenUrl: https://id.test/token\n    credentials:\n      clientId: app\n      clientSecret: pw\n      placement: basic_auth_header\n";
        let s = Setup::new(auth);
        let spec = effective_oauth2(s.req(), &s.ctx).unwrap();
        let cfg = TokenConfig::new(&spec, s.req(), &s.ctx).unwrap();
        let grant = Grant::AuthorizationCode {
            code: "c0de-SECRET",
            redirect_uri: "http://127.0.0.1:5/cb",
            verifier: Some("v-SECRET"),
        };
        let (resolved, secrets) = build(&cfg, &grant, s.req(), &s.ctx).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode("app:pw");
        let header = format!("Basic {encoded}");
        assert!(resolved.sensitive_values().contains(&header));
        for secret in [encoded.as_str(), "c0de-SECRET", "v-SECRET", "pw"] {
            assert!(
                secrets.iter().any(|s| s == secret),
                "{secret} is not scrubbed"
            );
        }
    }
}
