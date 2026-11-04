//! Kept sends carry keyed digests of their masked header values, so Compare finds a change
//! the mask hides after a relaunch, and a reveal shows the true value in every tab.

use crate::support;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use apinata_app::app::VolatileSecrets;
use apinata_app::last_response::install;
use apinata_app::last_response::{DIGEST_KEY_NAME, ResponseStore};
use apinata_app::response_pane::{ResponsePane, ResponseTab};
use apinata_app::send_model::SendModel;
use apinata_app::send_model::SendState;
use apinata_core::{DigestKey, MemoryStore, SecretKey, SecretStore};
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext};
use support::*;

use crate::compare_reveal::{click, header_row, kept, label, open_compare, template};
use crate::restore_disk::{close, relaunch_with};

fn plain_collection() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://127.0.0.1:9/a")),
    ])
}

fn current_key(cx: &mut TestAppContext) -> Option<DigestKey> {
    cx.update(|cx| cx.global::<ResponseStore>().digest_key())
}

#[gpui_kit::test]
fn the_app_loads_the_digest_key_from_its_secret_store(cx: &mut TestAppContext) {
    let col = plain_collection();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let _s = started_with_store(cx, &col, secrets.clone());
    let loaded = current_key(cx).expect("the key is loaded once the app starts");
    let stored = DigestKey::load(secrets.as_ref(), DIGEST_KEY_NAME).unwrap();
    assert_eq!(loaded.id(), stored.id());
}

#[gpui_kit::test]
fn a_volatile_run_makes_and_uses_no_digest_key(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(VolatileSecrets));
    let col = plain_collection();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let _s = started_with_store(cx, &col, secrets.clone());
    assert!(current_key(cx).is_none());
    let slot = SecretKey::new("", "", DIGEST_KEY_NAME);
    assert_eq!(secrets.get(&slot).unwrap(), None, "no key was made");
}

const SECRET: &str = "Echo-S3cret-66";

// Answers every later request with `headers`.
fn answer_with(cx: &mut TestAppContext, server: &wiremock::MockServer, headers: &[(&str, &str)]) {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(server.reset());
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(template(headers))
            .mount(server),
    );
}

fn send_and_wait(
    cx: &mut TestAppContext,
    model: &Entity<SendModel>,
    pane: &Entity<ResponsePane>,
    root: &Path,
    n: usize,
) {
    let r = root.to_path_buf();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    finished(cx, model);
    wait_real(cx, |cx| pane.read_with(cx, |p, _| p.history().len() == n));
}

// Opens `a.yml` in `s` and sends it twice, answered with `first` then `second` as headers.
fn two_sends(
    cx: &mut TestAppContext,
    s: &Started,
    server: &wiremock::MockServer,
    first: &[(&str, &str)],
    second: &[(&str, &str)],
) -> Entity<ResponsePane> {
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = editor_of(cx, s, tab);
    let (model, pane) = editor.read_with(cx, |e, _| (e.send_model().clone(), e.response().clone()));
    for (n, headers) in [(1, first), (2, second)] {
        answer_with(cx, server, headers);
        send_and_wait(cx, &model, &pane, &s.root, n);
    }
    pane
}

fn row_named(cx: &mut TestAppContext, win: AnyWindowHandle, name: &str) -> usize {
    (0..40usize)
        .find(|&ix| {
            present(cx, win, ("header-name", ix)) && label(cx, win, ("header-name", ix)) == name
        })
        .unwrap_or_else(|| panic!("no {name} row"))
}

#[gpui_kit::test]
fn an_echoed_secret_reveals_the_same_text_in_headers_and_compare(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, template(&[]));
    let a = request("a", &format!("{}/ok?token={{{{token}}}}", server.uri()));
    let dev = "name: dev\nvariables:\n  - name: token\n    secret: true\n";
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("a.yml", &a),
    ]);
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    secrets.set(&c.secret_key("dev", "token"), SECRET).unwrap();
    let s = started_with_store(cx, &col, secrets);
    s.ws.update(cx, |w, cx| {
        w.select_environment(&s.root, Some("dev".into()), cx)
    });
    cx.run_until_parked();
    let pane = two_sends(
        cx,
        &s,
        &server,
        &[("x-echo", "before")],
        &[("x-echo", SECRET)],
    );

    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    frame(cx, s.win);
    let row = row_named(cx, s.win, "x-echo");
    assert_eq!(label(cx, s.win, ("header-value", row)), "[redacted]");
    click(cx, s.win, ("header-reveal", row));
    let in_headers = label(cx, s.win, ("header-value", row));
    assert_eq!(in_headers, SECRET);

    open_compare(cx, s.win, &pane);
    let ix = header_row(cx, s.win, "x-echo").expect("an x-echo row");
    assert!(
        !label(cx, s.win, ("compare-row", ix)).contains(SECRET),
        "masked until revealed"
    );
    click(cx, s.win, ("compare-eye", ix));
    let in_compare = label(cx, s.win, ("compare-row", ix));
    assert!(
        in_compare.contains(&in_headers) && in_compare.contains("before"),
        "{in_compare}"
    );
}

// The row shows masked text on both sides, and its eye is locked: the kept side has no true value.
fn assert_locked_masked(cx: &mut TestAppContext, win: AnyWindowHandle, ix: usize) {
    let row = label(cx, win, ("compare-row", ix));
    assert!(
        row.contains("sid=") && !row.contains("aaa") && !row.contains("bbb"),
        "{row}"
    );
    assert!(present(cx, win, ("compare-reveal", ix)), "a locked eye");
    click(cx, win, ("compare-eye", ix));
    assert_eq!(
        label(cx, win, ("compare-row", ix)),
        row,
        "nothing to reveal"
    );
    assert!(!present(cx, win, ("compare-hide", ix)));
}

// A send kept under a key before the pane opened, then one live send answered with `live`.
fn live_after_kept(
    cx: &mut TestAppContext,
    kept_cookie: &str,
    live: &str,
) -> (
    AnyWindowHandle,
    Entity<ResponsePane>,
    Box<dyn std::any::Any>,
) {
    http_ready(cx);
    let server = mock_server(cx, template(&[("set-cookie", live)]));
    let col = collection_for(&server.uri(), "");
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    store.set_digest_key(DigestKey::load(&MemoryStore::new(), DIGEST_KEY_NAME).unwrap());
    let earlier = SystemTime::now() - Duration::from_secs(400);
    store
        .save(
            &root,
            &ip("a.yml"),
            &kept(&[("set-cookie", kept_cookie)]),
            earlier,
        )
        .unwrap();
    cx.update(|cx| install(store, cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let (model, pane) = editor.read_with(cx, |e, _| (e.send_model().clone(), e.response().clone()));
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 1)
            && model.read_with(cx, |m, _| matches!(m.state(), SendState::Done(_)))
    });
    send_and_wait(cx, &model, &pane, &root, 2);
    open_compare(cx, win, &pane);
    (win, pane, Box::new((server, col, state, editor, ws)))
}

#[gpui_kit::test]
fn a_live_send_finds_a_changed_cookie_against_a_kept_one(cx: &mut TestAppContext) {
    let (win, _pane, _keep) = live_after_kept(cx, "sid=aaa; Path=/", "sid=bbb; Path=/");
    let ix = header_row(cx, win, "set-cookie").expect("the changed cookie's row");
    assert_locked_masked(cx, win, ix);
}

#[gpui_kit::test]
fn a_live_send_with_the_kept_cookie_makes_no_row(cx: &mut TestAppContext) {
    let (win, _pane, _keep) = live_after_kept(cx, "sid=aaa; Path=/", "sid=aaa; Path=/");
    assert_eq!(header_row(cx, win, "set-cookie"), None);
}

// `a.yml`'s editor in `s` once its two kept sends are listed and the newest is restored.
fn restored_pane(cx: &mut TestAppContext, s: &Started) -> Entity<ResponsePane> {
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = editor_of(cx, s, tab);
    let (model, pane) = editor.read_with(cx, |e, _| (e.send_model().clone(), e.response().clone()));
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, _| p.history().len() == 2)
            && model.read_with(cx, |m, _| {
                m.restored_ref().is_some() && matches!(m.state(), SendState::Done(_))
            })
    });
    pane
}

// Two sends with different cookies, then a relaunch with kept data over the first run's
// secret store when `same_store`, else over a fresh one (a reset keychain).
fn relaunched(
    cx: &mut TestAppContext,
    same_store: bool,
) -> (Started, Entity<ResponsePane>, tempfile::TempDir) {
    http_ready(cx);
    let server = mock_server(cx, template(&[]));
    let col = collection_for(&server.uri(), "");
    let secrets: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let s = started_with_store(cx, &col, secrets.clone());
    assert!(current_key(cx).is_some(), "the first run digests its sends");
    two_sends(
        cx,
        &s,
        &server,
        &[("set-cookie", "sid=aaa; Path=/")],
        &[("set-cookie", "sid=bbb; Path=/")],
    );
    let root = s.root.clone();
    let files = close(cx, s);
    let store: Arc<dyn SecretStore> = if same_store {
        secrets
    } else {
        Arc::new(MemoryStore::new())
    };
    let s = relaunch_with(cx, files, root, store);
    wait_real(cx, |cx| current_key(cx).is_some());
    // The relaunch restores the tab, so `restored_pane` only activates it.
    assert!(
        tab_names(cx, &s.ws).contains(&"a.yml".to_owned()),
        "the session kept the tab"
    );
    let pane = restored_pane(cx, &s);
    open_compare(cx, s.win, &pane);
    (s, pane, col)
}

#[gpui_kit::test]
fn a_relaunch_on_the_same_keychain_finds_a_changed_masked_cookie(cx: &mut TestAppContext) {
    let (s, _pane, _col) = relaunched(cx, true);
    let ix = header_row(cx, s.win, "set-cookie").expect("the changed cookie's row");
    assert_locked_masked(cx, s.win, ix);
}

#[gpui_kit::test]
fn a_relaunch_on_another_keychain_compares_masked_text(cx: &mut TestAppContext) {
    let (s, _pane, _col) = relaunched(cx, false);
    assert_eq!(header_row(cx, s.win, "set-cookie"), None);
}
