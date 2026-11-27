use crate::support;

use apinata_app::auth_section::{AuthMode, AuthSection};
use apinata_app::command::Edit;
use apinata_app::jwt_editor::{JwtEditor, JwtField};
use apinata_core::{
    AuthSpec, JwtAlgorithm, JwtPlacement, JwtSpec, OAuth1KeyKind, OAuth1PrivateKey,
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

fn editor(cx: &mut TestAppContext, s: &Entity<AuthSection>) -> Entity<JwtEditor> {
    s.read_with(cx, |s, _| s.jwt_editor().clone())
}

fn on(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    e: &Entity<JwtEditor>,
    f: impl FnOnce(&mut JwtEditor, &mut Window, &mut Context<JwtEditor>),
) {
    cx.update_window(win, |_, w, cx| e.update(cx, |e, cx| f(e, w, cx)))
        .unwrap();
    frame(cx, win);
}

fn last(cx: &mut TestAppContext, host: &Host) -> JwtSpec {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Auth(AuthSpec::Jwt(j))) => j,
        other => panic!("no jwt edit: {other:?}"),
    }
}

fn shown(cx: &mut TestAppContext, win: AnyWindowHandle, ids: &[&'static str]) -> Vec<bool> {
    cx.update_window(win, |_, w, _| {
        ids.iter().map(|id| w.try_find(*id).is_some()).collect()
    })
    .unwrap()
}

fn masked(cx: &mut TestAppContext, e: &Entity<JwtEditor>, f: JwtField) -> bool {
    let input = e.read_with(cx, |e, _| e.input(f).clone());
    input.read_with(cx, |i, _| i.presentation().is_masked())
}

#[gpui_kit::test]
fn choosing_jwt_sends_the_new_auth_defaults(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::None);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(AuthMode::Jwt, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(last(cx, &host), JwtSpec::default());
    assert_eq!(AuthMode::Jwt.label(), "JWT Bearer");
    assert_eq!(
        AuthMode::of(&AuthSpec::Jwt(JwtSpec::default())),
        Some(AuthMode::Jwt)
    );
}

#[gpui_kit::test]
fn hs_shows_the_secret_and_the_other_families_the_key(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::Jwt(JwtSpec::default()));
    let e = editor(cx, &s);
    let ids = [
        "jwt-algorithm",
        "jwt-secret",
        "jwt-secret-base64",
        "jwt-private-key",
        "jwt-key-kind",
        "jwt-headers",
        "jwt-payload",
        "jwt-expires-in",
        "jwt-placement",
        "jwt-header-prefix",
        "jwt-query-param-key",
    ];
    assert_eq!(
        shown(cx, win, &ids),
        [
            true, true, true, false, false, true, true, true, true, true, false
        ]
    );
    assert!(masked(cx, &e, JwtField::Secret));
    assert!(masked(cx, &e, JwtField::PrivateKey));
    assert!(!masked(cx, &e, JwtField::Payload));
    on(cx, win, &e, |e, w, cx| {
        e.set_algorithm(JwtAlgorithm::Es256, w, cx)
    });
    assert_eq!(last(cx, &host).algorithm, JwtAlgorithm::Es256);
    assert_eq!(
        shown(cx, win, &["jwt-secret", "jwt-private-key", "jwt-key-kind"]),
        [false, true, true]
    );
    on(cx, win, &e, |e, w, cx| {
        e.set_placement(JwtPlacement::Query, w, cx)
    });
    assert_eq!(last(cx, &host).placement, JwtPlacement::Query);
    assert_eq!(
        shown(cx, win, &["jwt-header-prefix", "jwt-query-param-key"]),
        [false, true]
    );
}

#[gpui_kit::test]
fn typing_the_checkbox_and_the_key_kind_send_the_whole_spec(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::Jwt(JwtSpec::default()));
    let e = editor(cx, &s);
    type_at_end(cx, win, None, JwtField::Secret.id(), "s3");
    type_at_end(cx, win, None, JwtField::Payload.id(), "{}");
    type_at_end(cx, win, None, JwtField::ExpiresIn.id(), "60");
    cx.run_until_parked();
    on(cx, win, &e, |e, w, cx| e.set_secret_base64(true, w, cx));
    let mut want = JwtSpec::default();
    (want.secret, want.payload, want.expires_in) = ("s3".into(), "{}".into(), "60".into());
    want.secret_base64 = true;
    assert_eq!(last(cx, &host), want);

    on(cx, win, &e, |e, w, cx| {
        e.set_algorithm(JwtAlgorithm::Rs256, w, cx)
    });
    type_at_end(cx, win, None, JwtField::PrivateKey.id(), "keys/k.pem");
    cx.run_until_parked();
    on(cx, win, &e, |e, w, cx| {
        e.set_key_kind(OAuth1KeyKind::File, w, cx)
    });
    assert_eq!(
        last(cx, &host).private_key,
        Some(OAuth1PrivateKey::new(OAuth1KeyKind::File, "keys/k.pem"))
    );
}

#[gpui_kit::test]
fn an_absent_prefix_and_unknown_names_survive_an_unrelated_edit(cx: &mut TestAppContext) {
    let mut j = JwtSpec::default();
    j.header_prefix = None;
    j.algorithm = JwtAlgorithm::Other("XS1".into());
    j.placement = JwtPlacement::Other("cookie".into());
    let (win, host, _s) = section(cx, AuthSpec::Jwt(j.clone()));
    type_at_end(cx, win, None, JwtField::Headers.id(), "{}");
    cx.run_until_parked();
    j.headers = "{}".into();
    assert_eq!(last(cx, &host), j);
}

#[gpui_kit::test]
fn jwt_fields_and_choices_sit_in_form_rows(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, AuthSpec::Jwt(JwtSpec::default()));
    cx.update_window(win, |_, w, _| {
        for (row, control) in [
            ("jwt-algorithm-row", "jwt-algorithm"),
            ("jwt-payload-row", "jwt-payload"),
            ("jwt-placement-row", "jwt-placement"),
        ] {
            assert!(w.within(row).try_find("form-label").is_some(), "{row}");
            assert!(w.within(row).try_find(control).is_some(), "{row}");
        }
    })
    .unwrap();
}
