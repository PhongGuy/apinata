use crate::support;

use apinata_app::app::VolatileSecrets;
use apinata_app::cookie_jar::{
    CookieField, CookieJarView, expiry_label, open_cookie_jar, parse_expires,
};
use apinata_app::executor::{SharedExecutor, install, shared};
use apinata_app::persist::Paths;
use apinata_core::{CookieInfo, CookieJar, Persistence, SameSite, SecretStore};
use apinata_core::{ExecOptions, Executor, MemoryStore};
use chrono::TimeZone as _;
use gpui_kit::ElementId;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, Render, TestAppContext, Window,
    WindowOptions, div,
};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use support::*;

struct Blank;
impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn dialog(
    cx: &mut TestAppContext,
    filled: bool,
) -> (
    AnyWindowHandle,
    Option<Arc<Executor>>,
    Entity<CookieJarView>,
) {
    support::init(cx);
    let exec = filled.then(|| Arc::new(Executor::new(ExecOptions::new()).unwrap()));
    if let Some(e) = &exec {
        fill_jar(e);
        cx.update(|cx| cx.set_global(SharedExecutor(e.clone())));
    }
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let view = cx
        .update_window(win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, win);
    (win, exec, view)
}

#[test]
fn expiry_labels() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    assert_eq!(expiry_label(None, now), "Session");
    assert_eq!(
        expiry_label(Some(now - Duration::from_secs(1)), now),
        "Expired"
    );
    assert_eq!(
        expiry_label(Some(now + Duration::from_secs(300)), now),
        "in 5 min"
    );
    assert_eq!(
        expiry_label(Some(now + Duration::from_secs(3 * 3600)), now),
        "in 3 h"
    );
    assert_eq!(
        expiry_label(Some(now + Duration::from_secs(2 * 86400 + 60)), now),
        "in 2 d"
    );
}

#[gpui_kit::test]
fn the_dialog_lists_and_deletes_one_cookie(cx: &mut TestAppContext) {
    let (win, exec, _view) = dialog(cx, true);
    let exec = exec.unwrap();
    assert!(
        cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
    assert!(present(cx, win, ("jar-cookie", 1usize)) && present(cx, win, ("jar-domain", 0usize)));
    cx.update_window(win, |_, w, cx| w.click(("jar-delete", 0usize), cx))
        .unwrap();
    frame(cx, win);
    let names: Vec<String> = exec.cookies().list().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["theme"]);
    assert!(!present(cx, win, ("jar-cookie", 1usize)));
}

#[gpui_kit::test]
fn a_domain_delete_and_clear_empty_the_jar(cx: &mut TestAppContext) {
    let (win, exec, view) = dialog(cx, true);
    let exec = exec.unwrap();
    cx.update_window(win, |_, w, cx| w.click(("jar-domain-delete", 0usize), cx))
        .unwrap();
    frame(cx, win);
    assert!(exec.cookies().list().is_empty() && present(cx, win, "jar-empty"));
    assert!(!present(cx, win, "jar-clear"), "nothing to clear");
    fill_jar(&exec);
    view.update(cx, |v, cx| v.refresh(cx));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("jar-clear", cx))
        .unwrap();
    frame(cx, win);
    assert!(exec.cookies().list().is_empty());
}

#[gpui_kit::test]
fn without_an_executor_the_dialog_says_so(cx: &mut TestAppContext) {
    let (win, _, _) = dialog(cx, false);
    assert!(present(cx, win, "jar-unavailable"));
}

#[gpui_kit::test]
fn install_builds_one_shared_executor(cx: &mut TestAppContext) {
    assert!(cx.update(|cx| shared(cx)).is_none());
    let store = MemoryStore::new();
    cx.update(|cx| install(&test_settings(), &store, cx))
        .unwrap();
    let a = cx.update(|cx| shared(cx)).unwrap();
    let b = cx.update(|cx| shared(cx)).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
}

// One response setting `n` cookies, so the jar holds `n` rows.
fn fill_many(exec: &Executor, n: usize) {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        let mut set = ResponseTemplate::new(200);
        for i in 0..n {
            set = set.append_header("set-cookie", format!("c{i:02}=v; Path=/").as_str());
        }
        Mock::given(path("/set"))
            .respond_with(set)
            .mount(&server)
            .await;
        let dir = write_tree(&[
            ("opencollection.yml", ROOT),
            ("set.yml", &request("set", &format!("{}/set", server.uri()))),
        ]);
        let c = apinata_core::Collection::open(dir.path()).unwrap();
        let ctx = apinata_core::VarContext::new(&c, apinata_core::ContextOptions::new()).unwrap();
        let req = apinata_core::resolve(c.request(&ip("set.yml")).unwrap(), &ctx).unwrap();
        exec.send(&req).await.unwrap();
    });
}

#[gpui_kit::test]
fn a_full_jar_scrolls_inside_the_dialog(cx: &mut TestAppContext) {
    support::init(cx);
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    fill_many(&exec, 40);
    cx.update(|cx| cx.set_global(SharedExecutor(exec.clone())));
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    cx.simulate_window_resize(win, gpui_kit::size(gpui_kit::px(900.), gpui_kit::px(400.)));
    cx.update_window(win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, win);
    let before = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("jar-cookie", 0usize)).bounds()
        })
        .unwrap();
    let after = cx
        .update_window(win, |_, w, cx| {
            w.scroll(
                "cookie-jar-scroll",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-200.),
                )),
                cx,
            );
            w.find(("jar-cookie", 0usize)).bounds()
        })
        .unwrap();
    assert!(
        after.top() < before.top(),
        "no scroll: {before:?} then {after:?}"
    );
}

/// A loaded jar over `paths.cookies`, as a fresh start would read it.
fn saved_jar(paths: &Paths, store: Arc<dyn SecretStore>) -> CookieJar {
    let jar = CookieJar::open(paths.cookies.clone(), store);
    jar.load();
    jar
}

fn cookie_names(jar: &CookieJar) -> Vec<String> {
    jar.list().into_iter().map(|c| c.name).collect()
}

#[test]
fn the_cookie_file_sits_in_the_profile_and_beside_the_token_cache() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        Paths::in_dir(dir.path()).cookies,
        dir.path().join("cookies.json")
    );
    if let Some(p) = Paths::platform() {
        assert_eq!(p.cookies.parent(), p.oauth2_tokens.parent());
        assert!(p.cookies.ends_with("apinata/cookies.json"));
    }
}

#[gpui_kit::test]
fn a_saved_cookie_is_loaded_at_start_and_a_new_one_is_written_at_quit(cx: &mut TestAppContext) {
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let seed = store.clone();
    let s = started_on(
        cx,
        &col,
        move |paths| {
            let jar = saved_jar(paths, seed);
            jar.insert(CookieInfo::new("saved.test", "kept", "K1"))
                .unwrap();
            jar.save().unwrap();
        },
        store.clone(),
    );
    let exec = cx.update(|cx| shared(cx)).expect("an executor");
    assert_eq!(exec.cookies().persistence(), Persistence::Encrypted);
    assert_eq!(cookie_names(exec.cookies()), ["kept"]);
    exec.cookies()
        .insert(CookieInfo::new("new.test", "added", "A1"))
        .unwrap();
    cx.quit();
    let paths = Paths::in_dir(s.files.path());
    assert_eq!(cookie_names(&saved_jar(&paths, store)), ["added", "kept"]);
}

#[gpui_kit::test]
fn the_save_tick_writes_a_change(cx: &mut TestAppContext) {
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started_with_store(cx, &col, store.clone());
    let paths = Paths::in_dir(s.files.path());
    let exec = cx.update(|cx| shared(cx)).expect("an executor");
    exec.cookies()
        .insert(CookieInfo::new("tick.test", "ticked", "T1"))
        .unwrap();
    cx.run_until_parked();
    assert!(
        !paths.cookies.exists(),
        "nothing is written before the tick"
    );
    cx.executor()
        .advance_clock(apinata_app::app::COOKIE_SAVE_TICK);
    cx.run_until_parked();
    assert_eq!(cookie_names(&saved_jar(&paths, store)), ["ticked"]);
}

#[gpui_kit::test]
fn volatile_secrets_keep_cookies_in_memory_and_leave_the_file_alone(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(VolatileSecrets));
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started_with(cx, &col, |paths| {
        std::fs::write(&paths.cookies, b"kept by another run").unwrap();
    });
    let exec = cx.update(|cx| shared(cx)).expect("an executor");
    assert!(matches!(
        exec.cookies().persistence(),
        Persistence::MemoryOnly { .. }
    ));
    exec.cookies()
        .insert(CookieInfo::new("v.test", "sid", "1"))
        .unwrap();
    cx.executor().advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
    cx.quit();
    let path = Paths::in_dir(s.files.path()).cookies;
    assert_eq!(std::fs::read(path).unwrap(), b"kept by another run");
}

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, win);
}

fn dialog_on(cx: &mut TestAppContext, win: AnyWindowHandle) -> Entity<CookieJarView> {
    let view = cx
        .update_window(win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, win);
    view
}

fn form_error(cx: &mut TestAppContext, view: &Entity<CookieJarView>) -> Option<String> {
    view.read_with(cx, |v, _| v.form_error().map(str::to_owned))
}

#[test]
fn expiry_text_parses_local_minutes_and_rfc3339_and_rejects_the_past() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    assert_eq!(parse_expires("", now), Ok(None));
    assert_eq!(parse_expires("   ", now), Ok(None));
    assert_eq!(
        parse_expires("2030-01-02T03:04:05Z", now),
        Ok(Some(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_893_553_445)
        ))
    );
    let local = chrono::NaiveDate::from_ymd_opt(2030, 1, 2)
        .unwrap()
        .and_hms_opt(3, 4, 0)
        .unwrap();
    let expected = chrono::Local
        .from_local_datetime(&local)
        .earliest()
        .map(SystemTime::from);
    assert_eq!(parse_expires("2030-01-02 03:04", now), Ok(expected));
    assert!(
        parse_expires("2020-01-01T00:00:00Z", now)
            .unwrap_err()
            .contains("future")
    );
    assert!(parse_expires("next tuesday", now).is_err());
    assert!(parse_expires("2030-13-01 00:00", now).is_err());
}

#[gpui_kit::test]
fn a_cookie_added_edited_and_deleted_in_the_dialog_is_saved_each_time(cx: &mut TestAppContext) {
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started_with_store(cx, &col, store.clone());
    let paths = Paths::in_dir(s.files.path());
    let view = dialog_on(cx, s.win);
    assert_eq!(
        view.read_with(cx, |v, _| v.footer()),
        "Cookies are saved encrypted."
    );
    assert!(present(cx, s.win, "jar-footer"));

    click(cx, s.win, "jar-add");
    assert!(present(cx, s.win, "jar-form"));
    replace_input(cx, s.win, None, "jar-form-domain", "app.test");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    replace_input(cx, s.win, None, "jar-form-value", "first");
    click(cx, s.win, "jar-form-secure");
    click(cx, s.win, "jar-form-save");
    cx.run_until_parked();
    assert!(!present(cx, s.win, "jar-form"), "the form closes on save");
    let saved = saved_jar(&paths, store.clone()).list();
    assert_eq!(saved.len(), 1);
    let c = &saved[0];
    assert_eq!(
        (
            c.domain.as_str(),
            c.path.as_str(),
            c.name.as_str(),
            c.value.as_str()
        ),
        ("app.test", "/", "sid", "first")
    );
    assert!(c.secure && !c.http_only && c.host_only && c.expires.is_none());

    click(cx, s.win, ("jar-edit", 0usize));
    let readonly: Vec<bool> = view.read_with(cx, |v, cx| {
        [
            CookieField::Domain,
            CookieField::Path,
            CookieField::Name,
            CookieField::Value,
        ]
        .into_iter()
        .map(|f| {
            v.form_input(f)
                .expect("the form is open")
                .read(cx)
                .presentation()
                .is_readonly()
        })
        .collect()
    });
    assert_eq!(readonly, [true, true, true, false]);
    replace_input(cx, s.win, None, "jar-form-value", "second");
    replace_input(cx, s.win, None, "jar-form-expires", "2099-01-01T00:00:00Z");
    click(cx, s.win, "jar-form-save");
    cx.run_until_parked();
    let saved = saved_jar(&paths, store.clone()).list();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].value, "second");
    assert!(
        saved[0].secure && saved[0].host_only,
        "an edit keeps the flags and host_only"
    );
    assert_eq!(
        saved[0].expires,
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(4_070_908_800))
    );

    click(cx, s.win, ("jar-delete", 0usize));
    cx.run_until_parked();
    assert!(saved_jar(&paths, store).list().is_empty());
}

#[gpui_kit::test]
fn invalid_input_shows_an_inline_error_and_inserts_nothing(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let view = dialog_on(cx, s.win);
    let jar_len =
        |cx: &mut TestAppContext| cx.update(|cx| shared(cx)).unwrap().cookies().list().len();
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-domain", ".app.test");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    replace_input(cx, s.win, None, "jar-form-value", "v");
    replace_input(cx, s.win, None, "jar-form-expires", "2001-01-01 00:00");
    click(cx, s.win, "jar-form-save");
    assert!(form_error(cx, &view).expect("an error").contains("future"));
    assert!(present(cx, s.win, "jar-form-expires-error") && present(cx, s.win, "jar-form"));
    replace_input(cx, s.win, None, "jar-form-expires", "soon");
    click(cx, s.win, "jar-form-save");
    assert!(form_error(cx, &view).is_some());

    replace_input(cx, s.win, None, "jar-form-expires", "");
    replace_input(cx, s.win, None, "jar-form-name", "a b");
    click(cx, s.win, "jar-form-save");
    assert!(
        form_error(cx, &view)
            .expect("an error")
            .starts_with("Invalid cookie name")
    );
    assert_eq!(jar_len(cx), 0);

    replace_input(cx, s.win, None, "jar-form-name", "sid");
    click(cx, s.win, "jar-form-save");
    assert!(!present(cx, s.win, "jar-form"));
    let listed = cx.update(|cx| shared(cx)).unwrap().cookies().list();
    assert_eq!(
        (listed[0].domain.as_str(), listed[0].host_only),
        ("app.test", false)
    );
}

#[gpui_kit::test]
fn while_the_jar_loads_editing_is_disabled_and_the_footer_says_so(cx: &mut TestAppContext) {
    support::init(cx);
    let dir = tempfile::tempdir().unwrap();
    let jar = Arc::new(CookieJar::open(
        dir.path().join("cookies.json"),
        Arc::new(MemoryStore::new()),
    ));
    jar.insert(CookieInfo::new("early.test", "e", "E1"))
        .unwrap();
    let exec = Arc::new(Executor::new(ExecOptions::new().cookie_jar(jar.clone())).unwrap());
    cx.update(|cx| cx.set_global(SharedExecutor(exec)));
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    let view = dialog_on(cx, win);
    assert_eq!(
        view.read_with(cx, |v, _| v.footer()),
        "Loading saved cookies"
    );
    assert!(!view.read_with(cx, |v, _| v.editable()));
    click(cx, win, ("jar-delete", 0usize));
    click(cx, win, "jar-clear");
    click(cx, win, "jar-add");
    assert_eq!(jar.list().len(), 1);
    assert!(!present(cx, win, "jar-form"));

    jar.load();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.footer()),
        "Cookies are saved encrypted."
    );
    click(cx, win, ("jar-delete", 0usize));
    assert!(jar.list().is_empty());
}

#[gpui_kit::test]
fn volatile_secrets_show_the_session_only_footer(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_global(VolatileSecrets));
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let view = dialog_on(cx, s.win);
    assert_eq!(
        view.read_with(cx, |v, _| v.footer()),
        "Cookies are kept for this session only: cookies are not saved"
    );
    assert!(present(cx, s.win, "jar-footer"));
}

/// The locked fields of an edit form go through `ui::lock_input`: read-only in every theme.
/// Value stays editable in all.
#[gpui_kit::test]
fn the_edit_form_locks_identity_fields_through_lock_input(cx: &mut TestAppContext) {
    use apinata_app::theme::{DARK, HC_DARK, HC_LIGHT, LIGHT};
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started_with_store(cx, &col, store);
    let view = dialog_on(cx, s.win);
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-domain", "app.test");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    replace_input(cx, s.win, None, "jar-form-value", "first");
    click(cx, s.win, "jar-form-save");
    cx.run_until_parked();
    click(cx, s.win, ("jar-edit", 0usize));
    for theme in [HC_LIGHT, HC_DARK, LIGHT, DARK] {
        use_theme(cx, theme);
        frame(cx, s.win);
        let state = |cx: &mut TestAppContext, f: CookieField| {
            view.read_with(cx, |v, cx| {
                let p = v.form_input(f).unwrap().read(cx).presentation();
                (p.is_readonly(), p.is_disabled())
            })
        };
        for f in [CookieField::Domain, CookieField::Path, CookieField::Name] {
            assert_eq!(state(cx, f), (true, false), "{theme} {f:?}");
        }
        assert_eq!(
            state(cx, CookieField::Value),
            (false, false),
            "{theme} Value"
        );
    }
}

fn jar_label(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) -> String {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).label().unwrap_or_default().to_owned()
    })
    .unwrap()
}

#[gpui_kit::test]
fn jar_values_stay_masked_until_their_eye_is_clicked(cx: &mut TestAppContext) {
    let (win, _exec, _view) = dialog(cx, true);
    let value = |cx: &mut TestAppContext, ix: usize| jar_label(cx, win, ("jar-value", ix));
    assert_eq!(value(cx, 0), apinata_app::response_pane::NAME_MASK);
    assert_eq!(value(cx, 1), apinata_app::response_pane::NAME_MASK);
    click(cx, win, ("jar-reveal", 0usize));
    frame(cx, win);
    assert_eq!(value(cx, 0), "S1");
    assert_eq!(value(cx, 1), apinata_app::response_pane::NAME_MASK);
    assert!(present(cx, win, ("jar-hide", 0usize)));
    click(cx, win, ("jar-hide", 0usize));
    frame(cx, win);
    assert_eq!(value(cx, 0), apinata_app::response_pane::NAME_MASK);
}

#[gpui_kit::test]
fn editing_a_masked_cookie_works_on_the_real_value(cx: &mut TestAppContext) {
    let (win, exec, view) = dialog(cx, true);
    let exec = exec.unwrap();
    let stored = |exec: &Executor| -> Vec<String> {
        exec.cookies().list().into_iter().map(|c| c.value).collect()
    };
    click(cx, win, ("jar-edit", 0usize));
    let shown = view.read_with(cx, |v, cx| {
        v.form_input(CookieField::Value)
            .unwrap()
            .read(cx)
            .value()
            .to_string()
    });
    assert_eq!(shown, "S1");
    click(cx, win, "jar-form-save");
    cx.run_until_parked();
    assert!(stored(&exec).contains(&"S1".to_owned()));
    click(cx, win, ("jar-edit", 0usize));
    replace_input(cx, win, None, "jar-form-value", "S2");
    click(cx, win, "jar-form-save");
    cx.run_until_parked();
    assert!(stored(&exec).contains(&"S2".to_owned()));
}

/// The dialog over a fresh executor holding exactly `cookies`.
fn dialog_over(
    cx: &mut TestAppContext,
    cookies: Vec<CookieInfo>,
) -> (AnyWindowHandle, Arc<Executor>) {
    support::init(cx);
    let exec = Arc::new(Executor::new(ExecOptions::new()).unwrap());
    for c in cookies {
        exec.cookies().insert(c).unwrap();
    }
    cx.update(|cx| cx.set_global(SharedExecutor(exec.clone())));
    let (win, _) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank)))
        .unwrap();
    cx.update_window(win, |_, w, cx| open_cookie_jar(w, cx))
        .unwrap();
    frame(cx, win);
    (win, exec)
}

#[gpui_kit::test]
fn an_empty_value_shows_empty_with_no_eye(cx: &mut TestAppContext) {
    let (win, _exec) = dialog_over(cx, vec![CookieInfo::new("a.test", "blank", "")]);
    assert_eq!(jar_label(cx, win, ("jar-value", 0usize)), "");
    assert!(!present(cx, win, ("jar-reveal", 0usize)));
    assert!(!present(cx, win, ("jar-hide", 0usize)));
}

#[gpui_kit::test]
fn a_long_revealed_value_keeps_the_row_buttons_inside_the_dialog(cx: &mut TestAppContext) {
    let (win, _exec) = dialog_over(cx, vec![CookieInfo::new("a.test", "big", "x".repeat(300))]);
    click(cx, win, ("jar-reveal", 0usize));
    frame(cx, win);
    let (jar, edit, delete) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("cookie-jar").bounds(),
                w.find(("jar-edit", 0usize)).bounds(),
                w.find(("jar-delete", 0usize)).bounds(),
            )
        })
        .unwrap();
    assert!(edit.right() <= jar.right() && delete.right() <= jar.right());
}

// Each error line starts with a capital, a bad domain's included, as other alerts do.
#[gpui_kit::test]
fn a_bad_domain_error_line_starts_with_a_capital(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let view = dialog_on(cx, s.win);
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-domain", "a b");
    replace_input(cx, s.win, None, "jar-form-name", "a b");
    replace_input(cx, s.win, None, "jar-form-value", "v");
    click(cx, s.win, "jar-form-save");
    let error = form_error(cx, &view).expect("an error");
    assert!(error.starts_with("Invalid cookie domain"), "{error}");
    for line in error.lines() {
        assert!(line.starts_with(char::is_uppercase), "{line}");
    }
}

#[gpui_kit::test]
fn every_invalid_cookie_field_is_marked_not_only_the_first(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let view = dialog_on(cx, s.win);
    click(cx, s.win, "jar-add");
    // A bad domain and a bad expiry together, then a bad value and a bad expiry.
    replace_input(cx, s.win, None, "jar-form-domain", "a b");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    replace_input(cx, s.win, None, "jar-form-value", "v");
    replace_input(cx, s.win, None, "jar-form-expires", "soon");
    click(cx, s.win, "jar-form-save");
    for id in ["jar-form-domain", "jar-form-expires"] {
        assert!(present(cx, s.win, format!("{id}-invalid-icon")), "{id}");
    }
    for id in ["jar-form-path", "jar-form-name", "jar-form-value"] {
        assert!(!present(cx, s.win, format!("{id}-invalid-icon")), "{id}");
    }
    assert!(form_error(cx, &view).is_some());
    replace_input(cx, s.win, None, "jar-form-domain", "a.test");
    replace_input(cx, s.win, None, "jar-form-value", "a;b");
    click(cx, s.win, "jar-form-save");
    for id in ["jar-form-value", "jar-form-expires"] {
        assert!(present(cx, s.win, format!("{id}-invalid-icon")), "{id}");
    }
    assert!(!present(cx, s.win, "jar-form-domain-invalid-icon"));
}

#[gpui_kit::test]
fn enter_on_a_focused_delete_button_deletes_that_cookie(cx: &mut TestAppContext) {
    let (win, exec, _view) = dialog(cx, true);
    let exec = exec.unwrap();
    cx.update(apinata_app::keymap::init);
    enter_on(cx, win, ("jar-delete", 0usize));
    frame(cx, win);
    let names: Vec<String> = exec.cookies().list().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["theme"], "Enter pressed Delete");
    assert!(
        cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap(),
        "the jar stays open"
    );
}

// Enter on a focused cookie eye flips the value, and focus stays on the eye both ways.
#[gpui_kit::test]
fn enter_on_the_cookie_eye_keeps_focus(cx: &mut TestAppContext) {
    let (win, _exec, _view) = dialog(cx, true);
    cx.update(apinata_app::keymap::init);
    assert!(present(cx, win, ("jar-reveal", 0usize)));
    enter_on(cx, win, ("jar-eye", 0usize));
    frame(cx, win);
    assert_eq!(jar_label(cx, win, ("jar-value", 0usize)), "S1");
    assert!(present(cx, win, ("jar-hide", 0usize)));
    assert!(
        holds_focus(cx, win, ("jar-eye", 0usize)),
        "the flipped eye holds focus"
    );
    press_key(cx, win, "enter");
    assert_eq!(
        jar_label(cx, win, ("jar-value", 0usize)),
        apinata_app::response_pane::NAME_MASK
    );
    assert!(
        holds_focus(cx, win, ("jar-eye", 0usize)),
        "focus stays through the flip back"
    );
}

#[gpui_kit::test]
fn enter_on_the_focused_form_cancel_closes_only_the_form(cx: &mut TestAppContext) {
    let (win, _exec, _view) = dialog(cx, true);
    cx.update(apinata_app::keymap::init);
    click(cx, win, "jar-add");
    assert!(present(cx, win, "jar-form"));
    enter_on(cx, win, "jar-form-cancel");
    frame(cx, win);
    assert!(!present(cx, win, "jar-form"), "Enter pressed Cancel");
    assert!(present(cx, win, "cookie-jar"), "the jar stays open");
}

fn jar_open(cx: &mut TestAppContext, win: AnyWindowHandle) -> bool {
    cx.update_window(win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn press(cx: &mut TestAppContext, win: AnyWindowHandle, key: &str) {
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
    frame(cx, win);
}

fn listed_names(cx: &mut TestAppContext) -> Vec<String> {
    let exec = cx.update(|cx| shared(cx)).unwrap();
    exec.cookies().list().into_iter().map(|c| c.name).collect()
}

// Fills a valid new cookie and leaves the caret in the Expires field.
fn fill_valid_form(cx: &mut TestAppContext, win: AnyWindowHandle) {
    click(cx, win, "jar-add");
    replace_input(cx, win, None, "jar-form-domain", "app.test");
    replace_input(cx, win, None, "jar-form-name", "sid");
    replace_input(cx, win, None, "jar-form-value", "first");
    type_at_end(cx, win, None, "jar-form-expires", "");
    frame(cx, win);
}

#[gpui_kit::test]
fn enter_in_a_valid_form_field_saves_the_cookie_and_keeps_the_jar_open(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let _view = dialog_on(cx, s.win);
    fill_valid_form(cx, s.win);
    press(cx, s.win, "enter");
    assert_eq!(listed_names(cx), ["sid"], "Enter saved the cookie");
    assert!(!present(cx, s.win, "jar-form"), "the form closes on save");
    assert!(jar_open(cx, s.win), "the jar stays open");
}

#[gpui_kit::test]
fn escape_after_an_enter_save_closes_the_jar(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let _view = dialog_on(cx, s.win);
    fill_valid_form(cx, s.win);
    press(cx, s.win, "enter");
    assert!(jar_open(cx, s.win));
    press(cx, s.win, "escape");
    assert!(!jar_open(cx, s.win), "Escape closes the jar");
}

#[gpui_kit::test]
fn enter_in_an_invalid_form_shows_the_error_and_saves_nothing(cx: &mut TestAppContext) {
    let col = write_tree(&[("opencollection.yml", ROOT)]);
    let s = started(cx, &col);
    let view = dialog_on(cx, s.win);
    click(cx, s.win, "jar-add");
    replace_input(cx, s.win, None, "jar-form-name", "sid");
    replace_input(cx, s.win, None, "jar-form-value", "v");
    type_at_end(cx, s.win, None, "jar-form-domain", "");
    press(cx, s.win, "enter");
    assert!(form_error(cx, &view).is_some(), "the form shows its error");
    assert!(present(cx, s.win, "jar-form-domain-error") && present(cx, s.win, "jar-form"));
    assert!(jar_open(cx, s.win), "the jar stays open");
    assert!(listed_names(cx).is_empty());
}

#[gpui_kit::test]
fn enter_in_the_search_field_keeps_the_jar_open(cx: &mut TestAppContext) {
    let (win, exec, _view) = dialog(cx, true);
    let exec = exec.unwrap();
    cx.update(apinata_app::keymap::init);
    type_at_end(cx, win, None, "jar-search", "example");
    press(cx, win, "enter");
    assert!(jar_open(cx, win), "the jar stays open");
    assert_eq!(exec.cookies().list().len(), 2, "nothing changed");
}

#[gpui_kit::test]
fn escape_closes_the_jar(cx: &mut TestAppContext) {
    let (win, _exec, _view) = dialog(cx, true);
    cx.update(apinata_app::keymap::init);
    type_at_end(cx, win, None, "jar-search", "x");
    press(cx, win, "escape");
    assert!(!jar_open(cx, win));
}

fn checked(cx: &mut TestAppContext, win: AnyWindowHandle, id: &'static str) -> Option<bool> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).checked()
    })
    .unwrap()
}

#[gpui_kit::test]
fn enter_on_a_focused_form_checkbox_toggles_it_and_keeps_the_jar(cx: &mut TestAppContext) {
    let (win, _exec, _view) = dialog(cx, true);
    cx.update(apinata_app::keymap::init);
    click(cx, win, "jar-add");
    assert_eq!(checked(cx, win, "jar-form-http-only"), Some(false));
    enter_on(cx, win, "jar-form-http-only");
    frame(cx, win);
    assert!(present(cx, win, "jar-form"), "the form stays open");
    assert_eq!(
        checked(cx, win, "jar-form-http-only"),
        Some(true),
        "Enter toggled HttpOnly"
    );
}

fn console_lines(cx: &mut TestAppContext) -> Vec<(String, apinata_core::ConsoleLevel, String)> {
    cx.update(|cx| {
        apinata_app::log_feed::ensure(cx)
            .console_since(0)
            .entries
            .iter()
            .map(|l| (l.request.clone(), l.level, l.text.clone()))
            .collect()
    })
}

#[gpui_kit::test]
fn dropped_saved_cookies_become_one_console_warning(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| apinata_app::app::note_dropped_cookies(2, cx));
    assert_eq!(
        console_lines(cx),
        [(
            "Cookies".to_owned(),
            apinata_core::ConsoleLevel::Warn,
            "2 saved cookies were dropped: their domains are public suffixes, so they would reach every site under them.".to_owned()
        )]
    );
}

#[gpui_kit::test]
fn no_dropped_cookies_log_nothing(cx: &mut TestAppContext) {
    support::init(cx);
    cx.update(|cx| apinata_app::app::note_dropped_cookies(0, cx));
    assert!(console_lines(cx).is_empty());
}

/// Seals `cookies` (saved-cookie JSON objects) into `path` the way the jar's file is sealed,
/// its key stored in `store`, so a load can read cookies the jar itself would never save.
/// Copies apinata-core sealed.rs `FileBody` and `FILE_VERSION`, keyed by cookies.rs `KEY_NAME`.
fn seal_cookies(path: &std::path::Path, store: &MemoryStore, cookies: &str) {
    use aes_gcm::aead::{Aead, KeyInit};
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let key = [7u8; 32];
    let name = apinata_core::SecretKey::new("", "", "cookie-jar-key");
    store.set(&name, &b64.encode(key)).unwrap();
    let nonce = [3u8; 12];
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key).unwrap();
    let data = cipher
        .encrypt(
            &aes_gcm::Nonce::try_from(&nonce[..]).unwrap(),
            cookies.as_bytes(),
        )
        .unwrap();
    let body = serde_json::json!({
        "version": 1,
        "nonce": b64.encode(nonce),
        "data": b64.encode(data),
    });
    std::fs::write(path, body.to_string()).unwrap();
}

/// Opens the file `seal_cookies` wrote, after the jar may have rewritten it with the same key.
fn unseal_cookies(path: &std::path::Path) -> String {
    use aes_gcm::aead::{Aead, KeyInit};
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let body: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let field = |k: &str| b64.decode(body[k].as_str().unwrap()).unwrap();
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(&[7u8; 32]).unwrap();
    let nonce = field("nonce");
    let plain = cipher
        .decrypt(
            &aes_gcm::Nonce::try_from(&nonce[..]).unwrap(),
            field("data").as_ref(),
        )
        .unwrap();
    String::from_utf8(plain).unwrap()
}

#[gpui_kit::test]
fn loading_the_saved_jar_notes_its_dropped_cookies_once(cx: &mut TestAppContext) {
    support::init(cx);
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("cookies.json");
    let store = Arc::new(MemoryStore::new());
    let entry = |domain: &str, name: &str, host_only: bool| {
        format!(
            r#"{{"domain":"{domain}","path":"/","name":"{name}","value":"v","expires":null,"secure":false,"httpOnly":false,"hostOnly":{host_only}}}"#
        )
    };
    let cookies = format!(
        "[{},{}]",
        entry("co.uk", "wide", false),
        entry("a.test", "kept", true)
    );
    seal_cookies(&path, &store, &cookies);
    let jar = Arc::new(CookieJar::open(path, store));
    cx.update(|cx| apinata_app::app::load_saved_cookies(jar.clone(), cx));
    cx.run_until_parked();
    let notice = (
        "Cookies".to_owned(),
        apinata_core::ConsoleLevel::Warn,
        "1 saved cookie was dropped: its domain is a public suffix, so it would reach every site under it.".to_owned(),
    );
    assert_eq!(console_lines(cx), std::slice::from_ref(&notice));
    let names: Vec<String> = jar.list().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["kept"]);
    cx.update(|cx| apinata_app::app::load_saved_cookies(jar.clone(), cx));
    cx.run_until_parked();
    assert_eq!(console_lines(cx), [notice]);
}

// Two app contexts stand for two launches over one jar file: the first drops and warns, and
// must rewrite the file so the second finds nothing to drop and stays quiet.
#[gpui_kit::test]
fn a_dropped_cookie_warns_on_the_first_launch_only(
    first: &mut TestAppContext,
    second: &mut TestAppContext,
) {
    support::init(first);
    support::init(second);
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("cookies.json");
    let store = Arc::new(MemoryStore::new());
    seal_cookies(
        &path,
        &store,
        r#"[{"domain":"co.uk","path":"/","name":"wide","value":"v","expires":null,"secure":false,"httpOnly":false,"hostOnly":false},{"domain":"a.test","path":"/","name":"kept","value":"v","expires":null,"secure":false,"httpOnly":false,"hostOnly":true}]"#,
    );
    let launch = |cx: &mut TestAppContext| {
        let jar = Arc::new(CookieJar::open(path.clone(), store.clone()));
        cx.update(|cx| apinata_app::app::load_saved_cookies(jar.clone(), cx));
        cx.run_until_parked();
        jar
    };
    launch(first);
    assert_eq!(console_lines(first).len(), 1);
    let jar = launch(second);
    assert!(console_lines(second).is_empty());
    let names: Vec<String> = jar.list().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["kept"]);
    let on_disk = unseal_cookies(&path);
    assert!(
        !on_disk.contains("co.uk") && on_disk.contains("kept"),
        "{on_disk}"
    );
}

#[gpui_kit::test]
fn a_cookies_same_site_shows_in_its_flags_and_an_edit_keeps_it(cx: &mut TestAppContext) {
    let (win, exec, view) = dialog(cx, true);
    let exec = exec.unwrap();
    for (name, same_site) in [
        ("a-strict", Some(SameSite::Strict)),
        ("b-none", Some(SameSite::None)),
        ("c-unset", None),
    ] {
        let mut c = CookieInfo::new("same.test", name, "1");
        c.secure = name == "a-strict";
        c.same_site = same_site;
        exec.cookies().insert(c).unwrap();
    }
    view.update(cx, |v, cx| v.refresh(cx));
    frame(cx, win);
    let row = |cx: &mut TestAppContext, name: &str| {
        view.read_with(cx, |v, _| v.cookies().iter().position(|c| c.name == name))
            .expect("listed")
    };
    let flags = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("jar-flags", ix)).label().map(str::to_owned)
        })
        .unwrap()
    };
    let strict = row(cx, "a-strict");
    assert_eq!(
        flags(cx, strict).as_deref(),
        Some("HTTPS only, This host only, Same site: Strict")
    );
    let none = row(cx, "b-none");
    assert_eq!(
        flags(cx, none).as_deref(),
        Some("This host only, Same site: None")
    );
    let unset = row(cx, "c-unset");
    assert_eq!(flags(cx, unset).as_deref(), Some("This host only"));

    cx.update_window(win, |_, w, cx| w.click(("jar-edit", strict), cx))
        .unwrap();
    frame(cx, win);
    replace_input(cx, win, None, "jar-form-value", "2");
    click(cx, win, "jar-form-save");
    cx.run_until_parked();
    let edited = exec
        .cookies()
        .list()
        .into_iter()
        .find(|c| c.name == "a-strict")
        .expect("kept");
    assert_eq!(
        (edited.value.as_str(), edited.same_site),
        ("2", Some(SameSite::Strict))
    );
}

// An empty jar spaces its note apart from the search above and the footer below.
#[gpui_kit::test]
fn the_empty_jar_is_spaced_from_the_search_and_the_footer(cx: &mut TestAppContext) {
    use apinata_app::style::{SPACE_4, at_zoom};
    let (win, _exec, _view) = dialog(cx, true);
    cx.update_window(win, |_, w, cx| w.click(("jar-domain-delete", 0usize), cx))
        .unwrap();
    frame(cx, win);
    let (search, empty, footer) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("jar-search").bounds(),
                w.find("jar-empty").bounds(),
                w.find("jar-footer").bounds(),
            )
        })
        .unwrap();
    let room = at_zoom(SPACE_4, 1.0);
    assert!(
        empty.top() - search.bottom() >= room,
        "{search:?} {empty:?}"
    );
    assert!(
        footer.top() - empty.bottom() >= room,
        "{empty:?} {footer:?}"
    );
}
