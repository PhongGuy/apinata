use crate::support;

use apinata_app::auth_section::{AuthMode, AuthSection};
use apinata_app::command::{Command, Edit, Target};
use apinata_app::oauth2_editor::{
    OAuth2Editor, OAuth2Field, OAuth2Toggle, ParamCell, PlacementKind,
};
use apinata_app::snapshot::DotenvNames;
use apinata_core::{
    AuthSpec, ClientAuth, OAuth2Flow, OAuth2Param, OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage,
    TokenPlacement, TokenSource,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Context, Entity, TestAppContext, Window};
use support::*;

type Host = Entity<SectionHost<AuthSection>>;

fn section(
    cx: &mut TestAppContext,
    spec: AuthSpec,
) -> (AnyWindowHandle, Host, Entity<AuthSection>) {
    let (win, host) = host_section(cx, AuthSection::new);
    let s = host.read_with(cx, |h, _| h.view.clone());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&spec, w, cx)))
        .unwrap();
    frame(cx, win);
    (win, host, s)
}

fn editor(cx: &mut TestAppContext, s: &Entity<AuthSection>) -> Entity<OAuth2Editor> {
    s.read_with(cx, |s, _| s.oauth2().clone())
}

fn on(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    e: &Entity<OAuth2Editor>,
    f: impl FnOnce(&mut OAuth2Editor, &mut Window, &mut Context<OAuth2Editor>),
) {
    cx.update_window(win, |_, w, cx| e.update(cx, |e, cx| f(e, w, cx)))
        .unwrap();
    frame(cx, win);
}

fn last(cx: &mut TestAppContext, host: &Host) -> OAuth2Spec {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Auth(AuthSpec::OAuth2(o))) => o,
        other => panic!("no oauth2 edit: {other:?}"),
    }
}

fn visible(cx: &mut TestAppContext, win: AnyWindowHandle, fields: &[OAuth2Field]) -> Vec<bool> {
    cx.update_window(win, |_, w, _| {
        fields
            .iter()
            .map(|f| w.try_find(f.id()).is_some())
            .collect()
    })
    .unwrap()
}

fn cc() -> OAuth2Spec {
    let mut o = OAuth2Spec::new(OAuth2Flow::ClientCredentials);
    o.access_token_url = "https://id.example.com/token".into();
    o.client_id = "svc".into();
    o
}

const ALL: [OAuth2Field; 10] = [
    OAuth2Field::AuthorizationUrl,
    OAuth2Field::AccessTokenUrl,
    OAuth2Field::RefreshTokenUrl,
    OAuth2Field::CallbackUrl,
    OAuth2Field::ClientId,
    OAuth2Field::ClientSecret,
    OAuth2Field::Username,
    OAuth2Field::Password,
    OAuth2Field::Scope,
    OAuth2Field::State,
];

#[gpui_kit::test]
fn choosing_oauth2_sends_the_fresh_defaults(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::None);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(AuthMode::OAuth2, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(
        last(cx, &host),
        OAuth2Spec::new(OAuth2Flow::AuthorizationCode)
    );
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("oauth2-flow").is_some())
            .unwrap()
    );
    assert!(AuthMode::ALL.contains(&AuthMode::OAuth2));
}

#[gpui_kit::test]
fn each_flow_shows_its_own_fields(cx: &mut TestAppContext) {
    let (win, _host, s) = section(cx, AuthSpec::OAuth2(cc()));
    let e = editor(cx, &s);
    let t = true;
    let f = false;
    let table = [
        (
            OAuth2Flow::ClientCredentials,
            [f, t, t, f, t, t, f, f, t, f],
            t,
            f,
        ),
        (OAuth2Flow::Password, [f, t, t, f, t, t, t, t, t, f], t, f),
        (
            OAuth2Flow::AuthorizationCode,
            [t, t, t, t, t, t, f, f, t, t],
            t,
            t,
        ),
        (OAuth2Flow::Implicit, [t, f, f, t, t, f, f, f, t, t], f, f),
    ];
    for (flow, fields, client_auth, pkce) in table {
        on(cx, win, &e, |e, w, cx| e.set_flow(flow, w, cx));
        assert_eq!(visible(cx, win, &ALL), fields, "{flow:?}");
        let shown = cx
            .update_window(win, |_, w, _| {
                [
                    "oauth2-client-auth",
                    "oauth2-pkce",
                    "oauth2-token-id",
                    "oauth2-token-source",
                    "oauth2-auto-fetch",
                    "oauth2-auto-refresh",
                ]
                .map(|id| w.try_find(id).is_some())
            })
            .unwrap();
        assert_eq!(shown, [client_auth, pkce, t, t, t, t], "{flow:?}");
    }
}

#[gpui_kit::test]
fn typing_sends_the_whole_spec_and_secrets_are_masked(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::OAuth2(cc()));
    let e = editor(cx, &s);
    type_at_end(cx, win, None, OAuth2Field::ClientId.id(), "2");
    cx.run_until_parked();
    let mut want = cc();
    want.client_id = "svc2".into();
    assert_eq!(last(cx, &host), want);
    for (field, masked) in [
        (OAuth2Field::ClientSecret, true),
        (OAuth2Field::Password, true),
        (OAuth2Field::ClientId, false),
        (OAuth2Field::Scope, false),
    ] {
        let input = e.read_with(cx, |e, _| e.input(field).clone());
        assert_eq!(
            input.read_with(cx, |i, _| i.presentation().is_masked()),
            masked,
            "{field:?}"
        );
    }
}

#[gpui_kit::test]
fn selects_and_toggles_send_edits(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::OAuth2(cc()));
    let e = editor(cx, &s);
    on(cx, win, &e, |e, w, cx| {
        e.set_client_auth(ClientAuth::BasicHeader, w, cx)
    });
    assert_eq!(last(cx, &host).client_auth, Some(ClientAuth::BasicHeader));
    on(cx, win, &e, |e, w, cx| {
        e.set_token_placement(PlacementKind::Query, w, cx)
    });
    assert_eq!(
        last(cx, &host).token_placement,
        TokenPlacement::Query {
            key: "access_token".into()
        }
    );
    type_at_end(cx, win, None, OAuth2Field::QueryKey.id(), "2");
    cx.run_until_parked();
    assert_eq!(
        last(cx, &host).token_placement,
        TokenPlacement::Query {
            key: "access_token2".into()
        }
    );
    on(cx, win, &e, |e, w, cx| {
        e.set_token_placement(PlacementKind::Header, w, cx)
    });
    assert_eq!(
        last(cx, &host).token_placement,
        TokenPlacement::Header {
            prefix: "Bearer".into()
        }
    );
    on(cx, win, &e, |e, w, cx| {
        e.set_token_source(TokenSource::IdToken, w, cx)
    });
    on(cx, win, &e, |e, w, cx| {
        e.set_toggle(OAuth2Toggle::AutoFetch, false, w, cx)
    });
    on(cx, win, &e, |e, w, cx| {
        e.set_toggle(OAuth2Toggle::AutoRefresh, true, w, cx)
    });
    on(cx, win, &e, |e, w, cx| {
        e.set_toggle(OAuth2Toggle::Pkce, true, w, cx)
    });
    let o = last(cx, &host);
    assert_eq!(
        (o.token_source, o.auto_fetch, o.auto_refresh, o.pkce),
        (TokenSource::IdToken, false, true, true)
    );
    assert_eq!(o.client_auth, Some(ClientAuth::BasicHeader));
}

#[gpui_kit::test]
fn switching_flows_keeps_other_flows_fields(cx: &mut TestAppContext) {
    let mut code = OAuth2Spec::new(OAuth2Flow::AuthorizationCode);
    code.authorization_url = "https://a".into();
    let (win, host, s) = section(cx, AuthSpec::OAuth2(code));
    let e = editor(cx, &s);
    on(cx, win, &e, |e, w, cx| {
        e.set_flow(OAuth2Flow::ClientCredentials, w, cx)
    });
    let sent = last(cx, &host);
    assert_eq!(
        (sent.flow, sent.authorization_url.as_str()),
        (OAuth2Flow::ClientCredentials, "https://a")
    );
    // The actor echoes what it wrote, which has lost the code flow's fields.
    let echo = AuthSpec::OAuth2(sent.on_disk());
    cx.update_window(win, |_, w, cx| s.update(cx, |s, cx| s.load(&echo, w, cx)))
        .unwrap();
    frame(cx, win);
    on(cx, win, &e, |e, w, cx| {
        e.set_flow(OAuth2Flow::AuthorizationCode, w, cx)
    });
    assert_eq!(last(cx, &host).authorization_url, "https://a");
}

#[gpui_kit::test]
fn unknown_strings_survive_an_unrelated_edit(cx: &mut TestAppContext) {
    let mut o = cc();
    o.client_auth = Some(ClientAuth::Other("private_key_jwt".into()));
    o.token_source = TokenSource::Other("refresh_token".into());
    o.token_placement = TokenPlacement::Unset;
    let (win, host, _s) = section(cx, AuthSpec::OAuth2(o.clone()));
    type_at_end(cx, win, None, OAuth2Field::Scope.id(), "x");
    cx.run_until_parked();
    o.scope = "x".into();
    assert_eq!(last(cx, &host), o);
}

const SPARSE: &str = "info:\n  name: a\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://id/token\n";

#[gpui_kit::test]
fn an_untouched_oauth2_block_keeps_its_bytes(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", SPARSE)]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(
            (
                "section",
                apinata_app::request_editor::Section::Auth as usize,
            ),
            cx,
        );
        w.render_frame(cx)
    })
    .unwrap();
    cx.run_until_parked();
    ws.update(cx, |w, cx| w.send(&root, edit_url("a.yml", "http://y"), cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Request(ip("a.yml"))), cx)
    });
    cx.run_until_parked();
    let saved = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert_eq!(saved, SPARSE.replace("http://x", "http://y"));
}

#[gpui_kit::test]
fn an_edit_in_the_tab_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (
        write_tree(&[("opencollection.yml", ROOT), ("a.yml", SPARSE)]),
        write_tree(&[("opencollection.yml", ROOT), ("a.yml", SPARSE)]),
    );
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(
            (
                "section",
                apinata_app::request_editor::Section::Auth as usize,
            ),
            cx,
        );
        w.render_frame(cx)
    })
    .unwrap();
    type_at_end(cx, win, Some("auth"), OAuth2Field::ClientId.id(), "svc");
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Request(ip("a.yml"))), cx)
    });
    cx.run_until_parked();
    let path = twin.path().join("a.yml");
    let mut r = apinata_core::RequestFile::load(&path).unwrap();
    let AuthSpec::OAuth2(mut o) = r.auth() else {
        panic!("oauth2")
    };
    o.client_id = "svc".into();
    r.set_auth(AuthSpec::OAuth2(o));
    r.save().unwrap();
    assert_eq!(
        std::fs::read_to_string(col.path().join("a.yml")).unwrap(),
        std::fs::read_to_string(&path).unwrap()
    );
}

#[gpui_kit::test]
fn typing_a_prefix_while_unset_shows_the_header_placement(cx: &mut TestAppContext) {
    let mut o = cc();
    o.token_placement = TokenPlacement::Unset;
    let (win, host, s) = section(cx, AuthSpec::OAuth2(o));
    let e = editor(cx, &s);
    type_at_end(cx, win, None, OAuth2Field::HeaderPrefix.id(), "Token");
    cx.run_until_parked();
    assert!(matches!(
        last(cx, &host).token_placement,
        TokenPlacement::Header { .. }
    ));
    let select = e.read_with(cx, |e, _| e.placement_choice().clone());
    let shown = select.read_with(cx, |s, _| s.selected_value().cloned());
    assert_eq!(shown.as_deref(), Some("Header"));
}

fn shown_rows(cx: &mut TestAppContext, win: AnyWindowHandle, n: usize) -> Vec<bool> {
    cx.update_window(win, |_, w, _| {
        (0..n)
            .map(|ix| w.try_find(("oauth2-param-name", ix)).is_some())
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn rows_show_only_the_flows_stages(cx: &mut TestAppContext) {
    let mut o = OAuth2Spec::new(OAuth2Flow::AuthorizationCode);
    o.params = vec![
        OAuth2Param::new(
            OAuth2Stage::Authorization,
            "prompt",
            "consent",
            OAuth2ParamPlacement::Query,
        ),
        OAuth2Param::new(
            OAuth2Stage::Token,
            "audience",
            "api",
            OAuth2ParamPlacement::Body,
        ),
        OAuth2Param::new(
            OAuth2Stage::Refresh,
            "X-R",
            "1",
            OAuth2ParamPlacement::Header,
        ),
    ];
    let (win, host, s) = section(cx, AuthSpec::OAuth2(o.clone()));
    let e = editor(cx, &s);
    assert_eq!(shown_rows(cx, win, 3), [true, true, true]);
    on(cx, win, &e, |e, w, cx| {
        e.set_flow(OAuth2Flow::ClientCredentials, w, cx)
    });
    assert_eq!(shown_rows(cx, win, 3), [false, true, true]);
    assert_eq!(
        last(cx, &host).params,
        o.params,
        "hidden rows stay in the spec"
    );
    on(cx, win, &e, |e, w, cx| {
        e.set_flow(OAuth2Flow::Implicit, w, cx)
    });
    assert_eq!(shown_rows(cx, win, 3), [true, false, false]);
}

#[gpui_kit::test]
fn a_row_is_added_edited_and_removed(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::OAuth2(cc()));
    let e = editor(cx, &s);
    on(cx, win, &e, |e, w, cx| e.add_param(w, cx));
    assert_eq!(
        last(cx, &host).params,
        [OAuth2Param::new(
            OAuth2Stage::Token,
            "",
            "",
            OAuth2ParamPlacement::Body
        )]
    );
    type_at_end(cx, win, None, ("oauth2-param-name", 0usize), "audience");
    cx.run_until_parked();
    type_at_end(cx, win, None, ("oauth2-param-value", 0usize), "api");
    cx.run_until_parked();
    on(cx, win, &e, |e, w, cx| {
        e.set_param_stage(0, OAuth2Stage::Refresh, w, cx)
    });
    on(cx, win, &e, |e, w, cx| {
        e.set_param_placement(0, OAuth2ParamPlacement::Header, w, cx)
    });
    assert_eq!(
        last(cx, &host).params,
        [OAuth2Param::new(
            OAuth2Stage::Refresh,
            "audience",
            "api",
            OAuth2ParamPlacement::Header
        )]
    );
    let value = e.read_with(cx, |e, _| e.param_input(0, ParamCell::Value).clone());
    assert_eq!(value.read_with(cx, |i, _| i.value().to_string()), "api");
    on(cx, win, &e, |e, w, cx| e.remove_param(0, w, cx));
    assert!(last(cx, &host).params.is_empty());
}

#[gpui_kit::test]
fn the_code_flow_adds_authorization_rows_as_query_parameters(cx: &mut TestAppContext) {
    let (win, host, s) = section(
        cx,
        AuthSpec::OAuth2(OAuth2Spec::new(OAuth2Flow::AuthorizationCode)),
    );
    let e = editor(cx, &s);
    on(cx, win, &e, |e, w, cx| e.add_param(w, cx));
    assert_eq!(
        last(cx, &host).params,
        [OAuth2Param::new(
            OAuth2Stage::Authorization,
            "",
            "",
            OAuth2ParamPlacement::Query
        )]
    );
    assert!(
        cx.update_window(win, |_, w, _| w.try_find("oauth2-param-add").is_some())
            .unwrap()
    );
}

#[gpui_kit::test]
fn a_parameter_added_in_the_tab_saves_what_core_saves(cx: &mut TestAppContext) {
    let (col, twin) = (
        write_tree(&[("opencollection.yml", ROOT), ("a.yml", SPARSE)]),
        write_tree(&[("opencollection.yml", ROOT), ("a.yml", SPARSE)]),
    );
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(
            (
                "section",
                apinata_app::request_editor::Section::Auth as usize,
            ),
            cx,
        );
        w.render_frame(cx)
    })
    .unwrap();
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    let e = editor(cx, &auth);
    on(cx, win, &e, |e, w, cx| e.add_param(w, cx));
    cx.run_until_parked();
    // The table sits below the fold of the section body; scroll it into view.
    cx.update_window(win, |_, w, cx| {
        let down =
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-1000.)));
        w.scroll("section-body", down, cx)
    })
    .unwrap();
    frame(cx, win);
    type_at_end(
        cx,
        win,
        Some("auth"),
        ("oauth2-param-name", 0usize),
        "audience",
    );
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Save(Target::Request(ip("a.yml"))), cx)
    });
    cx.run_until_parked();
    let path = twin.path().join("a.yml");
    let mut r = apinata_core::RequestFile::load(&path).unwrap();
    let AuthSpec::OAuth2(mut o) = r.auth() else {
        panic!("oauth2")
    };
    o.params = vec![OAuth2Param::new(
        OAuth2Stage::Token,
        "audience",
        "",
        OAuth2ParamPlacement::Body,
    )];
    r.set_auth(AuthSpec::OAuth2(o));
    r.save().unwrap();
    assert_eq!(
        std::fs::read_to_string(col.path().join("a.yml")).unwrap(),
        std::fs::read_to_string(&path).unwrap()
    );
}

#[gpui_kit::test]
fn a_parameter_row_matches_the_other_fields_height(cx: &mut TestAppContext) {
    let (win, _host, s) = section(cx, AuthSpec::OAuth2(cc()));
    let e = editor(cx, &s);
    on(cx, win, &e, |e, w, cx| e.add_param(w, cx));
    let (name, client) = cx
        .update_window(win, |_, w, _| {
            (
                w.find(("oauth2-param-name", 0usize)).bounds().size.height,
                w.find(OAuth2Field::ClientId.id()).bounds().size.height,
            )
        })
        .unwrap();
    assert_eq!(name, client);
}

#[gpui_kit::test]
fn the_callback_placeholder_names_the_registered_redirect(cx: &mut TestAppContext) {
    let (_win, _host, s) = section(
        cx,
        AuthSpec::OAuth2(OAuth2Spec::new(OAuth2Flow::AuthorizationCode)),
    );
    let e = editor(cx, &s);
    let input = e.read_with(cx, |e, _| e.input(OAuth2Field::CallbackUrl).clone());
    assert_eq!(
        input.read_with(cx, |i, _| i.presentation().placeholder().to_string()),
        "Redirect URL registered with the provider"
    );
}

fn edit_count(cx: &mut TestAppContext, host: &Host) -> usize {
    host.read_with(cx, |h, _| h.edits.len())
}

#[gpui_kit::test]
fn the_client_secret_emits_once_on_blur(cx: &mut TestAppContext) {
    let (win, host, _s) = section(cx, AuthSpec::OAuth2(cc()));
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    let before = edit_count(cx, &host);
    type_at_end(cx, win, None, OAuth2Field::ClientSecret.id(), "abc");
    frame(cx, win);
    assert_eq!(edit_count(cx, &host), before, "no edit while typing");
    cx.update_window(win, |_, w, cx| w.click(OAuth2Field::ClientId.id(), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(edit_count(cx, &host), before + 1);
    assert_eq!(last(cx, &host).client_secret, "abc");
}

// Scrolls the section body just far enough to bring `id` into view below the token panel.
fn reveal(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        let body = w.find("section-body").bounds();
        let below = w.find(id).bounds().bottom() - body.bottom();
        if below > gpui_kit::px(0.) {
            let up = gpui_kit::point(gpui_kit::px(0.), -below - gpui_kit::px(8.));
            w.scroll("section-body", gpui_kit::ScrollDelta::Pixels(up), cx);
        }
    })
    .unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn a_moved_secret_shows_the_badge_and_writes_nothing(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x")),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.activate_window();
        w.click(
            (
                "section",
                apinata_app::request_editor::Section::Auth as usize,
            ),
            cx,
        );
        w.render_frame(cx)
    })
    .unwrap();
    let auth = ed.read_with(cx, |e, _| e.auth().clone());
    cx.update_window(win, |_, w, cx| {
        auth.update(cx, |s, cx| s.set_mode(AuthMode::OAuth2, w, cx))
    })
    .unwrap();
    frame(cx, win);
    cx.run_until_parked();
    reveal(cx, win, OAuth2Field::ClientSecret.id());
    type_at_end(
        cx,
        win,
        Some("auth"),
        OAuth2Field::ClientSecret.id(),
        "s3cret-value",
    );
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.within("auth").click(OAuth2Field::ClientId.id(), cx)
    })
    .unwrap();
    frame(cx, win);
    cx.run_until_parked();
    frame(cx, win);
    let e = editor(cx, &auth);
    assert_eq!(e.read_with(cx, |e, _| e.notice().map(str::to_owned)), None);
    assert!(!col.path().join(".env").exists() && !col.path().join(".gitignore").exists());
    // The field now shows where the secret goes, never the reference text.
    assert_eq!(
        badge_text(cx, win).as_deref(),
        Some("Saved to .env when you save")
    );
    assert!(!present(cx, win, OAuth2Field::ClientSecret.id()));
}

const PLAIN_FILE: &str = "info:\n  name: a\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://id/token\n    credentials:\n      clientId: svc\n      clientSecret: old-plain\n";

// Opens `a.yml` of `col` in an editor window on its Auth section; the first item is the
// session dir, kept alive for the test.
fn auth_tab(
    cx: &mut TestAppContext,
    col: &tempfile::TempDir,
) -> (
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<apinata_app::workspace::Workspace>,
    std::path::PathBuf,
    Entity<apinata_app::request_editor::RequestEditor>,
) {
    let (state, ws, root) = open_dir(cx, col);
    let (win, ed) = editor_for(cx, &ws, &root, "a.yml", false);
    cx.update_window(win, |_, w, cx| {
        w.click(
            (
                "section",
                apinata_app::request_editor::Section::Auth as usize,
            ),
            cx,
        );
        w.render_frame(cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    (state, win, ws, root, ed)
}

#[gpui_kit::test]
fn a_plain_secret_shows_the_move_notice(cx: &mut TestAppContext) {
    let mut plain = cc();
    plain.client_secret = "old-plain".into();
    let (win, _host, _s) = section(cx, AuthSpec::OAuth2(plain));
    assert!(present(cx, win, "oauth2-plain-notice"));
    assert_eq!(
        apinata_app::oauth2_editor::PLAIN_NOTICE,
        "Stored in plain text"
    );
    for secret in ["{{process.env.X}}", ""] {
        let mut o = cc();
        o.client_secret = secret.into();
        let (win, _host, _s) = section(cx, AuthSpec::OAuth2(o));
        assert!(!present(cx, win, "oauth2-plain-notice"), "{secret:?}");
    }

    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", PLAIN_FILE)]);
    let (_state, win, ws, root, _ed) = auth_tab(cx, &col);
    assert!(present(cx, win, "oauth2-plain-notice"));
    reveal(cx, win, "oauth2-move-secrets");
    cx.update_window(win, |_, w, cx| w.click("oauth2-move-secrets", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    assert_eq!(
        std::fs::read_to_string(col.path().join("a.yml")).unwrap(),
        PLAIN_FILE
    );
    let AuthSpec::OAuth2(o) = ws_snap(cx, &ws, &root).content.requests[&ip("a.yml")]
        .auth
        .clone()
    else {
        panic!("oauth2")
    };
    assert_eq!(
        o.client_secret,
        "{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}"
    );
    assert!(!present(cx, win, "oauth2-plain-notice"));
}

#[gpui_kit::test]
fn the_url_bar_stays_while_oauth_scrolls(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", PLAIN_FILE)]);
    let (_state, win, _ws, _root, _ed) = auth_tab(cx, &col);
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(600.)));
    frame(cx, win);
    let bounds = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, _| {
            (w.find("url").bounds(), w.find("section-strip").bounds())
        })
        .unwrap()
    };
    let before = bounds(cx);
    cx.update_window(win, |_, w, cx| {
        let down =
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-5000.)));
        w.scroll("section-body", down, cx)
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(bounds(cx), before, "the URL row and section bar stay");
    cx.update_window(win, |_, w, _| {
        let body = w.find("section-body").bounds();
        let add = w.find("oauth2-param-add").bounds();
        let height = w.viewport_size().height;
        assert!(
            add.top() >= body.top() && add.bottom() <= body.bottom().min(height),
            "add {add:?} outside body {body:?}"
        );
        assert!(
            add.size.width < body.size.width / 2.,
            "add {add:?} body {body:?}"
        );
        assert!(
            (add.left() - body.left()).abs()
                <= apinata_app::style::at_zoom(apinata_app::style::PANE_INSET, 1.0),
            "add {add:?} body {body:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn saving_with_focus_in_the_secret_moves_it(cx: &mut TestAppContext) {
    let file = PLAIN_FILE.replace("      clientSecret: old-plain\n", "");
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, win, _ws, _root, _ed) = auth_tab(cx, &col);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    type_at_end(
        cx,
        win,
        Some("auth"),
        OAuth2Field::ClientSecret.id(),
        "typed-s3cret",
    );
    // No blur: the save action goes out while the secret field still has focus.
    cx.update_window(win, |_, w, cx| {
        w.dispatch_action(Box::new(apinata_app::request_editor::SaveRequest), cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    let saved = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(
        saved.contains("clientSecret: \"{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}\"")
            && !saved.contains("typed-s3cret"),
        "{saved}"
    );
    let env = std::fs::read_to_string(col.path().join(".env")).unwrap_or_default();
    assert!(
        env.contains("APINATA_OAUTH2_CLIENT_SECRET='typed-s3cret'"),
        "{env}"
    );
}

// Types a client secret in an OAuth 2.0 request tab of a tab area and, without blurring,
// dispatches `action`; `then` answers whatever the action asks.
fn tab_area_secret_survives(
    cx: &mut TestAppContext,
    action: impl gpui_kit::Action,
    then: impl FnOnce(&mut TestAppContext),
) {
    let file = PLAIN_FILE.replace("      clientSecret: old-plain\n", "");
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &file)]);
    let (_state, ws, root) = open_dir(cx, &col);
    support::init(cx);
    cx.update(apinata_app::keymap::init);
    let shared = ws.clone();
    let (win, _area) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::tab_area::TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    open_tab(cx, &ws, &root, "a.yml");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.activate_window();
        w.click(
            (
                "section",
                apinata_app::request_editor::Section::Auth as usize,
            ),
            cx,
        );
        w.render_frame(cx)
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    type_at_end(
        cx,
        win,
        Some("auth"),
        OAuth2Field::ClientSecret.id(),
        "typed-s3cret",
    );
    cx.dispatch_action(win, action);
    cx.run_until_parked();
    then(cx);
    cx.run_until_parked();
    let saved = std::fs::read_to_string(col.path().join("a.yml")).unwrap();
    assert!(
        saved.contains("clientSecret: \"{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}\"")
            && !saved.contains("typed-s3cret"),
        "{saved}"
    );
    let env = std::fs::read_to_string(col.path().join(".env")).unwrap_or_default();
    assert!(
        env.contains("APINATA_OAUTH2_CLIENT_SECRET='typed-s3cret'"),
        "{env}"
    );
}

#[gpui_kit::test]
fn save_all_with_focus_in_the_secret_moves_it(cx: &mut TestAppContext) {
    tab_area_secret_survives(cx, apinata_app::tab_area::SaveAll, |_| {});
}

#[gpui_kit::test]
fn closing_with_focus_in_the_secret_offers_to_save_it(cx: &mut TestAppContext) {
    tab_area_secret_survives(cx, apinata_app::tab_area::CloseTab, |cx| {
        answer_confirm(cx, "confirm-save")
    });
}

#[gpui_kit::test]
fn a_template_secret_shows_and_a_literal_stays_masked(cx: &mut TestAppContext) {
    let mut spec = cc();
    spec.client_secret = "{{client_secret}}".into();
    spec.password = "hunter2".into();
    let (_win, _host, s) = section(cx, AuthSpec::OAuth2(spec));
    let e = editor(cx, &s);
    for (field, masked) in [
        (OAuth2Field::ClientSecret, false),
        (OAuth2Field::Password, true),
    ] {
        let input = e.read_with(cx, |e, _| e.input(field).clone());
        assert_eq!(
            input.read_with(cx, |i, _| i.presentation().is_masked()),
            masked,
            "{field:?}"
        );
    }
}

const PARAM_FILE: &str = "info:\n  name: a\n  type: http\n  seq: 1\n\nhttp:\n  method: GET\n  url: http://x\n  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: http://id/token\n    credentials:\n      clientId: svc\n    additionalParameters:\n      accessTokenRequest:\n        - name: audience\n          value: api\n          placement: body\n";

// Side by side at 200% the request pane is narrower than a parameter row's fixed selects
// plus the mode selects; every control must stay inside the section body, and the name
// and value cells keep room to type in.
// Opens `file` on its Auth section in a 1280x800 window at 200% zoom with the
// response beside the request, the narrowest pane the layout offers.
fn narrow_auth_pane(cx: &mut TestAppContext, file: &str) -> (Started, tempfile::TempDir) {
    use apinata_app::app::AppState;
    use apinata_app::request_editor::Section;
    use apinata_app::settings::Orientation;
    use gpui_kit::px;
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", file)]);
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    // Pick Auth at the default size, while the whole section strip is in view.
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Auth as usize), cx)
    })
    .unwrap();
    frame(cx, s.win);
    gpui_kit::VisualTestContext::from_window(s.win, cx)
        .simulate_resize(gpui_kit::size(px(1280.), px(800.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |s| {
                s.response_orientation = Orientation::Horizontal;
                s.zoom = 2.0;
            })
        });
    });
    cx.run_until_parked();
    frame(cx, s.win);
    // Form rows decide to stack on the frame after they measure.
    frame(cx, s.win);
    (s, col)
}

#[gpui_kit::test]
fn oauth_rows_fit_the_request_pane_side_by_side_at_200_percent(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (s, _col) = narrow_auth_pane(cx, PARAM_FILE);
    cx.update_window(s.win, |_, w, _| {
        let body = w.find("section-body").bounds();
        let inside = |b: gpui_kit::Bounds<gpui_kit::Pixels>| {
            b.left() >= body.left() - px(0.5) && b.right() <= body.right() + px(0.5)
        };
        for id in ["auth-mode", "oauth2-flow", "oauth2-access-token-url"] {
            let b = w.find(id).bounds();
            assert!(inside(b), "{id} {b:?} leaves the section body {body:?}");
        }
        for id in [
            "oauth2-param-stage",
            "oauth2-param-name",
            "oauth2-param-value",
            "oauth2-param-placement",
            "oauth2-param-remove",
        ] {
            let b = w.find((id, 0usize)).bounds();
            assert!(inside(b), "{id} {b:?} leaves the section body {body:?}");
        }
        for id in ["oauth2-param-name", "oauth2-param-value"] {
            let b = w.find((id, 0usize)).bounds();
            assert!(
                b.size.width >= px(100.),
                "{id} too narrow to type in: {b:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn plain_secret_notice_fits_the_request_pane_side_by_side_at_200_percent(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (s, _col) = narrow_auth_pane(cx, PLAIN_FILE);
    cx.update_window(s.win, |_, w, _| {
        let body = w.find("section-body").bounds();
        for id in ["oauth2-plain-notice", "oauth2-move-secrets"] {
            let b = w.find(id).bounds();
            assert!(
                b.left() >= body.left() - px(0.5) && b.right() <= body.right() + px(0.5),
                "{id} {b:?} leaves the section body {body:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn oauth2_fields_and_choices_sit_in_form_rows(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, AuthSpec::OAuth2(cc()));
    cx.update_window(win, |_, w, _| {
        for (row, control) in [
            ("oauth2-flow-row", "oauth2-flow"),
            ("oauth2-access-token-url-row", "oauth2-access-token-url"),
            ("oauth2-token-placement-row", "oauth2-token-placement"),
        ] {
            assert!(w.within(row).try_find("form-label").is_some(), "{row}");
            assert!(w.within(row).try_find(control).is_some(), "{row}");
        }
    })
    .unwrap();
}

// The PKCE box sits in the field column, under the selects and inputs, not in the label column.
#[gpui_kit::test]
fn the_pkce_box_sits_in_the_field_column(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (win, _host, s) = section(cx, AuthSpec::OAuth2(cc()));
    let e = editor(cx, &s);
    on(cx, win, &e, |e, w, cx| {
        e.set_flow(OAuth2Flow::AuthorizationCode, w, cx)
    });
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let field = w
            .within("oauth2-client-auth-row")
            .find("form-control")
            .bounds();
        let control = w.within("oauth2-pkce-row").find("form-control").bounds();
        let pkce = w.find("oauth2-pkce").bounds();
        assert_eq!(control.left(), field.left());
        assert!(
            pkce.left() >= field.left() - px(0.5),
            "{pkce:?} vs {field:?}"
        );
    })
    .unwrap();
}

// The auto-fetch and auto-refresh boxes share the PKCE box's field column.
#[gpui_kit::test]
fn the_auto_token_boxes_sit_in_the_field_column(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (win, _host, _s) = section(cx, AuthSpec::OAuth2(cc()));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let field = w
            .within("oauth2-client-auth-row")
            .find("form-control")
            .bounds();
        for (row, id) in [
            ("oauth2-auto-fetch-row", "oauth2-auto-fetch"),
            ("oauth2-auto-refresh-row", "oauth2-auto-refresh"),
        ] {
            let control = w.within(row).find("form-control").bounds();
            let b = w.find(id).bounds();
            assert_eq!(control.left(), field.left(), "{row}");
            assert!(
                b.left() >= field.left() - px(0.5),
                "{id} {b:?} vs {field:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn mixed_text_around_a_reference_stays_masked(cx: &mut TestAppContext) {
    let mut spec = cc();
    spec.client_secret = "lit-{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}".into();
    spec.password = "  {{pw}}  ".into();
    let (_win, _host, s) = section(cx, AuthSpec::OAuth2(spec));
    let e = editor(cx, &s);
    for (field, masked) in [
        (OAuth2Field::ClientSecret, true),
        (OAuth2Field::Password, false),
    ] {
        let input = e.read_with(cx, |e, _| e.input(field).clone());
        assert_eq!(
            input.read_with(cx, |i, _| i.presentation().is_masked()),
            masked,
            "{field:?}"
        );
    }
}

#[gpui_kit::test]
fn the_token_rows_share_the_field_column(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, AuthSpec::OAuth2(cc()));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let field = w
            .within("oauth2-client-auth-row")
            .find("form-control")
            .bounds();
        for row in [
            "oauth2-token-id-row",
            "oauth2-token-placement-row",
            "oauth2-token-source-row",
        ] {
            let control = w.within(row).find("form-control").bounds();
            assert_eq!(control.left(), field.left(), "{row}");
        }
    })
    .unwrap();
}

// Tells the section which `.env` names are stored and which its draft holds.
fn names(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    s: &Entity<AuthSection>,
    stored: &[&str],
    held: &[&str],
) {
    let names = DotenvNames {
        stored: std::sync::Arc::new(stored.iter().map(|n| n.to_string()).collect()),
        held: held.iter().map(|n| n.to_string()).collect(),
    };
    cx.update_window(win, |_, _, cx| {
        s.update(cx, |s, cx| s.set_dotenv(names, cx))
    })
    .unwrap();
    frame(cx, win);
}

fn badge_text(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let found = w.try_find("oauth2-client-secret-stored")?;
        found.label().map(str::to_owned)
    })
    .unwrap()
}

const SECRET_NAME: &str = "APINATA_OAUTH2_CLIENT_SECRET";

fn stored_secret() -> OAuth2Spec {
    let mut spec = cc();
    spec.client_secret = "{{process.env.APINATA_OAUTH2_CLIENT_SECRET}}".into();
    spec
}

#[gpui_kit::test]
fn a_dotenv_secret_shows_a_badge_instead_of_its_reference(cx: &mut TestAppContext) {
    let (win, _host, s) = section(cx, AuthSpec::OAuth2(stored_secret()));
    // A reference to a name neither `.env` has nor the draft holds is the user's own text.
    assert!(!present(cx, win, "oauth2-client-secret-stored"));
    assert!(present(cx, win, OAuth2Field::ClientSecret.id()));
    names(cx, win, &s, &[], &[SECRET_NAME]);
    assert_eq!(
        badge_text(cx, win).as_deref(),
        Some("Saved to .env when you save")
    );
    names(cx, win, &s, &[SECRET_NAME], &[]);
    assert_eq!(badge_text(cx, win).as_deref(), Some("Stored in .env"));
    assert!(present(cx, win, "oauth2-client-secret-replace"));
    assert!(!present(cx, win, OAuth2Field::ClientSecret.id()));
    // A reference that is no `.env` key is the user's own template and stays visible.
    let mut odd = cc();
    odd.client_secret = "{{process.env.my-key}}".into();
    let (win, _host, s) = section(cx, AuthSpec::OAuth2(odd));
    names(cx, win, &s, &["my-key"], &[]);
    assert!(!present(cx, win, "oauth2-client-secret-stored"));
    assert!(present(cx, win, OAuth2Field::ClientSecret.id()));
}

#[gpui_kit::test]
fn replace_takes_a_new_value_in_an_empty_masked_field(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::OAuth2(stored_secret()));
    names(cx, win, &s, &[SECRET_NAME], &[]);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    let before = edit_count(cx, &host);
    cx.update_window(win, |_, w, cx| w.click("oauth2-client-secret-replace", cx))
        .unwrap();
    frame(cx, win);
    assert!(!present(cx, win, "oauth2-client-secret-stored"));
    let input = editor(cx, &s).read_with(cx, |e, _| e.input(OAuth2Field::ClientSecret).clone());
    assert_eq!(input.read_with(cx, |i, _| i.value().to_string()), "");
    cx.update_window(win, |_, w, cx| w.input("my-client", cx))
        .unwrap();
    frame(cx, win);
    assert!(input.read_with(cx, |i, _| i.presentation().is_masked()));
    assert_eq!(
        input.read_with(cx, |i, _| i.value().to_string()),
        "my-client"
    );
    cx.update_window(win, |_, w, cx| w.click(OAuth2Field::ClientId.id(), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(edit_count(cx, &host), before + 1);
    assert_eq!(last(cx, &host).client_secret, "my-client");
}

#[gpui_kit::test]
fn replacing_with_nothing_keeps_the_stored_secret(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::OAuth2(stored_secret()));
    names(cx, win, &s, &[SECRET_NAME], &[]);
    cx.update_window(win, |_, w, _| w.activate_window())
        .unwrap();
    frame(cx, win);
    let before = edit_count(cx, &host);
    cx.update_window(win, |_, w, cx| w.click("oauth2-client-secret-replace", cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(OAuth2Field::ClientId.id(), cx))
        .unwrap();
    frame(cx, win);
    assert_eq!(edit_count(cx, &host), before);
    assert!(present(cx, win, "oauth2-client-secret-stored"));
}
