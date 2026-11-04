use crate::support;

use apinata_app::auth_section::{AuthMode, AuthSection};
use apinata_app::command::Edit;
use apinata_app::oauth1_editor::{OAuth1Editor, OAuth1Field};
use apinata_core::{
    AuthSpec, OAuth1KeyKind, OAuth1Placement, OAuth1PrivateKey, OAuth1SignatureMethod, OAuth1Spec,
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

fn editor(cx: &mut TestAppContext, s: &Entity<AuthSection>) -> Entity<OAuth1Editor> {
    s.read_with(cx, |s, _| s.oauth1().clone())
}

fn on(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    e: &Entity<OAuth1Editor>,
    f: impl FnOnce(&mut OAuth1Editor, &mut Window, &mut Context<OAuth1Editor>),
) {
    cx.update_window(win, |_, w, cx| e.update(cx, |e, cx| f(e, w, cx)))
        .unwrap();
    frame(cx, win);
}

fn last(cx: &mut TestAppContext, host: &Host) -> OAuth1Spec {
    match host.read_with(cx, |h, _| h.edits.last().cloned()) {
        Some(Edit::Auth(AuthSpec::OAuth1(o))) => o,
        other => panic!("no oauth1 edit: {other:?}"),
    }
}

fn shown(cx: &mut TestAppContext, win: AnyWindowHandle, ids: &[&'static str]) -> Vec<bool> {
    cx.update_window(win, |_, w, _| {
        ids.iter().map(|id| w.try_find(*id).is_some()).collect()
    })
    .unwrap()
}

fn masked(cx: &mut TestAppContext, e: &Entity<OAuth1Editor>, f: OAuth1Field) -> bool {
    let input = e.read_with(cx, |e, _| e.input(f).clone());
    input.read_with(cx, |i, _| i.presentation().is_masked())
}

fn hmac() -> OAuth1Spec {
    let mut o = OAuth1Spec::default();
    (o.consumer_key, o.consumer_secret) = (Some("ck".into()), Some("cs".into()));
    o
}

#[gpui_kit::test]
fn choosing_oauth1_sends_the_new_auth_defaults(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::None);
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.set_mode(AuthMode::OAuth1, w, cx))
    })
    .unwrap();
    frame(cx, win);
    assert_eq!(last(cx, &host), OAuth1Spec::default());
    let labels: Vec<&str> = AuthMode::ALL.iter().map(|m| m.label()).collect();
    assert_eq!(labels[labels.len() - 2..], ["OAuth 1.0", "OAuth 2.0"]);
}

#[gpui_kit::test]
fn every_field_shows_and_secrets_are_masked(cx: &mut TestAppContext) {
    let (win, _host, s) = section(cx, AuthSpec::OAuth1(hmac()));
    let e = editor(cx, &s);
    let ids = [
        "oauth1-consumer-key",
        "oauth1-consumer-secret",
        "oauth1-access-token",
        "oauth1-token-secret",
        "oauth1-signature-method",
        "oauth1-private-key",
        "oauth1-key-kind",
        "oauth1-placement",
        "oauth1-include-body-hash",
        "oauth1-callback-url",
        "oauth1-verifier",
        "oauth1-timestamp",
        "oauth1-nonce",
        "oauth1-version",
        "oauth1-realm",
    ];
    let want: Vec<bool> = ids
        .iter()
        .map(|id| !matches!(*id, "oauth1-private-key" | "oauth1-key-kind"))
        .collect();
    assert_eq!(shown(cx, win, &ids), want);
    for (f, m) in [
        (OAuth1Field::ConsumerSecret, true),
        (OAuth1Field::TokenSecret, true),
        (OAuth1Field::PrivateKey, true),
        (OAuth1Field::ConsumerKey, false),
        (OAuth1Field::AccessToken, false),
        (OAuth1Field::Realm, false),
    ] {
        assert_eq!(masked(cx, &e, f), m, "{f:?}");
    }
}

#[gpui_kit::test]
fn rsa_methods_swap_the_consumer_secret_for_the_private_key(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::OAuth1(hmac()));
    let e = editor(cx, &s);
    on(cx, win, &e, |e, w, cx| {
        e.set_signature_method(OAuth1SignatureMethod::RsaSha256, w, cx)
    });
    assert_eq!(
        last(cx, &host).signature_method,
        OAuth1SignatureMethod::RsaSha256
    );
    assert_eq!(
        shown(
            cx,
            win,
            &[
                "oauth1-consumer-secret",
                "oauth1-private-key",
                "oauth1-key-kind"
            ]
        ),
        [false, true, true]
    );
    type_at_end(cx, win, None, OAuth1Field::PrivateKey.id(), "k");
    cx.run_until_parked();
    assert_eq!(
        last(cx, &host).private_key,
        Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "k"))
    );
    on(cx, win, &e, |e, w, cx| {
        e.set_key_kind(OAuth1KeyKind::File, w, cx)
    });
    assert_eq!(
        last(cx, &host).private_key,
        Some(OAuth1PrivateKey::new(OAuth1KeyKind::File, "k"))
    );
}

#[gpui_kit::test]
fn typing_selects_and_the_checkbox_send_the_whole_spec(cx: &mut TestAppContext) {
    let (win, host, s) = section(cx, AuthSpec::OAuth1(hmac()));
    let e = editor(cx, &s);
    type_at_end(cx, win, None, OAuth1Field::ConsumerKey.id(), "2");
    type_at_end(cx, win, None, OAuth1Field::Realm.id(), "r");
    cx.run_until_parked();
    on(cx, win, &e, |e, w, cx| {
        e.set_placement(OAuth1Placement::Body, w, cx)
    });
    on(cx, win, &e, |e, w, cx| e.set_include_body_hash(true, w, cx));
    let mut want = hmac();
    (want.consumer_key, want.realm) = (Some("ck2".into()), Some("r".into()));
    (want.placement, want.include_body_hash) = (OAuth1Placement::Body, true);
    assert_eq!(last(cx, &host), want);
}

#[gpui_kit::test]
fn absent_fields_and_unknown_names_survive_an_unrelated_edit(cx: &mut TestAppContext) {
    let mut o = hmac();
    (o.callback_url, o.verifier, o.private_key) = (None, None, None);
    o.signature_method = OAuth1SignatureMethod::Other("HMAC-MD5".into());
    o.placement = OAuth1Placement::Other("cookie".into());
    let (win, host, _s) = section(cx, AuthSpec::OAuth1(o.clone()));
    type_at_end(cx, win, None, OAuth1Field::Nonce.id(), "n");
    cx.run_until_parked();
    o.nonce = Some("n".into());
    assert_eq!(last(cx, &host), o);
}

#[gpui_kit::test]
fn a_key_kind_picked_before_any_key_survives_a_reload(cx: &mut TestAppContext) {
    // A file read with no `privateKey` has no key at all, not an empty one.
    let mut o = hmac();
    (o.signature_method, o.private_key) = (OAuth1SignatureMethod::RsaSha256, None);
    let (win, host, s) = section(cx, AuthSpec::OAuth1(o.clone()));
    let e = editor(cx, &s);
    on(cx, win, &e, |e, w, cx| {
        e.set_key_kind(OAuth1KeyKind::File, w, cx)
    });
    // A key with no value is not written, so the app reloads the spec without one.
    cx.update_window(win, |_, w, cx| {
        s.update(cx, |s, cx| s.load(&AuthSpec::OAuth1(o.clone()), w, cx))
    })
    .unwrap();
    frame(cx, win);
    type_at_end(cx, win, None, OAuth1Field::PrivateKey.id(), "keys/k.pem");
    cx.run_until_parked();
    assert_eq!(
        last(cx, &host).private_key,
        Some(OAuth1PrivateKey::new(OAuth1KeyKind::File, "keys/k.pem"))
    );
}

#[gpui_kit::test]
fn oauth1_fields_and_choices_sit_in_form_rows(cx: &mut TestAppContext) {
    let (win, _host, _s) = section(cx, AuthSpec::OAuth1(OAuth1Spec::default()));
    cx.update_window(win, |_, w, _| {
        for (row, control) in [
            ("oauth1-consumer-key-row", "oauth1-consumer-key"),
            ("oauth1-signature-method-row", "oauth1-signature-method"),
            ("oauth1-placement-row", "oauth1-placement"),
        ] {
            assert!(w.within(row).try_find("form-label").is_some(), "{row}");
            assert!(w.within(row).try_find(control).is_some(), "{row}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_body_hash_box_sits_in_the_field_column(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let (win, _host, _s) = section(cx, AuthSpec::OAuth1(OAuth1Spec::default()));
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let field = w
            .within("oauth1-placement-row")
            .find("form-control")
            .bounds();
        let control = w
            .within("oauth1-include-body-hash-row")
            .find("form-control")
            .bounds();
        let check = w.find("oauth1-include-body-hash").bounds();
        assert_eq!(control.left(), field.left());
        assert!(
            (check.left() - field.left()).abs() < px(0.5),
            "{check:?} vs {field:?}"
        );
    })
    .unwrap();
}
