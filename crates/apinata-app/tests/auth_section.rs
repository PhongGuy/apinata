use crate::support;

use apinata_app::auth_section::{AuthField, AuthMode, AuthSection};
use apinata_app::command::Edit;
use apinata_app::style::{FORM_LABEL_WIDTH, JWT_BLOCK_MAX_HEIGHT, at_zoom};
use apinata_core::{
    ApiKeyPlacement, AuthSpec, EdgeGridSpec, OAuth1SignatureMethod, OAuth1Spec, OAuth2Flow,
    OAuth2Spec,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use gpui_kit::{px, size};
use support::*;

fn auth(
    cx: &mut TestAppContext,
    spec: AuthSpec,
) -> (
    AnyWindowHandle,
    Entity<SectionHost<AuthSection>>,
    Entity<AuthSection>,
) {
    let (win, host) = host_section(cx, AuthSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host, s)
}

fn last_auth(cx: &mut TestAppContext, host: &Entity<SectionHost<AuthSection>>) -> AuthSpec {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Auth(a)) => a,
        _ => panic!("no auth edit"),
    }
}

fn mode(cx: &mut TestAppContext, win: AnyWindowHandle, s: &Entity<AuthSection>, m: AuthMode) {
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.set_mode(m, w, cx)))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

fn shown<const N: usize>(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    ids: [&'static str; N],
) -> [bool; N] {
    cx.update_window(win, |_, w, _| ids.map(|id| w.try_find(id).is_some()))
        .unwrap()
}

fn basic(u: &str, p: &str) -> AuthSpec {
    AuthSpec::Basic {
        username: u.into(),
        password: p.into(),
    }
}

#[gpui_kit::test]
fn basic_auth_shows_masked_fields_and_sends_edits(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, basic("u", "p"));
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(AuthMode::Basic));
    let pw = s.read_with(cx, |s, _| s.input(AuthField::Password).clone());
    assert_eq!(
        pw.read_with(cx, |i, _| (
            i.presentation().is_masked(),
            i.value().to_string()
        )),
        (true, "p".into())
    );
    assert_eq!(
        shown(
            cx,
            win,
            ["auth-username", "auth-password", "auth-token", "auth-key"]
        ),
        [true, true, false, false]
    );
    type_at_end(cx, win, None, "auth-username", "2");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), basic("u2", "p"));
}

#[gpui_kit::test]
fn switching_modes_sends_the_remembered_or_empty_spec(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, basic("u", "p"));
    mode(cx, win, &s, AuthMode::Bearer);
    assert_eq!(
        last_auth(cx, &host),
        AuthSpec::Bearer {
            token: String::new()
        }
    );
    let token = s.read_with(cx, |s, _| s.input(AuthField::Token).clone());
    assert!(token.read_with(cx, |i, _| i.presentation().is_masked()));
    mode(cx, win, &s, AuthMode::Basic);
    assert_eq!(last_auth(cx, &host), basic("u", "p"));
    mode(cx, win, &s, AuthMode::Inherit);
    assert_eq!(last_auth(cx, &host), AuthSpec::Inherit);
    assert_eq!(
        shown(cx, win, ["auth-inherit", "auth-username"]),
        [true, false]
    );
    mode(cx, win, &s, AuthMode::None);
    assert_eq!(last_auth(cx, &host), AuthSpec::None);
}

#[gpui_kit::test]
fn api_key_placement_and_fields(cx: &mut TestAppContext) {
    let key = |k: &str, p| AuthSpec::ApiKey {
        key: k.into(),
        value: "v".into(),
        placement: p,
    };
    let (win, host, s) = auth(cx, key("k", ApiKeyPlacement::Header));
    let value = s.read_with(cx, |s, _| s.input(AuthField::Value).clone());
    assert!(value.read_with(cx, |i, _| i.presentation().is_masked()));
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_placement(ApiKeyPlacement::Query, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), key("k", ApiKeyPlacement::Query));
    type_at_end(cx, win, None, "auth-key", "2");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), key("k2", ApiKeyPlacement::Query));
}

#[gpui_kit::test]
fn an_unsupported_mode_is_shown_read_only(cx: &mut TestAppContext) {
    let (win, host, s) = auth(
        cx,
        AuthSpec::Unsupported {
            mode: "oauth2".into(),
        },
    );
    assert_eq!(s.read_with(cx, |s, _| s.mode()), None);
    assert_eq!(
        shown(cx, win, ["auth-unsupported", "auth-username", "auth-token"]),
        [true, false, false]
    );
    assert_eq!(
        s.read_with(cx, |s, _| s.mode_placeholder()).as_deref(),
        Some("oauth2 (read-only)")
    );
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| {
            s.load(&AuthSpec::Bearer { token: "t".into() }, w, cx)
        })
    })
    .unwrap();
    assert_eq!(s.read_with(cx, |s, _| s.mode_placeholder()), None);
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let token = s.read_with(cx, |s, _| s.input(AuthField::Token).clone());
    assert_eq!(token.read_with(cx, |i, _| i.value().to_string()), "t");
    assert_eq!(shown(cx, win, ["auth-unsupported"]), [false]);
    assert!(host.read_with(cx, |h, _| h.edits.is_empty()));
}

// A request's own auth keeps the request wording.
#[gpui_kit::test]
fn no_auth_on_a_request_says_this_request(cx: &mut TestAppContext) {
    let (win, _host, _s) = auth(cx, AuthSpec::None);
    let text = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("auth-none")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(text.as_deref(), Some("This request sends no auth."));
}

fn aws(ak: &str, sk: &str, token: &str, service: &str, region: &str, profile: &str) -> AuthSpec {
    AuthSpec::AwsV4 {
        access_key_id: ak.into(),
        secret_access_key: sk.into(),
        session_token: token.into(),
        service: service.into(),
        region: region.into(),
        profile_name: profile.into(),
    }
}

#[gpui_kit::test]
fn digest_auth_edits_username_and_masked_password(cx: &mut TestAppContext) {
    let spec = AuthSpec::Digest {
        username: "u".into(),
        password: "p".into(),
    };
    let (win, host, s) = auth(cx, spec);
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(AuthMode::Digest));
    let pw = s.read_with(cx, |s, _| s.input(AuthField::Password).clone());
    assert!(pw.read_with(cx, |i, _| i.presentation().is_masked()));
    assert_eq!(
        shown(
            cx,
            win,
            ["auth-username", "auth-password", "auth-aws-region"]
        ),
        [true, true, false]
    );
    type_at_end(cx, win, None, "auth-password", "2");
    cx.run_until_parked();
    assert_eq!(
        last_auth(cx, &host),
        AuthSpec::Digest {
            username: "u".into(),
            password: "p2".into()
        }
    );
}

#[gpui_kit::test]
fn awsv4_auth_shows_six_fields_with_secrets_masked(cx: &mut TestAppContext) {
    let (win, host, s) = auth(
        cx,
        aws("AK", "SK", "TOK", "execute-api", "eu-west-1", "dev"),
    );
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(AuthMode::AwsV4));
    let masked = |cx: &mut TestAppContext, f: AuthField| {
        let input = s.read_with(cx, |s, _| s.input(f).clone());
        input.read_with(cx, |i, _| i.presentation().is_masked())
    };
    assert!(!masked(cx, AuthField::AccessKeyId));
    assert!(masked(cx, AuthField::SecretAccessKey));
    assert!(masked(cx, AuthField::SessionToken));
    assert!(!masked(cx, AuthField::ProfileName));
    assert_eq!(
        shown(
            cx,
            win,
            [
                "auth-aws-access-key-id",
                "auth-aws-secret-access-key",
                "auth-aws-session-token",
                "auth-aws-service",
                "auth-aws-region",
                "auth-aws-profile-name",
                "auth-username"
            ]
        ),
        [true, true, true, true, true, true, false]
    );
    type_at_end(cx, win, None, "auth-aws-region", "x");
    cx.run_until_parked();
    assert_eq!(
        last_auth(cx, &host),
        aws("AK", "SK", "TOK", "execute-api", "eu-west-1x", "dev")
    );
}

#[gpui_kit::test]
fn switching_to_the_new_modes_sends_empty_specs_and_back_restores(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, basic("u", "p"));
    mode(cx, win, &s, AuthMode::AwsV4);
    assert_eq!(last_auth(cx, &host), aws("", "", "", "", "", ""));
    mode(cx, win, &s, AuthMode::Digest);
    assert_eq!(
        last_auth(cx, &host),
        AuthSpec::Digest {
            username: String::new(),
            password: String::new()
        }
    );
    mode(cx, win, &s, AuthMode::Basic);
    assert_eq!(last_auth(cx, &host), basic("u", "p"));
    assert_eq!(
        AuthMode::ALL.iter().map(|m| m.label()).collect::<Vec<_>>(),
        [
            "No Auth",
            "Inherit",
            "Basic Auth",
            "Bearer Token",
            "API Key",
            "Digest Auth",
            "NTLM",
            "AWS Sig v4",
            "WSSE",
            "Akamai EdgeGrid",
            "OAuth 1.0",
            "OAuth 2.0"
        ]
    );
}

fn wsse(u: &str, p: &str) -> AuthSpec {
    AuthSpec::Wsse {
        username: u.into(),
        password: p.into(),
    }
}

#[gpui_kit::test]
fn wsse_edits_username_and_masked_password(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, wsse("u", "p"));
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(AuthMode::Wsse));
    let masked = |cx: &mut TestAppContext, f: AuthField| {
        let input = s.read_with(cx, |s, _| s.input(f).clone());
        input.read_with(cx, |i, _| i.presentation().is_masked())
    };
    assert!(masked(cx, AuthField::Password));
    assert!(!masked(cx, AuthField::Username));
    assert_eq!(
        shown(cx, win, ["auth-username", "auth-password", "auth-token"]),
        [true, true, false]
    );
    type_at_end(cx, win, None, "auth-password", "2");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), wsse("u", "p2"));
    type_at_end(cx, win, None, "auth-username", "x");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), wsse("ux", "p2"));
}

#[gpui_kit::test]
fn switching_to_wsse_sends_an_empty_spec_and_back_restores(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, basic("u", "p"));
    mode(cx, win, &s, AuthMode::Wsse);
    assert_eq!(last_auth(cx, &host), wsse("", ""));
    type_at_end(cx, win, None, "auth-username", "w");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), wsse("w", ""));
    mode(cx, win, &s, AuthMode::Basic);
    assert_eq!(last_auth(cx, &host), basic("u", "p"));
    mode(cx, win, &s, AuthMode::Wsse);
    assert_eq!(last_auth(cx, &host), wsse("w", ""));
}

fn eg(f: impl FnOnce(&mut EdgeGridSpec)) -> AuthSpec {
    let mut s = EdgeGridSpec::default();
    f(&mut s);
    AuthSpec::EdgeGrid(s)
}

fn filled(s: &mut EdgeGridSpec) {
    s.access_token = "at".into();
    s.client_token = "ct".into();
    s.client_secret = "cs".into();
    s.base_url = "https://b.test".into();
    s.nonce = "n".into();
    s.timestamp = "t".into();
    s.headers_to_sign = "X-A".into();
    s.max_body_size = Some(2048.0);
}

#[gpui_kit::test]
fn edgegrid_shows_its_fields_with_credentials_masked(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, eg(filled));
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(AuthMode::EdgeGrid));
    let masked = |cx: &mut TestAppContext, f: AuthField| {
        let input = s.read_with(cx, |s, _| s.input(f).clone());
        input.read_with(cx, |i, _| i.presentation().is_masked())
    };
    for f in [
        AuthField::AccessToken,
        AuthField::ClientToken,
        AuthField::ClientSecret,
    ] {
        assert!(masked(cx, f), "{f:?}");
    }
    for f in [
        AuthField::BaseUrl,
        AuthField::Nonce,
        AuthField::Timestamp,
        AuthField::HeadersToSign,
        AuthField::MaxBodySize,
    ] {
        assert!(!masked(cx, f), "{f:?}");
    }
    assert_eq!(
        shown(
            cx,
            win,
            [
                "auth-eg-access-token",
                "auth-eg-client-token",
                "auth-eg-client-secret",
                "auth-eg-base-url",
                "auth-eg-nonce",
                "auth-eg-timestamp",
                "auth-eg-headers-to-sign",
                "auth-eg-max-body-size",
                "auth-username"
            ]
        ),
        [true, true, true, true, true, true, true, true, false]
    );
    type_at_end(cx, win, None, "auth-eg-headers-to-sign", ",X-B");
    cx.run_until_parked();
    assert_eq!(
        last_auth(cx, &host),
        eg(|s| {
            filled(s);
            s.headers_to_sign = "X-A,X-B".into();
        })
    );
}

#[gpui_kit::test]
fn switching_to_edgegrid_sends_the_default_spec_and_back_restores(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, basic("u", "p"));
    mode(cx, win, &s, AuthMode::EdgeGrid);
    assert_eq!(
        last_auth(cx, &host),
        AuthSpec::EdgeGrid(EdgeGridSpec::default())
    );
    type_at_end(cx, win, None, "auth-eg-access-token", "a");
    cx.run_until_parked();
    let typed = eg(|s| s.access_token = "a".into());
    assert_eq!(last_auth(cx, &host), typed);
    mode(cx, win, &s, AuthMode::Basic);
    assert_eq!(last_auth(cx, &host), basic("u", "p"));
    mode(cx, win, &s, AuthMode::EdgeGrid);
    assert_eq!(last_auth(cx, &host), typed);
}

#[gpui_kit::test]
fn max_body_size_takes_a_number_clears_when_empty_and_refuses_text(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, eg(|s| s.max_body_size = Some(2048.0)));
    let max = s.read_with(cx, |s, _| s.input(AuthField::MaxBodySize).clone());
    assert_eq!(
        max.read_with(cx, |i, _| (
            i.presentation().is_masked(),
            i.value().to_string()
        )),
        (false, "2048".into())
    );
    replace_input(cx, win, None, "auth-eg-max-body-size", "1.5");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), eg(|s| s.max_body_size = Some(1.5)));

    let before = host.read_with(cx, |h, _| h.edits.len());
    type_at_end(cx, win, None, "auth-eg-max-body-size", "kb");
    cx.run_until_parked();
    assert!(present(cx, win, "auth-eg-max-body-size-error"));
    assert_eq!(
        host.read_with(cx, |h, _| h.edits.len()),
        before,
        "a non-number is not sent"
    );
    type_at_end(cx, win, None, "auth-eg-nonce", "n");
    cx.run_until_parked();
    assert_eq!(
        last_auth(cx, &host),
        eg(|s| {
            s.nonce = "n".into();
            s.max_body_size = Some(1.5);
        })
    );

    replace_input(cx, win, None, "auth-eg-max-body-size", "");
    cx.run_until_parked();
    assert!(!present(cx, win, "auth-eg-max-body-size-error"));
    assert_eq!(last_auth(cx, &host), eg(|s| s.nonce = "n".into()));
    replace_input(cx, win, None, "auth-eg-max-body-size", "4096");
    cx.run_until_parked();
    assert_eq!(
        last_auth(cx, &host),
        eg(|s| {
            s.nonce = "n".into();
            s.max_body_size = Some(4096.0);
        })
    );
}

#[gpui_kit::test]
fn a_loaded_max_body_size_is_spelled_the_way_javascript_spells_it(cx: &mut TestAppContext) {
    for (value, want) in [
        (1e300, "1e+300"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (f64::NAN, "NaN"),
        (2048.0, "2048"),
        (1.5, "1.5"),
    ] {
        let (_win, _host, s) = auth(cx, eg(|s| s.max_body_size = Some(value)));
        let max = s.read_with(cx, |s, _| s.input(AuthField::MaxBodySize).clone());
        assert_eq!(max.read_with(cx, |i, _| i.value().to_string()), want);
    }
}

#[gpui_kit::test]
fn auth_fields_use_the_form_column_and_stack_when_narrow(cx: &mut TestAppContext) {
    let (win, _host, _s) = auth(cx, basic("u", "p"));
    frame(cx, win);
    let parts = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, _| {
            (
                w.within("auth-username-row").find("form-label").bounds(),
                w.find("auth-username").bounds(),
            )
        })
        .unwrap()
    };
    let (label, input) = parts(cx);
    assert_eq!(label.size.width, at_zoom(FORM_LABEL_WIDTH, 1.0));
    assert!(label.right() <= input.left(), "{label:?} beside {input:?}");
    assert_eq!(label.top(), input.top());
    cx.simulate_window_resize(win, size(px(400.), px(600.)));
    frame(cx, win);
    frame(cx, win);
    let (label, input) = parts(cx);
    assert!(label.bottom() <= input.top(), "{label:?} over {input:?}");
}

#[gpui_kit::test]
fn the_api_key_placement_sits_in_a_form_row(cx: &mut TestAppContext) {
    let (win, _host, s) = auth(cx, basic("u", "p"));
    mode(cx, win, &s, AuthMode::ApiKey);
    let found = cx
        .update_window(win, |_, w, _| {
            [
                w.within("auth-placement-row")
                    .try_find("form-label")
                    .is_some(),
                w.within("auth-placement-row")
                    .try_find("auth-placement")
                    .is_some(),
                w.within("auth-key-row").try_find("auth-key").is_some(),
            ]
        })
        .unwrap();
    assert_eq!(found, [true, true, true]);
}

#[gpui_kit::test]
fn every_secret_input_shows_a_reference_and_masks_a_literal(cx: &mut TestAppContext) {
    let shown = [
        (
            AuthSpec::Bearer {
                token: "{{token}}".into(),
            },
            AuthField::Token,
        ),
        (basic("u", "  {{user.pw}} "), AuthField::Password),
        (
            AuthSpec::ApiKey {
                key: "k".into(),
                value: "{{process.env.KEY}}".into(),
                placement: ApiKeyPlacement::Header,
            },
            AuthField::Value,
        ),
    ];
    for (spec, field) in shown {
        let (_win, _host, s) = auth(cx, spec);
        let input = s.read_with(cx, |s, _| s.input(field).clone());
        assert!(
            !input.read_with(cx, |i, _| i.presentation().is_masked()),
            "{field:?} shows its reference"
        );
    }
    let hidden = [
        (
            AuthSpec::Bearer {
                token: "Bearer-{{token}}".into(),
            },
            AuthField::Token,
        ),
        (basic("u", "hunter2"), AuthField::Password),
    ];
    for (spec, field) in hidden {
        let (_win, _host, s) = auth(cx, spec);
        let input = s.read_with(cx, |s, _| s.input(field).clone());
        assert!(
            input.read_with(cx, |i, _| i.presentation().is_masked()),
            "{field:?} masks other text"
        );
    }
}

#[gpui_kit::test]
fn typing_a_reference_shows_it_and_typing_on_masks_it(cx: &mut TestAppContext) {
    let (win, _host, s) = auth(cx, aws("AK", "", "TOK", "execute-api", "eu-west-1", "dev"));
    let input = s.read_with(cx, |s, _| s.input(AuthField::SecretAccessKey).clone());
    type_at_end(
        cx,
        win,
        None,
        "auth-aws-secret-access-key",
        "{{aws.secret}}",
    );
    frame(cx, win);
    assert!(!input.read_with(cx, |i, _| i.presentation().is_masked()));
    type_at_end(cx, win, None, "auth-aws-secret-access-key", "x");
    frame(cx, win);
    assert!(input.read_with(cx, |i, _| i.presentation().is_masked()));
}

fn ntlm(u: &str, p: &str, d: &str) -> AuthSpec {
    AuthSpec::Ntlm {
        username: u.into(),
        password: p.into(),
        domain: d.into(),
    }
}

#[gpui_kit::test]
fn ntlm_edits_username_masked_password_and_domain(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, ntlm("u", "p", "CORP"));
    assert_eq!(s.read_with(cx, |s, _| s.mode()), Some(AuthMode::Ntlm));
    let masked = |cx: &mut TestAppContext, f: AuthField| {
        let input = s.read_with(cx, |s, _| s.input(f).clone());
        input.read_with(cx, |i, _| i.presentation().is_masked())
    };
    assert!(masked(cx, AuthField::Password));
    assert!(!masked(cx, AuthField::Username));
    assert!(!masked(cx, AuthField::Domain));
    assert_eq!(
        shown(
            cx,
            win,
            [
                "auth-username",
                "auth-password",
                "auth-domain",
                "auth-token"
            ]
        ),
        [true, true, true, false]
    );
    type_at_end(cx, win, None, "auth-password", "2");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), ntlm("u", "p2", "CORP"));
    type_at_end(cx, win, None, "auth-domain", "X");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), ntlm("u", "p2", "CORPX"));
    type_at_end(cx, win, None, "auth-username", "v");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), ntlm("uv", "p2", "CORPX"));
}

// Plain http through an HTTP proxy needs the proxy's help, so the form points at https.
#[gpui_kit::test]
fn ntlm_notes_that_https_avoids_the_proxy_pitfall(cx: &mut TestAppContext) {
    let (win, _host, _s) = auth(cx, ntlm("u", "p", "CORP"));
    let text = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("auth-ntlm-proxy-note")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap();
    assert_eq!(
        text.as_deref(),
        Some(
            "When you go through a proxy, use an https address. Over plain http, sign-in can \
             fail as if the password were wrong."
        )
    );
}

#[gpui_kit::test]
fn switching_to_ntlm_sends_an_empty_spec_and_back_restores(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, basic("u", "p"));
    mode(cx, win, &s, AuthMode::Ntlm);
    assert_eq!(last_auth(cx, &host), ntlm("", "", ""));
    type_at_end(cx, win, None, "auth-domain", "D");
    cx.run_until_parked();
    assert_eq!(last_auth(cx, &host), ntlm("", "", "D"));
    mode(cx, win, &s, AuthMode::Basic);
    assert_eq!(last_auth(cx, &host), basic("u", "p"));
    mode(cx, win, &s, AuthMode::Ntlm);
    assert_eq!(last_auth(cx, &host), ntlm("", "", "D"));
    mode(cx, win, &s, AuthMode::Wsse);
    assert!(shown(cx, win, ["auth-domain"]) == [false]);
}

#[gpui_kit::test]
fn a_reference_in_the_ntlm_password_is_shown_and_a_literal_is_masked(cx: &mut TestAppContext) {
    let (_win, _host, s) = auth(cx, ntlm("u", "{{ntlm.pw}}", "D"));
    let pw = s.read_with(cx, |s, _| s.input(AuthField::Password).clone());
    assert!(!pw.read_with(cx, |i, _| i.presentation().is_masked()));
    let (_win, _host, s) = auth(cx, ntlm("u", "hunter2", "D"));
    let pw = s.read_with(cx, |s, _| s.input(AuthField::Password).clone());
    assert!(pw.read_with(cx, |i, _| i.presentation().is_masked()));
}

mod jwt_view {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use std::sync::Arc;

    const ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\n";

    fn jwt(payload: &str) -> String {
        let p = |s: &str| URL_SAFE_NO_PAD.encode(s);
        format!("{}.{}.c2lnbmF0dXJl", p(r#"{"alg":"HS256"}"#), p(payload))
    }

    // A request whose `tok` variable is a secret holding `token`.
    fn secret_vars(token: &str) -> (tempfile::TempDir, Arc<apinata_core::VariableSet>) {
        let dir = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request("a", "x"))]);
        let c = apinata_core::Collection::open(dir.path()).unwrap();
        let options = apinata_core::ContextOptions::new()
            .sensitive_env_var("tok", token)
            .process_env(Vec::<(String, String)>::new());
        let ctx = apinata_core::VarContext::new(&c, options).unwrap();
        (
            dir,
            Arc::new(ctx.variables(c.request(&ip("a.yml")).unwrap())),
        )
    }

    #[gpui_kit::test]
    fn a_secret_variable_token_shows_claims_but_not_its_text(cx: &mut TestAppContext) {
        let token = jwt(r#"{"sub":"alice","exp":1}"#);
        let (_dir, vars) = secret_vars(&token);
        let (win, _host, s) = auth(
            cx,
            AuthSpec::Bearer {
                token: "{{tok}}".into(),
            },
        );
        s.update(cx, |s, cx| s.set_variables(Some(vars), cx));
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        assert_eq!(shown(cx, win, ["auth-jwt"]), [true]);
        let view = s.read_with(cx, |s, _| s.jwt_view()).expect("decoded");
        assert!(view.payload.contains("alice"), "{view:?}");
        assert!(view.header.contains("HS256"), "{view:?}");
        assert!(view.expiry.contains("expired"), "{view:?}");
        // Nothing the feature shows carries the raw token or its signature.
        let everything = format!("{view:?}");
        assert!(!everything.contains(&token));
        assert!(!everything.contains("c2lnbmF0dXJl"));
    }

    #[gpui_kit::test]
    fn a_literal_token_decodes_and_a_non_token_shows_nothing(cx: &mut TestAppContext) {
        let (win, _host, s) = auth(
            cx,
            AuthSpec::Bearer {
                token: jwt(r#"{"sub":"bob"}"#),
            },
        );
        let view = s.read_with(cx, |s, _| s.jwt_view()).expect("decoded");
        assert!(view.expiry.contains("No expiry"), "{view:?}");
        // Typing a fourth part makes it no longer a token, which clears the block.
        type_at_end(cx, win, None, "auth-token", ".x");
        cx.run_until_parked();
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        assert!(s.read_with(cx, |s, _| s.jwt_view()).is_none());
        assert_eq!(shown(cx, win, ["auth-jwt"]), [false]);
    }

    #[gpui_kit::test]
    fn a_large_payload_scrolls_inside_a_capped_block(cx: &mut TestAppContext) {
        let claims: Vec<String> = (0..200)
            .map(|i| format!(r#""claim{i}":"value{i}""#))
            .collect();
        let (win, _host, _s) = auth(
            cx,
            AuthSpec::Bearer {
                token: jwt(&format!("{{{}}}", claims.join(","))),
            },
        );
        cx.update_window(win, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let height = cx
            .update_window(win, |_, w, _| w.find("auth-jwt").bounds().size.height)
            .unwrap();
        assert!(
            height <= at_zoom(JWT_BLOCK_MAX_HEIGHT, 1.0),
            "decoded block grew to {height:?}"
        );
    }
}

// The eye button reveals a secret, which stays shown through later renders and typing, and
// a second press hides it again.
#[gpui_kit::test]
fn a_secret_field_reveals_with_its_eye_and_stays_shown_while_typing(cx: &mut TestAppContext) {
    let (win, host, s) = auth(cx, basic("u", "hunter2"));
    let pw = s.read_with(cx, |s, _| s.input(AuthField::Password).clone());
    let masked = |cx: &mut TestAppContext| pw.read_with(cx, |i, _| i.presentation().is_masked());
    let eye = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.within("auth-password").click("toggle-mask", cx)
        })
        .unwrap();
        frame(cx, win);
        frame(cx, win);
    };
    assert!(masked(cx));
    eye(cx);
    assert!(!masked(cx), "the eye shows the password");
    type_at_end(cx, win, None, "auth-password", "!");
    frame(cx, win);
    assert!(!masked(cx), "typing keeps it shown");
    assert_eq!(last_auth(cx, &host), basic("u", "hunter2!"));
    eye(cx);
    assert!(masked(cx), "a second press hides it");
}

// Every masked input in the Auth tab, the OAuth editors included, has the eye button, and a
// plain one has none.
#[gpui_kit::test]
fn every_secret_input_has_an_eye_button(cx: &mut TestAppContext) {
    let cases: [(AuthMode, &[&str]); 10] = [
        (AuthMode::Basic, &["auth-password"]),
        (AuthMode::Digest, &["auth-password"]),
        (AuthMode::Wsse, &["auth-password"]),
        (AuthMode::Ntlm, &["auth-password"]),
        (AuthMode::Bearer, &["auth-token"]),
        (AuthMode::ApiKey, &["auth-value"]),
        (
            AuthMode::AwsV4,
            &["auth-aws-secret-access-key", "auth-aws-session-token"],
        ),
        (
            AuthMode::EdgeGrid,
            &[
                "auth-eg-access-token",
                "auth-eg-client-token",
                "auth-eg-client-secret",
            ],
        ),
        (
            AuthMode::OAuth1,
            &["oauth1-consumer-secret", "oauth1-token-secret"],
        ),
        (AuthMode::OAuth2, &["oauth2-client-secret"]),
    ];
    let (win, _host, s) = auth(cx, AuthSpec::None);
    for (m, ids) in cases {
        mode(cx, win, &s, m);
        frame(cx, win);
        for id in ids {
            let has = cx
                .update_window(win, |_, w, _| {
                    w.within(*id).try_find("toggle-mask").is_some()
                })
                .unwrap();
            assert!(has, "{m:?} {id} has the eye");
        }
    }
    mode(cx, win, &s, AuthMode::Basic);
    let plain = cx
        .update_window(win, |_, w, _| {
            w.within("auth-username").try_find("toggle-mask").is_some()
        })
        .unwrap();
    assert!(!plain, "a plain field has no eye");
    // The RSA private key and the password grant's password show only for their settings.
    let mut rsa = OAuth1Spec::default();
    rsa.signature_method = OAuth1SignatureMethod::RsaSha256;
    let (win, _host, _s) = auth(cx, AuthSpec::OAuth1(rsa));
    frame(cx, win);
    let password_grant = AuthSpec::OAuth2(OAuth2Spec::new(OAuth2Flow::Password));
    let (win2, _host2, _s2) = auth(cx, password_grant);
    frame(cx, win2);
    for (w, id) in [(win, "oauth1-private-key"), (win2, "oauth2-password")] {
        let has = cx
            .update_window(w, |_, w, _| w.within(id).try_find("toggle-mask").is_some())
            .unwrap();
        assert!(has, "{id} has the eye");
    }
}

// Fields whose format is not obvious show a short example while empty.
#[gpui_kit::test]
fn auth_fields_show_example_placeholders(cx: &mut TestAppContext) {
    let (_win, _host, s) = auth(cx, AuthSpec::None);
    let hint = |cx: &mut TestAppContext, f: AuthField| {
        let input = s.read_with(cx, |s, _| s.input(f).clone());
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string())
    };
    let expected = [
        (AuthField::Key, "X-API-Key"),
        (AuthField::Domain, "CORP"),
        (AuthField::AccessKeyId, "AKIAIOSFODNN7EXAMPLE"),
        (AuthField::Service, "execute-api"),
        (AuthField::Region, "us-east-1"),
        (AuthField::ProfileName, "default"),
        (AuthField::BaseUrl, "https://akab-xxxx.luna.akamaiapis.net"),
        (AuthField::Nonce, "Generated when sent"),
        (AuthField::Timestamp, "Generated when sent"),
        (AuthField::HeadersToSign, "X-Header-One, X-Header-Two"),
        (AuthField::MaxBodySize, "131072"),
    ];
    for (f, want) in expected {
        assert_eq!(hint(cx, f), want, "{f:?}");
    }
}

// Field labels use sentence case, as the rest of the app does.
#[gpui_kit::test]
fn auth_field_labels_use_sentence_case(cx: &mut TestAppContext) {
    let (win, _host, s) = auth(cx, AuthSpec::None);
    let label = |cx: &mut TestAppContext, row: &'static str| {
        cx.update_window(win, |_, w, _| {
            w.within(row).find("form-label").label().map(str::to_owned)
        })
        .unwrap()
    };
    mode(cx, win, &s, AuthMode::ApiKey);
    assert_eq!(label(cx, "auth-placement-row").as_deref(), Some("Add to"));
    mode(cx, win, &s, AuthMode::AwsV4);
    let aws = [
        ("auth-aws-access-key-id-row", "Access key ID"),
        ("auth-aws-secret-access-key-row", "Secret access key"),
        ("auth-aws-session-token-row", "Session token"),
        ("auth-aws-profile-name-row", "Profile name"),
    ];
    for (row, want) in aws {
        assert_eq!(label(cx, row).as_deref(), Some(want), "{row}");
    }
    mode(cx, win, &s, AuthMode::EdgeGrid);
    let edgegrid = [
        ("auth-eg-access-token-row", "Access token"),
        ("auth-eg-client-token-row", "Client token"),
        ("auth-eg-client-secret-row", "Client secret"),
        ("auth-eg-headers-to-sign-row", "Headers to sign"),
        ("auth-eg-max-body-size-row", "Max body size"),
    ];
    for (row, want) in edgegrid {
        assert_eq!(label(cx, row).as_deref(), Some(want), "{row}");
    }
}

// Presses the eye of the secret input `id` and draws.
fn press_eye(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) {
    cx.update_window(win, |_, w, cx| w.within(id).click("toggle-mask", cx))
        .unwrap();
    frame(cx, win);
    frame(cx, win);
}

// The password input is shared by several types, so a reveal must not carry a value the
// user did not type into view.
#[gpui_kit::test]
fn a_revealed_password_is_hidden_again_when_the_auth_type_changes(cx: &mut TestAppContext) {
    let (win, _host, s) = auth(cx, basic("u", "hunter2"));
    let pw = s.read_with(cx, |s, _| s.input(AuthField::Password).clone());
    press_eye(cx, win, "auth-password");
    assert!(!pw.read_with(cx, |i, _| i.presentation().is_masked()));
    mode(cx, win, &s, AuthMode::Digest);
    frame(cx, win);
    assert!(pw.read_with(cx, |i, _| i.presentation().is_masked()));
}

// A reload from the file that brings a new value hides a revealed field again.
#[gpui_kit::test]
fn a_revealed_password_is_hidden_again_when_a_reload_brings_a_new_value(cx: &mut TestAppContext) {
    let (win, _host, s) = auth(cx, basic("u", "hunter2"));
    let pw = s.read_with(cx, |s, _| s.input(AuthField::Password).clone());
    press_eye(cx, win, "auth-password");
    assert!(!pw.read_with(cx, |i, _| i.presentation().is_masked()));
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&basic("u", "swordfish"), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert!(pw.read_with(cx, |i, _| i.presentation().is_masked()));
}

// The OAuth 2.0 client secret is hidden again when a reload brings a new one.
#[gpui_kit::test]
fn a_revealed_oauth2_secret_is_hidden_again_when_a_reload_brings_a_new_value(
    cx: &mut TestAppContext,
) {
    let spec = |secret: &str| {
        let mut o = OAuth2Spec::new(OAuth2Flow::ClientCredentials);
        o.client_secret = secret.into();
        AuthSpec::OAuth2(o)
    };
    let (win, _host, s) = auth(cx, spec("first"));
    frame(cx, win);
    let editor = s.read_with(cx, |s, _| s.oauth2().clone());
    let secret = editor.read_with(cx, |e, _| {
        e.input(apinata_app::oauth2_editor::OAuth2Field::ClientSecret)
            .clone()
    });
    press_eye(cx, win, "oauth2-client-secret");
    assert!(!secret.read_with(cx, |i, _| i.presentation().is_masked()));
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&spec("second"), w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(secret.read_with(cx, |i, _| i.value().to_string()), "second");
    assert!(secret.read_with(cx, |i, _| i.presentation().is_masked()));
}
