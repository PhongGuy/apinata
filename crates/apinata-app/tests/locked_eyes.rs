//! A kept response whose header values lost a secret when saved shows each such row's eye
//! masked and locked, saying why it cannot reveal; a locked button tells snapshots (and so the
//! pilot tree) it is disabled through a `locked` marker that leaves its bounds alone. Every
//! check reads the frame the platform drew next.

use crate::support;

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use apinata_app::body_view::SCRUBBED_TIP;
use apinata_app::executor::SharedExecutor;
use apinata_app::last_response::{Limits, ResponseStore};
use apinata_app::response_pane::ResponseTab;
use apinata_app::ui::{self, LockButton as _};
use apinata_core::{ExecOptions, Executor, Outcome, RequestResult, Response};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::test::{ElementSnapshot, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, ElementId, Entity, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, Window, WindowOptions,
};
use support::*;

const SECRET: &str = "s3cr3t-value-123";
const HOST: &str = "http://127.0.0.1:9/a";

// A kept send of `a.yml` restored into its editor, with what keeps its files alive.
struct Kept {
    win: AnyWindowHandle,
    store: ResponseStore,
    _keep: (tempfile::TempDir, tempfile::TempDir),
}

fn kept_result(body: &str) -> RequestResult {
    let response = Response::new(200)
        .with_url(HOST)
        .with_header("x-token", format!("token {SECRET}"))
        .with_header("x-plain", "hello")
        .with_header("set-cookie", format!("sid={SECRET}; Path=/"))
        .with_text_body(body);
    RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    )
}

// Saves a send with `body` scrubbed of SECRET, lets `edit` change the kept files, then restores
// it into an editor on the `tab` response tab and draws the next frame.
fn restored_with(
    cx: &mut TestAppContext,
    (tab, body): (ResponseTab, &str),
    jar: bool,
    edit: impl FnOnce(&ResponseStore),
) -> Kept {
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", &request("a", HOST))]);
    let (state, ws, root) = open_dir(cx, &col);
    let store = ResponseStore::new(state.path().join("responses"));
    store
        .save_redacted(
            &root,
            &ip("a.yml"),
            &kept_result(body),
            &[SECRET.to_owned()],
            UNIX_EPOCH + Duration::from_secs(1_760_000_000),
            Limits::default(),
        )
        .unwrap();
    edit(&store);
    if jar {
        let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
        fill_jar(&exec);
        cx.update(|cx| cx.set_global(SharedExecutor(exec)));
    }
    cx.update(|cx| apinata_app::last_response::install(store.clone(), cx));
    let (win, editor) = editor_for(cx, &ws, &root, "a.yml", false);
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    wait_real(cx, |cx| model.read_with(cx, |m, _| m.restored().is_some()));
    let pane = editor.read_with(cx, |e, _| e.response().clone());
    pane.update(cx, |p, cx| jar_tab(p, tab, cx));
    next_frame(cx, win);
    Kept {
        win,
        store,
        _keep: (col, state),
    }
}

fn restored(
    cx: &mut TestAppContext,
    tab: ResponseTab,
    jar: bool,
    edit: impl FnOnce(&ResponseStore),
) -> Kept {
    restored_with(cx, (tab, "ok"), jar, edit)
}

// What the last drawn frame holds for `id`, read without drawing another.
fn drawn(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> Option<ElementSnapshot> {
    let id = id.into();
    cx.update_window(win, |_, w, _| w.try_find(id)).unwrap()
}

// The `locked` marker inside `button` in the last drawn frame.
fn marker(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    button: impl Into<ElementId>,
) -> Option<ElementSnapshot> {
    let button = button.into();
    cx.update_window(win, |_, w, _| {
        w.try_find(button.clone())?;
        w.within(button).try_find("locked")
    })
    .unwrap()
}

fn row_named(
    cx: &mut TestAppContext,
    win: AnyWindowHandle,
    name_id: &'static str,
    name: &str,
) -> usize {
    (0..40)
        .find(|&ix| drawn(cx, win, (name_id, ix)).is_some_and(|e| e.label() == Some(name)))
        .unwrap_or_else(|| panic!("no {name} row in {name_id}"))
}

fn label_of(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> String {
    drawn(cx, win, id)
        .and_then(|e| e.label().map(str::to_owned))
        .unwrap_or_default()
}

// Asserts header row `ix` shows its masked, locked eye, which a click leaves masked.
fn assert_locked_eye(cx: &mut TestAppContext, k: &Kept, ix: usize, value: &str) {
    let eye = drawn(cx, k.win, ("header-eye", ix)).expect("the row has an eye");
    assert_eq!(eye.label(), Some(SCRUBBED_TIP), "the eye says why");
    assert!(
        drawn(cx, k.win, ("header-reveal", ix)).is_some(),
        "the eye shows its masked face"
    );
    let lock = marker(cx, k.win, ("header-eye", ix)).expect("the eye is locked");
    assert_eq!(lock.disabled(), Some(true), "the lock reads as disabled");
    assert_eq!(label_of(cx, k.win, ("header-value", ix)), value);
    cx.update_window(k.win, |_, w, cx| w.click(("header-eye", ix), cx))
        .unwrap();
    next_frame(cx, k.win);
    assert!(
        drawn(cx, k.win, ("header-hide", ix)).is_none(),
        "a click reveals nothing"
    );
    assert_eq!(label_of(cx, k.win, ("header-value", ix)), value);
}

// T1: a header that lost a secret when saved.

#[gpui_kit::test]
fn a_restored_scrubbed_header_shows_a_locked_eye(cx: &mut TestAppContext) {
    let k = restored(cx, ResponseTab::Headers, false, |_| {});
    let ix = row_named(cx, k.win, "header-name", "x-token");
    assert_locked_eye(cx, &k, ix, "token [redacted]");
}

#[gpui_kit::test]
fn a_restored_plain_header_has_no_eye(cx: &mut TestAppContext) {
    let k = restored(cx, ResponseTab::Headers, false, |_| {});
    let ix = row_named(cx, k.win, "header-name", "x-plain");
    assert!(drawn(cx, k.win, ("header-eye", ix)).is_none());
}

#[gpui_kit::test]
fn a_restored_scrubbed_set_cookie_shows_a_locked_eye(cx: &mut TestAppContext) {
    let k = restored(cx, ResponseTab::Headers, false, |_| {});
    let ix = row_named(cx, k.win, "header-name", "set-cookie");
    assert_locked_eye(cx, &k, ix, "sid=[redacted]");
}

// Every kept send file of `store`, the index aside.
fn entry_files(store: &ResponseStore) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![store.dir().to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for file in std::fs::read_dir(dir).unwrap() {
            let path = file.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[gpui_kit::test]
fn an_entry_without_masked_headers_restores_without_locked_eyes(cx: &mut TestAppContext) {
    let k = restored(cx, ResponseTab::Headers, false, |store| {
        let mut stripped = 0;
        for path in entry_files(store) {
            let named_index = path.file_name().is_some_and(|n| n == "index.json");
            if named_index || path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let mut entry: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            if entry
                .as_object_mut()
                .unwrap()
                .remove("masked_headers")
                .is_some()
            {
                stripped += 1;
            }
            std::fs::write(&path, entry.to_string()).unwrap();
        }
        assert_eq!(stripped, 1, "the kept entry named its scrubbed headers");
    });
    let ix = row_named(cx, k.win, "header-name", "x-token");
    assert!(drawn(cx, k.win, ("header-eye", ix)).is_none());
    let cookie = row_named(cx, k.win, "header-name", "set-cookie");
    assert!(
        marker(cx, k.win, ("header-eye", cookie)).is_none(),
        "nothing is locked"
    );
}

// Guard: the scrub path still keeps the secret out of every saved file.
#[gpui_kit::test]
fn the_kept_files_never_hold_the_secret(cx: &mut TestAppContext) {
    let k = restored(cx, ResponseTab::Headers, false, |_| {});
    let files = entry_files(&k.store);
    assert!(!files.is_empty());
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(SECRET),
            "{path:?} holds the secret"
        );
    }
}

// Guard: the Cookies tab lists the live jar, whose values the eye still reveals.
#[gpui_kit::test]
fn the_cookies_tab_eye_on_a_restored_response_reveals_the_jar_value(cx: &mut TestAppContext) {
    let k = restored(cx, ResponseTab::Cookies, true, |_| {});
    let sid = row_named(cx, k.win, "cookie-name", "sid");
    assert_eq!(label_of(cx, k.win, ("cookie-value", sid)), "[redacted]");
    assert!(
        marker(cx, k.win, ("cookie-eye", sid)).is_none(),
        "not locked"
    );
    cx.update_window(k.win, |_, w, cx| w.click(("cookie-reveal", sid), cx))
        .unwrap();
    next_frame(cx, k.win);
    assert_eq!(label_of(cx, k.win, ("cookie-value", sid)), "S1");
}

// A kept result has no timeline, so this response's cookies were never recorded.
#[gpui_kit::test]
fn a_restored_result_says_its_cookies_were_not_recorded(cx: &mut TestAppContext) {
    let k = restored(cx, ResponseTab::Cookies, false, |_| {});
    cx.update_window(k.win, |_, w, cx| {
        w.render_frame(cx);
        w.click("cookies-view-response", cx)
    })
    .unwrap();
    next_frame(cx, k.win);
    assert_eq!(
        label_of(cx, k.win, "set-cookies-unrecorded"),
        "Cookies were not recorded for this result."
    );
    assert!(drawn(cx, k.win, "set-cookies-none").is_none());
    assert!(drawn(cx, k.win, "cookies-view-jar").is_some());
}

// T3: a locked button reads as disabled to snapshots, at the same bounds.

#[gpui_kit::test]
fn a_restored_scrubbed_body_eye_reads_as_disabled(cx: &mut TestAppContext) {
    let body = format!("token={SECRET}");
    let k = restored_with(cx, (ResponseTab::Body, &body), false, |_| {});
    assert!(
        drawn(cx, k.win, "body-eye").is_some(),
        "the body has its eye"
    );
    let lock = marker(cx, k.win, "body-eye").expect("the eye is locked");
    assert_eq!(lock.disabled(), Some(true));
}

// A labelled and an icon-only button in a row, as toolbars and table rows hold them.
struct Pair {
    locked: bool,
}

impl Render for Pair {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::component::h_flex()
            .size_full()
            .child(Button::new("labelled").label("Send").lock(self.locked, cx))
            .child(ui::icon_button("icon-only", IconName::Copy, "Copy", None).lock(self.locked, cx))
    }
}

fn pair(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Pair>) {
    support::init(cx);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| Pair { locked: false })
            })
        })
        .unwrap();
    cx.run_until_parked();
    frame(cx, win);
    (win, view)
}

#[gpui_kit::test]
fn a_locked_button_reads_as_disabled_at_the_same_bounds(cx: &mut TestAppContext) {
    let (win, view) = pair(cx);
    let ids = ["labelled", "icon-only"];
    let open: Vec<_> = ids
        .iter()
        .map(|id| drawn(cx, win, *id).expect("drawn").bounds())
        .collect();
    for id in ids {
        assert!(
            marker(cx, win, id).is_none(),
            "{id}: an unlocked button has no marker"
        );
    }
    view.update(cx, |v, cx| {
        v.locked = true;
        cx.notify();
    });
    next_frame(cx, win);
    for (id, before) in ids.iter().zip(open) {
        let lock = marker(cx, win, *id).unwrap_or_else(|| panic!("{id}: locked"));
        assert_eq!(lock.disabled(), Some(true), "{id}");
        let after = drawn(cx, win, *id).unwrap().bounds();
        assert_eq!(after, before, "{id}: locking keeps the bounds");
        let inner = lock.bounds();
        assert!(
            after.contains(&inner.origin) && after.contains(&inner.center()),
            "{id}: the marker {inner:?} sits on the button {after:?}"
        );
    }
    view.update(cx, |v, cx| {
        v.locked = false;
        cx.notify();
    });
    next_frame(cx, win);
    for id in ids {
        assert!(
            marker(cx, win, id).is_none(),
            "{id}: unlocking drops the marker"
        );
    }
}
