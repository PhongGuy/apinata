//! The interpolated settings of one oauth2 auth, with the cache key and fingerprint they map to.

use super::OAuth2Error;
use super::cache::CacheKey;
use crate::context::VarContext;
use crate::edit::{ClientAuth, OAuth2Flow, OAuth2Param, OAuth2Spec, TokenPlacement, TokenSource};
use crate::interpolate::interpolate_tracked;
use crate::request::RequestFile;
use sha2::{Digest, Sha256};
use std::cell::RefCell;

/// An oauth2 auth with every placeholder expanded. It has no `Debug` impl because it holds the
/// client secret and password.
pub(crate) struct TokenConfig {
    pub(crate) flow: OAuth2Flow,
    pub(crate) token_url: String,
    pub(crate) refresh_url: String,
    pub(crate) authorization_url: String,
    pub(crate) callback_url: String,
    pub(crate) state: String,
    pub(crate) pkce: bool,
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) basic_auth: bool,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) scope: String,
    pub(crate) credentials_id: String,
    pub(crate) placement: TokenPlacement,
    pub(crate) source: TokenSource,
    pub(crate) auto_fetch: bool,
    pub(crate) auto_refresh: bool,
    pub(crate) params: Vec<OAuth2Param>,
    pub(crate) secrets: Vec<String>,
}

impl TokenConfig {
    /// Expand only the fields the flow uses; an undefined placeholder is `Resolve` and an
    /// empty query key is `MissingField`. Nothing is sent.
    pub(crate) fn new(
        spec: &OAuth2Spec,
        req: &RequestFile,
        ctx: &VarContext,
    ) -> Result<TokenConfig, OAuth2Error> {
        let (vars, names) = ctx.lookup_for(req);
        let tracker = ctx.tracker(names);
        let generator = RefCell::new(ctx.generator());
        let expand = |text: &str| interpolate_tracked(text, &vars, Some(&tracker), &generator);

        let flow = spec.flow;
        let implicit = flow == OAuth2Flow::Implicit;
        let mut cfg = TokenConfig {
            flow,
            token_url: String::new(),
            refresh_url: String::new(),
            authorization_url: String::new(),
            callback_url: String::new(),
            state: String::new(),
            pkce: flow == OAuth2Flow::AuthorizationCode && spec.pkce,
            client_id: expand(&spec.client_id)?,
            client_secret: String::new(),
            basic_auth: spec.client_auth == Some(ClientAuth::BasicHeader),
            username: String::new(),
            password: String::new(),
            scope: expand(&spec.scope)?,
            credentials_id: match expand(&spec.token_id)? {
                id if id.is_empty() => "credentials".to_owned(),
                id => id,
            },
            placement: match &spec.token_placement {
                TokenPlacement::Header { prefix } => TokenPlacement::Header {
                    prefix: expand(prefix)?,
                },
                TokenPlacement::Query { key } => match expand(key)? {
                    key if key.is_empty() => {
                        return Err(OAuth2Error::MissingField {
                            field: "tokenConfig.placement.query",
                        });
                    }
                    key => TokenPlacement::Query { key },
                },
                TokenPlacement::Unset => TokenPlacement::Unset,
            },
            source: spec.token_source.clone(),
            auto_fetch: spec.auto_fetch,
            auto_refresh: spec.auto_refresh,
            params: Vec::new(),
            secrets: Vec::new(),
        };
        if cfg.is_interactive() {
            cfg.authorization_url = expand(&spec.authorization_url)?;
            cfg.callback_url = expand(&spec.callback_url)?;
            cfg.state = expand(&spec.state)?;
        }
        if !implicit {
            cfg.token_url = expand(&spec.access_token_url)?;
            cfg.refresh_url = match expand(&spec.refresh_token_url)? {
                url if url.is_empty() => cfg.token_url.clone(),
                url => url,
            };
            cfg.client_secret = expand(&spec.client_secret)?;
        }
        if flow == OAuth2Flow::Password {
            cfg.username = expand(&spec.username)?;
            cfg.password = expand(&spec.password)?;
        }
        for param in spec
            .params
            .iter()
            .filter(|p| flow.stages().contains(&p.stage))
        {
            let name = expand(&param.name)?;
            if !name.is_empty() {
                let value = expand(&param.value)?;
                cfg.params
                    .push(OAuth2Param::new(param.stage, name, value, param.placement));
            }
        }

        let mut secrets = vec![cfg.client_secret.clone(), cfg.password.clone()];
        secrets.extend(tracker.into_values());
        for secret in secrets {
            if !secret.is_empty() && !cfg.secrets.contains(&secret) {
                cfg.secrets.push(secret);
            }
        }
        Ok(cfg)
    }

    /// Which cached token this config reads and writes.
    pub(crate) fn key(&self, ctx: &VarContext) -> CacheKey {
        let url = if self.flow == OAuth2Flow::Implicit {
            &self.authorization_url
        } else {
            &self.token_url
        };
        CacheKey::new(
            ctx.key_root(),
            ctx.environment().map(str::to_owned),
            url.as_str(),
            self.credentials_id.as_str(),
        )
    }

    /// Identifies the settings that decide which token the provider issues, so a stored token
    /// is discarded when the flow, client or scope changes.
    pub(crate) fn fingerprint(&self) -> String {
        let text = format!("{}\0{}\0{}", self.flow.name(), self.client_id, self.scope);
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Fails with `MissingField` for the first setting a token request cannot go without.
    pub(crate) fn require_token_request(&self) -> Result<(), OAuth2Error> {
        let missing = |field| Err(OAuth2Error::MissingField { field });
        if self.token_url.is_empty() {
            return missing("accessTokenUrl");
        }
        if self.client_id.is_empty() {
            return missing("clientId");
        }
        if self.flow == OAuth2Flow::Password && self.username.is_empty() {
            return missing("username");
        }
        Ok(())
    }

    /// Fails with `MissingField` for the first setting a browser sign-in cannot go without.
    pub(crate) fn require_sign_in(&self) -> Result<(), OAuth2Error> {
        let implicit = self.flow == OAuth2Flow::Implicit;
        if implicit && self.callback_url.is_empty() {
            return Err(OAuth2Error::MissingField {
                field: "callbackUrl",
            });
        }
        if !implicit {
            self.require_token_request()?;
        }
        if self.authorization_url.is_empty() {
            return Err(OAuth2Error::MissingField {
                field: "authorizationUrl",
            });
        }
        Ok(())
    }

    /// Whether the grant needs a browser sign-in.
    pub(crate) fn is_interactive(&self) -> bool {
        matches!(
            self.flow,
            OAuth2Flow::AuthorizationCode | OAuth2Flow::Implicit
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::effective_oauth2;
    use crate::{
        Collection, ContextOptions, ItemPath, OAuth2Flow, OAuth2Spec, ResolveError, TokenPlacement,
        VarContext,
    };
    use tempfile::TempDir;

    const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
    const CC: &str = "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"https://{{host}}/token\"\n    credentials:\n      clientId: app\n      clientSecret: \"{{secret}}\"\n    scope: read\n";

    fn request(auth: &str) -> String {
        format!(
            "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://api.test/r\n  auth:\n{auth}"
        )
    }
    fn open(files: &[(&str, &str)], options: ContextOptions) -> (TempDir, Collection, VarContext) {
        let dir = TempDir::new().unwrap();
        for (rel, text) in files {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, options).unwrap();
        (dir, c, ctx)
    }
    fn vars() -> ContextOptions {
        ContextOptions::new()
            .env_var("host", "id.test")
            .sensitive_env_var("secret", "s3cret-from-env")
    }
    fn config_of(auth: &str, options: ContextOptions) -> Result<TokenConfig, OAuth2Error> {
        let (_dir, c, ctx) = open(
            &[("opencollection.yml", ROOT), ("r.yml", &request(auth))],
            options,
        );
        let req = c.request(&ItemPath::new("r.yml").unwrap()).unwrap();
        let spec = effective_oauth2(req, &ctx).expect("an oauth2 auth");
        TokenConfig::new(&spec, req, &ctx)
    }

    #[test]
    fn fields_are_interpolated_and_secrets_collected() {
        let cfg = config_of(CC, vars()).unwrap();
        assert_eq!(cfg.token_url, "https://id.test/token");
        assert_eq!(cfg.refresh_url, "https://id.test/token");
        assert_eq!(cfg.client_secret, "s3cret-from-env");
        assert_eq!(cfg.credentials_id, "credentials");
        assert!(!cfg.basic_auth);
        assert!(cfg.secrets.contains(&"s3cret-from-env".to_owned()));
        assert!(cfg.secrets.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn an_undefined_placeholder_is_undefined_variable() {
        let err = config_of(CC, ContextOptions::new().env_var("host", "id.test"))
            .err()
            .expect("an error");
        assert!(
            matches!(err, OAuth2Error::Resolve(ResolveError::UndefinedVariable { ref name, .. }) if name == "secret"),
            "{err:?}"
        );
    }

    #[test]
    fn the_refresh_url_basic_auth_token_id_and_query_key_are_read() {
        let auth = "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://id.test/token\n    refreshTokenUrl: https://id.test/refresh\n    credentials:\n      clientId: app\n      placement: basic_auth_header\n    tokenConfig:\n      id: \"{{tid}}\"\n      placement:\n        query: access\n";
        let cfg = config_of(auth, ContextOptions::new().env_var("tid", "main")).unwrap();
        assert_eq!(cfg.refresh_url, "https://id.test/refresh");
        assert!(cfg.basic_auth);
        assert_eq!(cfg.credentials_id, "main");
        assert_eq!(
            cfg.placement,
            TokenPlacement::Query {
                key: "access".into()
            }
        );
    }

    #[test]
    fn required_fields_fail_before_any_io() {
        let missing = |auth: &str| match config_of(auth, ContextOptions::new())
            .unwrap()
            .require_token_request()
        {
            Err(OAuth2Error::MissingField { field }) => field,
            other => panic!("expected MissingField, got {other:?}"),
        };
        assert_eq!(
            missing(
                "    type: oauth2\n    flow: client_credentials\n    credentials:\n      clientId: app\n"
            ),
            "accessTokenUrl"
        );
        assert_eq!(
            missing(
                "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://id.test/t\n"
            ),
            "clientId"
        );
        assert_eq!(
            missing(
                "    type: oauth2\n    flow: resource_owner_password_credentials\n    accessTokenUrl: https://id.test/t\n    credentials:\n      clientId: app\n"
            ),
            "username"
        );
    }

    #[test]
    fn an_empty_query_key_is_missing_field() {
        let auth = "    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: https://id.test/t\n    credentials:\n      clientId: app\n    tokenConfig:\n      placement:\n        query: \"\"\n";
        assert!(matches!(
            config_of(auth, ContextOptions::new()),
            Err(OAuth2Error::MissingField {
                field: "tokenConfig.placement.query"
            })
        ));
    }

    #[test]
    fn the_key_and_fingerprint_follow_the_spec() {
        let (dir, c, ctx) = open(
            &[("opencollection.yml", ROOT), ("r.yml", &request(CC))],
            vars(),
        );
        let req = c.request(&ItemPath::new("r.yml").unwrap()).unwrap();
        let spec = effective_oauth2(req, &ctx).unwrap();
        let cfg = TokenConfig::new(&spec, req, &ctx).unwrap();
        let key = cfg.key(&ctx);
        assert_eq!(key.collection, std::fs::canonicalize(dir.path()).unwrap());
        assert_eq!(key.environment, None);
        assert_eq!(key.token_url, "https://id.test/token");
        assert_eq!(key.credentials_id, "credentials");
        let fp = cfg.fingerprint();
        assert_eq!(fp.len(), 64);
        assert!(
            fp.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        let with = |change: &dyn Fn(&mut OAuth2Spec)| {
            let mut other = spec.clone();
            change(&mut other);
            TokenConfig::new(&other, req, &ctx).unwrap().fingerprint()
        };
        assert_ne!(with(&|o| o.scope = "write".into()), fp);
        assert_ne!(with(&|o| o.client_id = "app2".into()), fp);
        assert_ne!(with(&|o| o.flow = OAuth2Flow::Password), fp);
        assert_eq!(with(&|o| o.auto_fetch = false), fp);
    }

    #[test]
    fn the_environment_is_part_of_the_key() {
        let dev = "name: dev\nvariables:\n  - name: host\n    value: id.test\n";
        let files = [
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", dev),
            ("r.yml", &request(CC)),
        ];
        let (_dir, c, ctx) = open(
            &files,
            ContextOptions::new()
                .env("dev")
                .sensitive_env_var("secret", "x"),
        );
        let req = c.request(&ItemPath::new("r.yml").unwrap()).unwrap();
        let cfg = TokenConfig::new(&effective_oauth2(req, &ctx).unwrap(), req, &ctx).unwrap();
        assert_eq!(cfg.key(&ctx).environment.as_deref(), Some("dev"));
    }

    #[test]
    fn implicit_keys_by_the_authorization_url() {
        let (_dir, c, ctx) = open(
            &[
                ("opencollection.yml", ROOT),
                (
                    "r.yml",
                    &request(
                        "    type: oauth2\n    flow: implicit\n    authorizationUrl: https://id.test/authorize\n    credentials:\n      clientId: app\n",
                    ),
                ),
            ],
            ContextOptions::new(),
        );
        let req = c.request(&ItemPath::new("r.yml").unwrap()).unwrap();
        let cfg = TokenConfig::new(&effective_oauth2(req, &ctx).unwrap(), req, &ctx).unwrap();
        assert_eq!(cfg.key(&ctx).token_url, "https://id.test/authorize");
        assert!(cfg.is_interactive());
    }

    #[test]
    fn inherited_oauth2_is_found_and_other_auth_is_not() {
        let folder = format!("info:\n  name: f\n  type: folder\nrequest:\n  auth:\n{CC}");
        let inherit = "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://api.test/r\n  auth: inherit\n";
        let (bearer, flowless) = (
            request("    type: bearer\n    token: x\n"),
            request("    type: oauth2\n"),
        );
        let files = [
            ("opencollection.yml", ROOT),
            ("f/folder.yml", folder.as_str()),
            ("f/r.yml", inherit),
            ("b.yml", bearer.as_str()),
            ("n.yml", flowless.as_str()),
        ];
        let (_dir, c, ctx) = open(&files, vars());
        let at = |rel: &str| c.request(&ItemPath::new(rel).unwrap()).unwrap();
        assert_eq!(
            effective_oauth2(at("f/r.yml"), &ctx).map(|s| s.flow),
            Some(OAuth2Flow::ClientCredentials)
        );
        assert!(effective_oauth2(at("b.yml"), &ctx).is_none());
        assert!(effective_oauth2(at("n.yml"), &ctx).is_none());
    }

    #[test]
    fn the_code_flow_reads_its_authorization_fields() {
        let auth = "    type: oauth2\n    flow: authorization_code\n    authorizationUrl: \"https://{{host}}/authorize\"\n    accessTokenUrl: https://id.test/token\n    callbackUrl: \"http://localhost:{{port}}/cb\"\n    credentials:\n      clientId: app\n    state: \"{{st}}\"\n    pkce: {}\n";
        let cfg = config_of(
            auth,
            ContextOptions::new()
                .env_var("host", "id.test")
                .env_var("port", "9001")
                .env_var("st", "abc"),
        )
        .unwrap();
        assert_eq!(cfg.authorization_url, "https://id.test/authorize");
        assert_eq!(cfg.callback_url, "http://localhost:9001/cb");
        assert_eq!(cfg.state, "abc");
        assert!(cfg.pkce);
        assert_eq!(cfg.token_url, "https://id.test/token");
        let cc = config_of(CC, vars()).unwrap();
        assert!(!cc.pkce && cc.callback_url.is_empty() && cc.authorization_url.is_empty());
    }

    // The first blank setting core refuses for `spec`, by the checks a Get token runs.
    fn first_refused(spec: &OAuth2Spec) -> Option<&'static str> {
        let (_dir, c, ctx) = open(
            &[
                ("opencollection.yml", ROOT),
                ("r.yml", &request("    type: none\n")),
            ],
            ContextOptions::new(),
        );
        let req = c.request(&ItemPath::new("r.yml").unwrap()).unwrap();
        let checked = TokenConfig::new(spec, req, &ctx).and_then(|cfg| {
            if !cfg.is_interactive() {
                return cfg.require_token_request();
            }
            cfg.require_sign_in()?;
            super::super::authorize::authorization_url(&cfg, "http://127.0.0.1:9/cb", "s", None)
                .map(drop)
        });
        match checked {
            Ok(()) => None,
            Err(OAuth2Error::MissingField { field }) => Some(field),
            Err(other) => panic!("expected MissingField, got {other:?}"),
        }
    }

    // `oauth2_missing_fields` lists exactly what core refuses: blanking each setting in turn on
    // a complete spec, the list holds that setting if and only if core stops on it.
    #[test]
    fn the_editor_check_matches_what_core_refuses() {
        type Blank = fn(&mut OAuth2Spec);
        let blanks: [(&str, Blank); 6] = [
            ("authorizationUrl", |o| o.authorization_url.clear()),
            ("accessTokenUrl", |o| o.access_token_url.clear()),
            ("callbackUrl", |o| o.callback_url.clear()),
            ("clientId", |o| o.client_id.clear()),
            ("username", |o| o.username.clear()),
            ("tokenConfig.placement.query", |o| {
                o.token_placement = TokenPlacement::Query { key: String::new() }
            }),
        ];
        for flow in [
            OAuth2Flow::ClientCredentials,
            OAuth2Flow::Password,
            OAuth2Flow::AuthorizationCode,
            OAuth2Flow::Implicit,
        ] {
            let mut full = OAuth2Spec::new(flow);
            full.authorization_url = "https://id.test/authorize".into();
            full.access_token_url = "https://id.test/token".into();
            full.callback_url = "http://127.0.0.1:9/cb".into();
            full.client_id = "app".into();
            full.username = "u".into();
            full.token_placement = TokenPlacement::Query { key: "t".into() };
            assert_eq!(first_refused(&full), None, "{flow:?}");
            assert!(crate::oauth2_missing_fields(&full).is_empty(), "{flow:?}");
            for (key, blank) in blanks {
                let mut o = full.clone();
                blank(&mut o);
                let refused: Vec<_> = first_refused(&o).into_iter().collect();
                assert_eq!(crate::oauth2_missing_fields(&o), refused, "{flow:?} {key}");
            }
        }
    }
}
