//! Fixtures shared by the app's integration tests.
#![allow(dead_code)]

pub mod fake_trash;

use apinata_core::ItemPath;
use tempfile::TempDir;

/// Initializes gpui-kit with reduced motion. Every test window starts here: entrance
/// animations run on wall-clock time, so under load a click would land mid-slide.
pub fn init(cx: &mut gpui_kit::TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        apinata_app::theme::install(cx);
        cx.set_reduce_motion(true);
    });
}

pub const ROOT: &str = "opencollection: 1.0.0\ninfo:\n  name: c\n";

pub fn request(name: &str, url: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n"
    )
}

pub fn folder(name: &str) -> String {
    format!("info:\n  name: {name}\n  type: folder\n  seq: 1\n")
}

pub fn write_tree(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

pub fn ip(rel: &str) -> ItemPath {
    ItemPath::new(rel).unwrap()
}

use apinata_app::command::{Command, Edit, NoticeKind, Target};
use apinata_app::model::CollectionModel;
use apinata_app::snapshot::CollectionSnapshot;
use apinata_core::{MemoryStore, SecretStore};
use gpui_kit::{Entity, TestAppContext};
use std::sync::Arc;

pub fn open(cx: &mut TestAppContext, dir: &TempDir) -> Entity<CollectionModel> {
    open_with(cx, dir, Arc::new(MemoryStore::new()))
}

pub fn open_with(
    cx: &mut TestAppContext,
    dir: &TempDir,
    store: Arc<dyn SecretStore>,
) -> Entity<CollectionModel> {
    let root = dir.path().to_path_buf();
    let model = cx.update(|cx| CollectionModel::open(root, store, cx));
    cx.run_until_parked();
    model
}

pub fn send(cx: &mut TestAppContext, model: &Entity<CollectionModel>, command: Command) -> u64 {
    model.update(cx, |m, _| m.send(command))
}

pub fn snap(cx: &mut TestAppContext, model: &Entity<CollectionModel>) -> Arc<CollectionSnapshot> {
    model.read_with(cx, |m, _| m.snapshot().cloned().expect("opened"))
}

pub fn notices(cx: &mut TestAppContext, model: &Entity<CollectionModel>) -> Vec<NoticeKind> {
    model.update(cx, |m, _| {
        m.take_notices().into_iter().map(|n| n.kind).collect()
    })
}

pub fn edit_url(rel: &str, url: &str) -> Command {
    Command::Edit {
        target: Target::Request(ip(rel)),
        edit: Edit::Url(url.into()),
    }
}

pub fn url(s: &CollectionSnapshot, rel: &str) -> String {
    s.content.requests[&ip(rel)].url.clone()
}

use apinata_core::{SecretError, SecretKey};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// A store that counts reads, can block each one like a keychain prompt, and can refuse deletes.
pub struct CountingStore {
    pub inner: MemoryStore,
    pub gets: AtomicUsize,
    pub delay: Duration,
    pub fail_deletes: bool,
}

impl CountingStore {
    pub fn new(delay: Duration) -> CountingStore {
        CountingStore {
            inner: MemoryStore::new(),
            gets: AtomicUsize::new(0),
            delay,
            fail_deletes: false,
        }
    }
    pub fn gets(&self) -> usize {
        self.gets.load(Ordering::SeqCst)
    }
}

impl SecretStore for CountingStore {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        std::thread::sleep(self.delay);
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        if self.fail_deletes {
            return Err(SecretError::Unavailable {
                message: "locked".into(),
            });
        }
        self.inner.delete(key)
    }
}

pub fn session_at(
    cx: &mut gpui_kit::TestAppContext,
    dir: &TempDir,
    start: apinata_app::session::Session,
) -> gpui_kit::Entity<apinata_app::persisted::Persisted<apinata_app::session::Session>> {
    use gpui_kit::AppContext;
    let path = dir.path().join("session.json");
    cx.new(|_| apinata_app::persisted::Persisted::new(start, path))
}

pub fn session_value(
    cx: &mut gpui_kit::TestAppContext,
    session: &gpui_kit::Entity<apinata_app::persisted::Persisted<apinata_app::session::Session>>,
) -> apinata_app::session::Session {
    session.read_with(cx, |p, _| p.get().clone())
}

pub fn workspace(
    cx: &mut gpui_kit::TestAppContext,
    session: &gpui_kit::Entity<apinata_app::persisted::Persisted<apinata_app::session::Session>>,
) -> gpui_kit::Entity<apinata_app::workspace::Workspace> {
    use gpui_kit::AppContext;
    let session = session.clone();
    let store: std::sync::Arc<dyn apinata_core::SecretStore> =
        std::sync::Arc::new(apinata_core::MemoryStore::new());
    cx.new(|cx| apinata_app::workspace::Workspace::new(store, session, cx))
}

/// A collection of `n` requests, enough of them to overflow the sidebar.
pub fn long_collection(n: usize) -> TempDir {
    let mut files = vec![("opencollection.yml".to_owned(), ROOT.to_owned())];
    for i in 0..n {
        files.push((
            format!("r{i:02}.yml"),
            request(&format!("r{i:02}"), "http://x/0"),
        ));
    }
    let refs: Vec<(&str, &str)> = (files.iter())
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    write_tree(&refs)
}

pub fn collection() -> TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ])
}

pub fn events(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
) -> std::rc::Rc<std::cell::RefCell<Vec<apinata_app::workspace::WorkspaceEvent>>> {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.subscribe(
            ws,
            move |_, e: &apinata_app::workspace::WorkspaceEvent, _| {
                sink.borrow_mut().push(e.clone())
            },
        )
        .detach()
    });
    seen
}

pub const SOCKET: &str = "info:\n  name: Socket\n  type: websocket\n  seq: 2\n";

pub fn sidebar_files() -> Vec<(&'static str, String)> {
    vec![
        ("opencollection.yml", ROOT.to_string()),
        ("users/folder.yml", folder("Users")),
        ("users/list.yml", request("List users", "http://x/users")),
        ("health.yml", request("Health", "http://x/health")),
        ("status.yml", request("Status", "http://x/status")),
        ("ws.yml", SOCKET.to_string()),
    ]
}

pub struct SidebarHarness {
    pub col: TempDir,
    pub state: TempDir,
    pub root: std::path::PathBuf,
    pub win: gpui_kit::AnyWindowHandle,
    pub ws: gpui_kit::Entity<apinata_app::workspace::Workspace>,
    pub view: gpui_kit::Entity<apinata_app::sidebar::Sidebar>,
}

pub fn open_sidebar(cx: &mut gpui_kit::TestAppContext) -> SidebarHarness {
    open_sidebar_with(cx, sidebar_files())
}

pub fn open_sidebar_with(
    cx: &mut gpui_kit::TestAppContext,
    files: Vec<(&'static str, String)>,
) -> SidebarHarness {
    open_sidebar_trash(cx, files, None)
}

/// `open_sidebar_with` over a workspace that deletes into `trash`, when given one.
pub fn open_sidebar_trash(
    cx: &mut gpui_kit::TestAppContext,
    files: Vec<(&'static str, String)>,
    trash: Option<std::sync::Arc<dyn apinata_core::Trash>>,
) -> SidebarHarness {
    use gpui_kit::AppContext;
    init(cx);
    cx.update(apinata_app::keymap::init);
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let col = write_tree(&refs);
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    if let Some(trash) = trash {
        ws.update(cx, |w, _| w.set_trash(trash));
    }
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| apinata_app::sidebar::Sidebar::new(shared, window, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    SidebarHarness {
        col,
        state,
        root,
        win,
        ws,
        view,
    }
}

pub fn frame(cx: &mut gpui_kit::TestAppContext, win: gpui_kit::AnyWindowHandle) {
    use gpui_kit::AppContext;
    use gpui_kit::test::TestWindowExt;
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
}

pub fn row(path: &str) -> gpui_kit::SharedString {
    apinata_app::sidebar::model::node_id(0, &ip(path))
}

pub fn shown(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    use gpui_kit::AppContext;
    use gpui_kit::test::TestWindowExt;
    frame(cx, win);
    let id = id.into();
    cx.update_window(win, |_, w, _| w.try_find(id).is_some())
        .unwrap()
}

pub fn sidebar_events(
    cx: &mut gpui_kit::TestAppContext,
    view: &gpui_kit::Entity<apinata_app::sidebar::Sidebar>,
) -> std::rc::Rc<std::cell::RefCell<Vec<apinata_app::sidebar::SidebarEvent>>> {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = seen.clone();
    cx.update(|cx| {
        cx.subscribe(view, move |_, e: &apinata_app::sidebar::SidebarEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    seen
}

pub fn secondary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

pub fn focus_tree(cx: &mut gpui_kit::TestAppContext, h: &SidebarHarness) {
    use gpui_kit::AppContext;
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    cx.update_window(h.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
}

pub fn two_requests() -> TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("b.yml", &request("b", "http://x/b")),
    ])
}

pub fn open_dir(
    cx: &mut gpui_kit::TestAppContext,
    col: &TempDir,
) -> (
    TempDir,
    gpui_kit::Entity<apinata_app::workspace::Workspace>,
    std::path::PathBuf,
) {
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    let root = col.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    (state, ws, root)
}

pub fn open_tab(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    rel: &str,
) -> apinata_app::tabs::TabId {
    ws.update(cx, |w, cx| w.open_item(root, ip(rel), false, cx))
        .expect("a request")
}

pub fn tab_names(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
) -> Vec<String> {
    ws.read_with(cx, |w, _| {
        w.tabs()
            .entries()
            .iter()
            .filter_map(|e| e.item())
            .map(|p| p.as_str().to_owned())
            .collect()
    })
}

pub fn editor_for(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    rel: &str,
    preview: bool,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    init(cx);
    cx.update(apinata_app::keymap::init);
    let tab = ws
        .update(cx, |w, cx| w.open_item(root, ip(rel), preview, cx))
        .expect("a request");
    let ws = ws.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::request_editor::RequestEditor::new(ws, tab, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, view)
}

pub fn type_at_end(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    scope: Option<&'static str>,
    id: impl Into<gpui_kit::ElementId>,
    text: &str,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        match scope {
            Some(s) => w.within(s).click(id, cx),
            None => w.click(id, cx),
        }
        w.press("end", cx);
        w.input(text, cx);
    })
    .unwrap();
}

pub fn view_of(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    rel: &str,
) -> std::sync::Arc<apinata_app::snapshot::RequestView> {
    ws.read_with(cx, |w, cx| w.request(root, &ip(rel), cx))
        .expect("a request")
}

pub fn key(k: &str) -> String {
    secondary(k)
}

pub struct Area {
    pub col: TempDir,
    pub state: TempDir,
    pub ws: gpui_kit::Entity<apinata_app::workspace::Workspace>,
    pub root: std::path::PathBuf,
    pub win: gpui_kit::AnyWindowHandle,
    pub view: gpui_kit::Entity<apinata_app::tab_area::TabArea>,
}

pub fn open_area(cx: &mut gpui_kit::TestAppContext) -> Area {
    open_area_of(cx, two_requests())
}

pub fn open_area_of(cx: &mut gpui_kit::TestAppContext, col: TempDir) -> Area {
    use gpui_kit::AppContext as _;
    let (state, ws, root) = open_dir(cx, &col);
    init(cx);
    let shared = ws.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::tab_area::TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    Area {
        col,
        state,
        ws,
        root,
        win,
        view,
    }
}

pub fn present(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).is_some()
    })
    .unwrap()
}

/// Presses Tab (a full key press) until the element `id` holds focus, at most 64 times (a
/// long form such as Preferences has many stops), then presses Enter on it: a keyboard
/// user's way to activate one dialog button.
pub fn enter_on(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    key_on(cx, win, id, "enter");
}

/// `enter_on` with any `key`: Tabs to element `id`, then presses `key` on it.
pub fn key_on(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
    key: &str,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id: gpui_kit::ElementId = id.into();
    let on = |cx: &mut gpui_kit::TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(id.clone()).focused()
        })
        .unwrap()
            == Some(true)
    };
    for _ in 0..64 {
        if on(cx) {
            break;
        }
        cx.update_window(win, |_, w, cx| w.press("tab", cx))
            .unwrap();
    }
    assert!(on(cx), "Tab reaches {id}");
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    cx.run_until_parked();
}

/// Whether element `id` holds focus in a freshly drawn frame; false when it is not drawn.
pub fn holds_focus(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.focused())
    })
    .unwrap()
        == Some(true)
}

/// A full press of `key` in `win`, then a drawn frame once the app settles.
pub fn press_key(cx: &mut gpui_kit::TestAppContext, win: gpui_kit::AnyWindowHandle, key: &str) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| w.press(key, cx)).unwrap();
    frame(cx, win);
}

pub fn wait_for(
    cx: &mut gpui_kit::TestAppContext,
    what: &str,
    mut done: impl FnMut(&mut gpui_kit::TestAppContext) -> bool,
) {
    let start = std::time::Instant::now();
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Four debounce windows: whatever the watcher was going to deliver has been applied.
pub fn settle(cx: &mut gpui_kit::TestAppContext) {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(1000);
    while std::time::Instant::now() < until {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    cx.run_until_parked();
}

pub fn ws_snap(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
) -> std::sync::Arc<apinata_app::snapshot::CollectionSnapshot> {
    ws.read_with(cx, |w, cx| w.snapshot(root, cx).expect("opened"))
}

pub struct Watched {
    pub state: TempDir,
    pub ws: gpui_kit::Entity<apinata_app::workspace::Workspace>,
    pub root: std::path::PathBuf,
}

/// Opens `dir` in a fresh workspace using `factory`, without waiting.
pub fn watching(
    cx: &mut gpui_kit::TestAppContext,
    dir: &TempDir,
    factory: Option<apinata_app::watch::set::WatchFactory>,
) -> Watched {
    cx.executor().allow_parking();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, cx| w.set_watch_factory(factory, cx));
    let root = dir.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    Watched { state, ws, root }
}

/// As `watching` with the real watcher, returned once the first gap reload (snapshot 2) has landed.
pub fn watched(cx: &mut gpui_kit::TestAppContext, dir: &TempDir) -> Watched {
    let w = watching(cx, dir, Some(apinata_app::watch::set::notify_factory()));
    wait_for(cx, "watching to start", |cx| {
        ws_snap(cx, &w.ws, &w.root).seq >= 2
    });
    w
}

struct FailOn {
    inner: Box<dyn apinata_app::watch::set::Watch>,
    dir: std::path::PathBuf,
}

impl apinata_app::watch::set::Watch for FailOn {
    fn watch(&mut self, dir: &std::path::Path) -> Result<(), apinata_app::watch::set::WatchError> {
        if dir == self.dir {
            return Err(apinata_app::watch::set::WatchError::Failed(
                "OS file watch limit reached.".into(),
            ));
        }
        self.inner.watch(dir)
    }
    fn unwatch(&mut self, dir: &std::path::Path) {
        self.inner.unwatch(dir)
    }
}

/// The real watcher, except that watching `dir` fails as the inotify limit does.
pub fn failing_on(dir: std::path::PathBuf) -> apinata_app::watch::set::WatchFactory {
    let real = apinata_app::watch::set::notify_factory();
    std::sync::Arc::new(move |handler| {
        let inner = real(handler)?;
        Ok(Box::new(FailOn {
            inner,
            dir: dir.clone(),
        }) as Box<dyn apinata_app::watch::set::Watch>)
    })
}

pub struct SectionHost<V: 'static> {
    pub view: gpui_kit::Entity<V>,
    pub edits: Vec<apinata_app::command::Edit>,
    /// The typing key each edit carried, in step with `edits`.
    pub typed: Vec<Option<(usize, u8)>>,
    /// Whether the view is in the tree; a hidden one is not drawn, as under another tab.
    pub shown: bool,
    /// A focus target outside the view, always drawn.
    pub elsewhere: gpui_kit::FocusHandle,
    _sub: gpui_kit::Subscription,
}

impl<V: gpui_kit::Render> gpui_kit::Render for SectionHost<V> {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{InteractiveElement as _, ParentElement as _, Styled as _};
        gpui_kit::div()
            .size_full()
            .flex()
            .flex_col()
            .child(gpui_kit::div().track_focus(&self.elsewhere))
            .children(self.shown.then(|| self.view.clone()))
    }
}

pub fn host_section<V>(
    cx: &mut gpui_kit::TestAppContext,
    build: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::Context<V>) -> V,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<SectionHost<V>>)
where
    V: gpui_kit::Render + gpui_kit::EventEmitter<apinata_app::section::SectionEdit>,
{
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    init(cx);
    let (win, host) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| {
                    let view = cx.new(|cx| build(w, cx));
                    let sub = cx.subscribe(
                        &view,
                        |h: &mut SectionHost<V>, _, e: &apinata_app::section::SectionEdit, _| {
                            h.edits.push(e.edit.clone());
                            h.typed.push(e.typed);
                        },
                    );
                    SectionHost {
                        view,
                        edits: Vec::new(),
                        typed: Vec::new(),
                        shown: true,
                        elsewhere: cx.focus_handle(),
                        _sub: sub,
                    }
                })
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, host)
}

/// Where tests click a code editor: its first line, since a click at the wrapper's centre misses it.
pub fn code_at() -> gpui_kit::Point<gpui_kit::Pixels> {
    gpui_kit::point(gpui_kit::px(200.), gpui_kit::px(10.))
}

/// Replace a code editor's text by typing. Typing `{` into an empty editor auto-closes it.
pub fn replace_code(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    scope: Option<&'static str>,
    id: impl Into<gpui_kit::ElementId>,
    text: &str,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        match scope {
            Some(s) => w.within(s).click_at(id, code_at(), cx),
            None => w.click_at(id, code_at(), cx),
        }
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        w.input(text, cx);
    })
    .unwrap();
}

/// Replace a single-line input's value by typing.
pub fn replace_input(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    scope: Option<&'static str>,
    id: impl Into<gpui_kit::ElementId>,
    text: &str,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        match scope {
            Some(s) => w.within(s).click(id, cx),
            None => w.click(id, cx),
        }
        w.press(&key("a"), cx);
        w.press("backspace", cx);
        w.input(text, cx);
    })
    .unwrap();
}

pub fn open_dir_with(
    cx: &mut gpui_kit::TestAppContext,
    col: &TempDir,
    store: std::sync::Arc<dyn apinata_core::SecretStore>,
) -> (
    TempDir,
    gpui_kit::Entity<apinata_app::workspace::Workspace>,
    std::path::PathBuf,
) {
    use gpui_kit::AppContext as _;
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = cx.new(|cx| apinata_app::workspace::Workspace::new(store, session, cx));
    let root = col.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    (state, ws, root)
}

pub fn env_editor(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<apinata_app::env_editor::EnvEditor>,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    init(cx);
    cx.update(apinata_app::keymap::init);
    let (ws, root) = (ws.clone(), root.to_path_buf());
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::env_editor::EnvEditor::new(ws, root, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    (win, view)
}

pub fn env_view(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
    root: &std::path::Path,
    name: &str,
) -> std::sync::Arc<apinata_app::snapshot::EnvView> {
    let s = ws
        .read_with(cx, |w, cx| w.snapshot(root, cx))
        .expect("open");
    s.content
        .environments
        .iter()
        .find(|e| e.name == name)
        .cloned()
        .expect("an environment")
}

/// Store `sid` and `theme` for 127.0.0.1 in `exec`'s jar through a real send.
pub fn fill_jar(exec: &apinata_core::Executor) {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let server = MockServer::start().await;
        let set = ResponseTemplate::new(200)
            .append_header("set-cookie", "sid=S1; Path=/")
            .append_header("set-cookie", "theme=dark; Path=/; Max-Age=3600");
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

/// Sends a GET to `<uri>/ok` through `exec` directly, on the app's tokio runtime.
pub fn send_direct(cx: &mut gpui_kit::TestAppContext, exec: &apinata_core::Executor, uri: &str) {
    let col = collection_for(uri, "");
    let c = apinata_core::Collection::open(col.path()).unwrap();
    let ctx = apinata_core::VarContext::new(&c, apinata_core::ContextOptions::new()).unwrap();
    let req = apinata_core::resolve(c.request(&ip("a.yml")).unwrap(), &ctx).unwrap();
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let _ = handle.block_on(exec.send(&req));
}

/// The default settings with the proxy Off, so no test send follows an ambient `HTTP_PROXY`.
pub fn test_settings() -> apinata_app::settings::Settings {
    let mut settings = apinata_app::settings::Settings::default();
    settings.proxy.mode = apinata_app::settings::ProxyMode::Off;
    settings
}

pub fn http_ready(cx: &mut gpui_kit::TestAppContext) {
    cx.executor().allow_parking();
    cx.update(apinata_app::tokio_bridge::init);
    let store = MemoryStore::new();
    cx.update(|cx| apinata_app::executor::install(&test_settings(), &store, cx))
        .unwrap();
}

pub fn mock_server(
    cx: &mut gpui_kit::TestAppContext,
    response: wiremock::ResponseTemplate,
) -> wiremock::MockServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    // A dedicated server, not one from wiremock's process-wide pool: a pooled server is
    // handed to the next test with the previous test's in-flight requests still arriving.
    let server = handle.block_on(wiremock::MockServer::builder().start());
    handle.block_on(
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(response)
            .mount(&server),
    );
    server
}

#[path = "../../../apinata-core/tests/common/sse_server.rs"]
pub mod sse_server;

#[path = "../../../apinata-core/tests/common/body_server.rs"]
pub mod body_server;

/// Starts a raw body server on a free 127.0.0.1 port, on the app's tokio runtime.
pub fn bodies(
    cx: &mut gpui_kit::TestAppContext,
    routes: Vec<(&str, body_server::Reply)>,
) -> body_server::BodyServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(body_server::body_server(routes))
}

/// Starts `script` on a free 127.0.0.1 port, on the app's tokio runtime.
pub fn sse(
    cx: &mut gpui_kit::TestAppContext,
    script: sse_server::Script,
) -> sse_server::SlowServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(sse_server::slow_sse_server(script))
}

/// Starts the QA server (`/large` and `/fast` routes) on the app's tokio runtime.
pub fn qa_sse(cx: &mut gpui_kit::TestAppContext) -> sse_server::SlowServer {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(sse_server::qa_sse_server())
}

/// `collection_for` aimed at `server`.
pub fn sse_collection(server: &sse_server::SlowServer) -> TempDir {
    collection_for(&format!("http://127.0.0.1:{}", server.port()), "")
}

#[path = "../../../apinata-cli/tests/common/ws_server.rs"]
pub mod ws_server;

#[path = "../../../apinata-core/tests/common/raw_server.rs"]
pub mod raw_server;

/// A GraphQL-over-WebSocket peer on the app's tokio runtime: replies with subprotocol
/// `protocol`, acks, and after the subscribe sends `frames`, `gap` apart. Returns its port.
pub fn ws_peer(
    cx: &mut gpui_kit::TestAppContext,
    protocol: &'static str,
    frames: Vec<String>,
    gap: std::time::Duration,
) -> u16 {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(ws_server::ws_server(protocol, frames, gap))
}

/// A raw server answering each request head with `reply`, on the app's runtime. Returns its port.
pub fn raw_peer(cx: &mut gpui_kit::TestAppContext, reply: &'static str) -> u16 {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    handle.block_on(raw_server::raw_server(reply))
}

/// Request file `a`: a graphql item whose body is `subscription { tick }`, sent to `url`.
pub fn subscription_item(url: &str) -> String {
    format!(
        "info:\n  name: a\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"{url}\"\n  body:\n    query: \"subscription {{ tick }}\"\n"
    )
}

/// A collection whose `a.yml` is `subscription_item("http://127.0.0.1:<port>/graphql")`.
pub fn subscription_collection(port: u16) -> TempDir {
    let a = subscription_item(&format!("http://127.0.0.1:{port}/graphql"));
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

/// A response pane in its own window over a fresh send model for `col`:
/// (window, pane, model, collection root).
pub fn pane_for(
    cx: &mut gpui_kit::TestAppContext,
    col: &TempDir,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<apinata_app::response_pane::ResponsePane>,
    gpui_kit::Entity<apinata_app::send_model::SendModel>,
    std::path::PathBuf,
) {
    use gpui_kit::AppContext as _;
    let (_state, ws, root) = open_dir(cx, col);
    let model = sender(cx, &ws);
    init(cx);
    let m = model.clone();
    let (win, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| apinata_app::response_pane::ResponsePane::new(m, w, cx))
            })
        })
        .unwrap();
    (win, pane, model, root)
}

pub fn served(cx: &mut gpui_kit::TestAppContext, server: &wiremock::MockServer) -> Vec<String> {
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let requests = handle
        .block_on(server.received_requests())
        .unwrap_or_default();
    requests.iter().map(|r| r.url.path().to_owned()).collect()
}

// Real sockets finish on tokio threads, so poll against a wall-clock deadline.
#[track_caller]
pub fn wait_real(
    cx: &mut gpui_kit::TestAppContext,
    mut done: impl FnMut(&mut gpui_kit::TestAppContext) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting on real I/O at {}",
            std::panic::Location::caller()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A collection whose `a.yml` targets `<uri>/ok`, with `extra` appended to the request file.
pub fn collection_for(uri: &str, extra: &str) -> TempDir {
    let a = format!("{}{extra}", request("a", &format!("{uri}/ok")));
    write_tree(&[("opencollection.yml", ROOT), ("a.yml", &a)])
}

pub fn sender(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
) -> gpui_kit::Entity<apinata_app::send_model::SendModel> {
    use gpui_kit::AppContext as _;
    let ws = ws.clone();
    cx.new(|_| apinata_app::send_model::SendModel::new(ws))
}

pub fn finished(
    cx: &mut gpui_kit::TestAppContext,
    model: &gpui_kit::Entity<apinata_app::send_model::SendModel>,
) -> std::sync::Arc<apinata_core::RequestResult> {
    use apinata_app::send_model::SendState;
    wait_real(cx, |cx| model.read_with(cx, |m, _| !m.is_sending()));
    model.read_with(cx, |m, _| match m.state() {
        SendState::Done(r) => r.clone(),
        _ => panic!("no result"),
    })
}

pub struct Sent {
    pub server: wiremock::MockServer,
    pub col: TempDir,
    pub state: TempDir,
    pub ws: gpui_kit::Entity<apinata_app::workspace::Workspace>,
    pub root: std::path::PathBuf,
    pub model: gpui_kit::Entity<apinata_app::send_model::SendModel>,
    pub result: std::sync::Arc<apinata_core::RequestResult>,
}

/// Serves `response`, sends `a.yml` once and waits for its result.
pub fn send_once(
    cx: &mut gpui_kit::TestAppContext,
    response: wiremock::ResponseTemplate,
    extra: &str,
) -> Sent {
    http_ready(cx);
    let server = mock_server(cx, response);
    let col = collection_for(&server.uri(), extra);
    let (state, ws, root) = open_dir(cx, &col);
    let model = sender(cx, &ws);
    let r = root.clone();
    assert!(model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    let result = finished(cx, &model);
    Sent {
        server,
        col,
        state,
        ws,
        root,
        model,
        result,
    }
}

pub struct Started {
    pub files: TempDir,
    pub root: std::path::PathBuf,
    pub win: gpui_kit::AnyWindowHandle,
    pub shell: gpui_kit::Entity<apinata_app::shell::Shell>,
    pub ws: gpui_kit::Entity<apinata_app::workspace::Workspace>,
}

/// The real `app::start` over a temporary settings directory, with `col` open.
pub fn started(cx: &mut gpui_kit::TestAppContext, col: &TempDir) -> Started {
    started_with(cx, col, |_| {})
}

/// `started`, with `prepare` run on the temporary paths before `app::start` reads them.
pub fn started_with(
    cx: &mut gpui_kit::TestAppContext,
    col: &TempDir,
    prepare: impl FnOnce(&apinata_app::persist::Paths),
) -> Started {
    let store = std::sync::Arc::new(apinata_core::MemoryStore::new());
    started_on(cx, col, prepare, store)
}

/// Marks the collection at `root` as allowed to run scripts in `paths`' script trust file.
pub fn allow_scripts(paths: &apinata_app::persist::Paths, root: &std::path::Path) {
    let mut t = apinata_core::ScriptTrust::load(paths.script_trust.clone()).unwrap();
    t.set_decision(root, Some(apinata_core::ScriptDecision::Allowed));
    t.save().unwrap();
}

/// `started`, with secrets read from `store`.
pub fn started_with_store(
    cx: &mut gpui_kit::TestAppContext,
    col: &TempDir,
    store: std::sync::Arc<dyn apinata_core::SecretStore>,
) -> Started {
    started_on(cx, col, |_| {}, store)
}

pub fn started_on(
    cx: &mut gpui_kit::TestAppContext,
    col: &TempDir,
    prepare: impl FnOnce(&apinata_app::persist::Paths),
    store: std::sync::Arc<dyn apinata_core::SecretStore>,
) -> Started {
    start_session(cx, &[col.path()], prepare, store)
}

/// `started`, with the saved session holding every one of `roots` open, in order.
pub fn started_with_session(
    cx: &mut gpui_kit::TestAppContext,
    roots: &[&std::path::Path],
) -> Started {
    let store = std::sync::Arc::new(apinata_core::MemoryStore::new());
    start_session(cx, roots, |_| {}, store)
}

fn start_session(
    cx: &mut gpui_kit::TestAppContext,
    roots: &[&std::path::Path],
    prepare: impl FnOnce(&apinata_app::persist::Paths),
    store: std::sync::Arc<dyn apinata_core::SecretStore>,
) -> Started {
    use gpui_kit::AppContext;
    use gpui_kit::test::TestWindowExt as _;
    init(cx);
    let files = tempfile::tempdir().unwrap();
    let paths = apinata_app::persist::Paths::in_dir(files.path());
    apinata_app::persist::save_json(&paths.settings, &test_settings()).unwrap();
    prepare(&paths);
    let session = apinata_app::session::Session {
        open: roots.iter().map(|r| r.to_path_buf()).collect(),
        ..Default::default()
    };
    apinata_app::persist::save_json(&paths.session, &session).unwrap();
    let (win, shell) = cx
        .update(|cx| apinata_app::app::start(&paths, store, Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let ws = shell.read_with(cx, |s, _| s.workspace().clone());
    Started {
        files,
        root: roots[0].to_path_buf(),
        win,
        shell,
        ws,
    }
}

/// A data directory with settings saved and no session, as on a first launch.
pub fn blank_data_dir() -> TempDir {
    let files = tempfile::tempdir().unwrap();
    let paths = apinata_app::persist::Paths::in_dir(files.path());
    apinata_app::persist::save_json(&paths.settings, &test_settings()).unwrap();
    files
}

/// The real `app::start` over `files` with nothing open; `root` is the scratch's directory.
pub fn started_blank(cx: &mut gpui_kit::TestAppContext, files: TempDir) -> Started {
    use gpui_kit::AppContext;
    use gpui_kit::test::TestWindowExt as _;
    init(cx);
    let paths = apinata_app::persist::Paths::in_dir(files.path());
    let store = std::sync::Arc::new(apinata_core::MemoryStore::new());
    let (win, shell) = cx
        .update(|cx| apinata_app::app::start(&paths, store, Vec::new(), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    let ws = shell.read_with(cx, |s, _| s.workspace().clone());
    Started {
        root: files.path().join("scratch"),
        files,
        win,
        shell,
        ws,
    }
}

/// The active tab's collection root and draft item; panics when it is not a draft request.
pub fn active_draft(
    cx: &mut gpui_kit::TestAppContext,
    ws: &gpui_kit::Entity<apinata_app::workspace::Workspace>,
) -> (std::path::PathBuf, ItemPath) {
    ws.read_with(cx, |w, _| {
        let tab = w.tabs().active().expect("an active tab");
        match &tab.kind {
            apinata_app::tabs::TabKind::Request(p) if p.is_draft() => (tab.root.clone(), p.clone()),
            other => panic!("the active tab is not a draft: {other:?}"),
        }
    })
}

pub fn choose_menu(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    path: &str,
    kind: apinata_app::sidebar::model::NodeKind,
    entry: apinata_app::sidebar::menu::MenuEntry,
) {
    use gpui_kit::AppContext;
    use gpui_kit::test::TestWindowExt as _;
    assert!(
        apinata_app::sidebar::menu::menu_entries(kind).contains(&entry),
        "entry offered"
    );
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(row(path), cx)
    })
    .unwrap();
    frame(cx, win);
    // A GraphQL row names its conversion entry after the other kind.
    let label = cx
        .update_window(win, |_, w, _| {
            let other = apinata_app::sidebar::menu::MenuEntry::ConvertToHttp.label();
            match try_menu_index(w, entry.label()) {
                None if entry == apinata_app::sidebar::menu::MenuEntry::ConvertToGraphql => other,
                _ => entry.label(),
            }
        })
        .unwrap();
    click_menu(cx, win, label);
}

/// The index of the open popup menu's entry labelled `label`; separators count, so this is
/// the index `within("popup-menu").click` takes. Panics when no entry has the label.
pub fn menu_index(w: &mut gpui_kit::Window, label: &str) -> usize {
    try_menu_index(w, label).unwrap_or_else(|| panic!("{label:?} is in the menu"))
}

/// [`menu_index`], or `None` when no entry of the open menu has the label.
pub fn try_menu_index(w: &mut gpui_kit::Window, label: &str) -> Option<usize> {
    use gpui_kit::test::TestWindowExt as _;
    // A plain entry carries its label itself; a drawn one (Delete, a greyed-out entry in a
    // high-contrast theme) carries it on the element inside.
    (0..64usize).find(|&ix| {
        let mut menu = w.within("popup-menu");
        match menu.try_find(ix) {
            None => false,
            Some(item) if item.label() == Some(label) => true,
            Some(_) => menu.within(ix).try_find_by_label(label).is_some(),
        }
    })
}

/// The labels of the open popup menu's entries in order, `None` for each separator.
pub fn menu_labels(w: &mut gpui_kit::Window) -> Vec<Option<String>> {
    use gpui_kit::test::TestWindowExt as _;
    let mut labels = Vec::new();
    for ix in 0..64usize {
        let mut menu = w.within("popup-menu");
        let Some(item) = menu.try_find(ix) else {
            break;
        };
        let label = item.label().map(str::to_owned).or_else(|| {
            let inner = menu.within(ix).elements();
            inner.iter().find_map(|e| e.label().map(str::to_owned))
        });
        labels.push(label);
    }
    labels
}

/// Clicks the open popup menu's entry labelled `label`, then lets it run.
pub fn click_menu(cx: &mut gpui_kit::TestAppContext, win: gpui_kit::AnyWindowHandle, label: &str) {
    use gpui_kit::AppContext;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        let ix = menu_index(w, label);
        w.within("popup-menu").click(ix, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

/// The key hints painted on the open popup menu's entry `label`, without the label itself.
pub fn menu_row_keys(w: &mut gpui_kit::Window, label: &str) -> Vec<String> {
    use gpui_kit::test::TestWindowExt as _;
    let ix = menu_index(w, label);
    let row = w.within("popup-menu").find(ix).bounds();
    let menu = gpui_kit::ElementId::Name("popup-menu".into());
    let mut keys: Vec<String> = gpui_kit::base::test_support::texts(w)
        .into_iter()
        .filter(|t| t.path().contains(&menu) && row.contains(&t.bounds().center()))
        // The label starts at the row's left edge; a hint sits at its right end.
        .filter(|t| t.bounds().origin.x > row.center().x)
        .map(|t| t.text().to_owned())
        .collect();
    keys.dedup();
    keys
}

/// Opens the title bar's collection switcher and clicks `id` in its dropdown.
pub fn switcher_click(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: &'static str,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

/// The window showing the confirm dialog, if any.
pub fn confirm_window(cx: &mut gpui_kit::TestAppContext) -> Option<gpui_kit::AnyWindowHandle> {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.run_until_parked();
    cx.windows().into_iter().find(|&win| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("confirm-dialog").is_some()
        })
        .unwrap_or(false)
    })
}

/// True when some window shows the confirm dialog.
pub fn confirm_open(cx: &mut gpui_kit::TestAppContext) -> bool {
    confirm_window(cx).is_some()
}

/// Clicks the confirm dialog's button `id`, then lets the answer run.
pub fn answer_confirm(cx: &mut gpui_kit::TestAppContext, id: &'static str) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let win = confirm_window(cx).expect("a confirm dialog is open");
    cx.update_window(win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

/// The keymap overrides AppState holds now.
pub fn keymap_overrides(cx: &mut gpui_kit::TestAppContext) -> apinata_app::shortcuts::Overrides {
    cx.update(|cx| {
        let keymap = cx.global::<apinata_app::app::AppState>().keymap.clone();
        keymap.read(cx).get().clone()
    })
}

/// Overrides from `(id, chords)` pairs.
pub fn overrides(entries: &[(&str, &[&str])]) -> apinata_app::shortcuts::Overrides {
    entries
        .iter()
        .map(|(id, keys)| (id.to_string(), keys.iter().map(|k| k.to_string()).collect()))
        .collect()
}

/// Edits the live keymap through `keymap::set_overrides` and lets the windows settle.
pub fn edit_keys(
    cx: &mut gpui_kit::TestAppContext,
    f: impl FnOnce(&mut apinata_app::shortcuts::Overrides),
) {
    cx.update(|cx| apinata_app::keymap::set_overrides(f, cx));
    cx.run_until_parked();
}

/// Presses `ks` verbatim with the shell root focused.
pub fn press_in_shell(cx: &mut gpui_kit::TestAppContext, s: &Started, ks: &str) {
    use gpui_kit::Focusable as _;
    let shell = s.shell.clone();
    let mut v = gpui_kit::VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| shell.read(cx).focus_handle(cx).focus(w, cx));
    v.simulate_keystrokes(ks);
    v.run_until_parked();
}

/// Opens Preferences over the window and renders it.
pub fn open_prefs(
    cx: &mut gpui_kit::TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::preferences::PreferencesView> {
    use gpui_kit::AppContext as _;
    let view = cx
        .update_window(s.win, |_, w, cx| apinata_app::preferences::open(w, cx))
        .unwrap()
        .expect("AppState is set");
    frame(cx, s.win);
    view
}

/// The `aria_label` of Preferences row `key-<id>-keys` after a frame.
pub fn key_label(cx: &mut gpui_kit::TestAppContext, s: &Started, id: &str) -> Option<String> {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let element = format!("key-{id}-keys");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(element).label().map(str::to_owned)
    })
    .unwrap()
}

/// Clicks element `id` in Preferences, wheeling the dialog body until it shows, then renders.
pub fn prefs_click(cx: &mut gpui_kit::TestAppContext, s: &Started, id: &str) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    prefs_reveal(cx, s, id);
    let id = id.to_string();
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    frame(cx, s.win);
}

/// Wheels the Preferences body until element `id` lies wholly inside its scrolled rows.
pub fn prefs_reveal(cx: &mut gpui_kit::TestAppContext, s: &Started, id: &str) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.to_string();
    // The Keyboard section sits below the fold of a long dialog, and the status line under
    // the rows clips them, so the target must sit inside the rows' viewport, if one is open.
    // A target above the rows (scrolled past, or a control over them) wheels the body up.
    for _ in 0..40 {
        let (visible, above) = cx
            .update_window(s.win, |_, w, cx| {
                w.render_frame(cx);
                let target = w.find(id.clone());
                let at = target.bounds();
                let rows = w.try_find("preferences-rows").map(|d| d.bounds());
                let inside = rows.is_none_or(|d| at.top() >= d.top() && at.bottom() <= d.bottom());
                let above = rows.is_some_and(|d| at.top() < d.top());
                (target.visible() && inside, above)
            })
            .unwrap();
        if visible {
            break;
        }
        let dy = if above { 200. } else { -200. };
        cx.update_window(s.win, |_, w, cx| {
            let delta =
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(dy)));
            w.scroll("preferences", delta, cx)
        })
        .unwrap();
    }
}

/// Steps keyboard focus through the window's Tab order until element `id` reports focus.
pub fn tab_to(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    for _ in 0..300 {
        let on = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.try_find(id.clone()).and_then(|e| e.focused())
            })
            .unwrap();
        if on == Some(true) {
            return;
        }
        cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
        cx.run_until_parked();
    }
    panic!("Tab never reached {id:?}");
}

/// Presses Tab until element `id` inside the element named `scope` reports focus.
pub fn tab_within_to(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    scope: &'static str,
    id: impl Into<gpui_kit::ElementId>,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    for _ in 0..40 {
        let on = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.within(scope)
                    .try_find(id.clone())
                    .and_then(|e| e.focused())
            })
            .unwrap();
        if on == Some(true) {
            return;
        }
        cx.simulate_keystrokes(win, "tab");
        cx.run_until_parked();
    }
    panic!("Tab never reached {id:?} in {scope}");
}

/// Opens the environment editor's "…" menu and clicks entry `ix` (the separator counts).
pub fn env_menu(cx: &mut gpui_kit::TestAppContext, win: gpui_kit::AnyWindowHandle, ix: usize) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click("env-more", cx)
    })
    .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Picks entry `ix` of the global editor's "…" menu inside one update of `w`, drawing before
/// each click so the button and then the menu are there to press.
pub fn global_menu_in(w: &mut gpui_kit::Window, cx: &mut gpui_kit::App, ix: usize) {
    use gpui_kit::test::TestWindowExt as _;
    w.render_frame(cx);
    w.click("global-env-more", cx);
    w.render_frame(cx);
    w.within("popup-menu").click(ix, cx);
}

/// Opens the global editor's Rename field from its "…" menu, inside one update of `w`.
pub fn global_rename_in(w: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    global_menu_in(
        w,
        cx,
        apinata_app::global_env_editor::GLOBAL_ENV_MENU_RENAME,
    );
}

/// Asks to delete the global editor's environment from its "…" menu, inside one update.
pub fn global_delete_in(w: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    global_menu_in(
        w,
        cx,
        apinata_app::global_env_editor::GLOBAL_ENV_MENU_DELETE,
    );
}

/// Gives `ed`'s tab a finished result, as a restored kept send does on open.
pub fn give_result(
    cx: &mut gpui_kit::TestAppContext,
    ed: &gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
) {
    use apinata_core::{Outcome, RequestResult, Response};
    let model = ed.read_with(cx, |e, _| e.send_model().clone());
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body("ok")),
        Vec::new(),
    );
    let restored = apinata_app::send_model::Restored {
        saved_at: std::time::UNIX_EPOCH,
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    model.update(cx, |m, cx| {
        m.restore(std::sync::Arc::new(result), restored, cx)
    });
}

/// The request editor of `tab` in a started app's tab area.
pub fn editor_of(
    cx: &mut gpui_kit::TestAppContext,
    s: &Started,
    tab: apinata_app::tabs::TabId,
) -> gpui_kit::Entity<apinata_app::request_editor::RequestEditor> {
    s.shell.read_with(cx, |sh, cx| {
        sh.tab_area()
            .read(cx)
            .editor(tab)
            .expect("an editor")
            .clone()
    })
}

/// A new draft in `parent`, opened as the active tab: its item path and history key.
pub fn new_draft_at(
    cx: &mut gpui_kit::TestAppContext,
    s: &Started,
    parent: ItemPath,
) -> (String, ItemPath) {
    s.ws.update(cx, |w, cx| w.new_draft(&s.root, parent, cx))
        .expect("open");
    cx.run_until_parked();
    s.ws.read_with(cx, |w, _| {
        let e = w.tabs().active().expect("the draft tab");
        let key = w.tabs().history_key(e.id).cloned().expect("keyed");
        (e.item().expect("a request").as_str().to_owned(), key)
    })
}

/// Sends `edit` to request `item` of `s`'s collection, without waiting for it.
pub fn edit_item(
    cx: &mut gpui_kit::TestAppContext,
    s: &Started,
    item: &str,
    edit: apinata_app::command::Edit,
) {
    let command = Command::Edit {
        target: Target::Request(ip(item)),
        edit,
    };
    s.ws.update(cx, |w, cx| w.send(&s.root, command, cx));
}

/// Lets pending work run, then the draft save delay pass.
pub fn wait_save(cx: &mut gpui_kit::TestAppContext) {
    cx.run_until_parked();
    cx.executor()
        .advance_clock(apinata_app::persisted::SAVE_DELAY + std::time::Duration::from_millis(50));
    cx.run_until_parked();
}

/// Where draft `key` of `s` is kept.
pub fn draft_path(s: &Started, key: &ItemPath) -> std::path::PathBuf {
    s.files
        .path()
        .join("drafts")
        .join(apinata_app::draft_file::file_name(key))
}

/// `started`, with this test holding the drafts lock, so the app keeps no drafts and prompts
/// for them on close as a second instance would. Keep the returned file alive.
pub fn started_without_drafts(
    cx: &mut gpui_kit::TestAppContext,
    col: &TempDir,
) -> (Started, std::fs::File) {
    let held = std::cell::RefCell::new(None);
    let s = started_with(cx, col, |paths| {
        *held.borrow_mut() = apinata_app::draft_file::lock(&paths.drafts)
    });
    let lock = held.into_inner().expect("the drafts lock is free");
    (s, lock)
}

/// Whether element `id` paints a bordered quad at its own origin in the theme's border colour.
pub fn outlined(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    use gpui_kit::AppContext as _;
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let at = w.find(id.clone()).bounds();
        let scale = w.scale_factor();
        let border = cx.theme().border;
        w.painted_quads().into_iter().any(|q| {
            (q.bounds.origin.x.0 / scale - f32::from(at.left())).abs() < 1.
                && (q.bounds.origin.y.0 / scale - f32::from(at.top())).abs() < 1.
                && q.border_widths.left.0 > 0.
                && q.border_color == border
        })
    })
    .unwrap()
}

/// Clicks element `id` with `modifiers` held, as raw events: the stock helpers take none.
pub fn click_with(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
    modifiers: gpui_kit::Modifiers,
) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let at = w.find(id).bounds().center();
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers,
            click_count: 1,
            first_mouse: false,
        };
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers,
            click_count: 1,
        };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(up.to_platform_input(), cx);
        // The keys come up after the click, as a real release reports; the window keeps the
        // last modifiers it saw, and the kit's own clicks reuse them.
        if modifiers != gpui_kit::Modifiers::none() {
            let release = gpui_kit::ModifiersChangedEvent {
                modifiers: gpui_kit::Modifiers::none(),
                capslock: w.capslock(),
            };
            w.dispatch_event(release.to_platform_input(), cx);
        }
        w.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// A left click as the platform delivers it, at `id`'s centre in the last drawn frame: mouse
/// down and up with no forced frame, so only a redraw the app scheduled shows its effect.
pub fn platform_click(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        let at = w.find(id).bounds().center();
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Default::default(),
            click_count: 1,
        };
        w.dispatch_event(down.to_platform_input(), cx);
        w.dispatch_event(up.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The value element `id` showed in the last drawn frame; empty when it shows none.
pub fn drawn_text(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> String {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, _| {
        w.try_find(id).and_then(|e| e.value().map(str::to_owned))
    })
    .unwrap()
    .unwrap_or_default()
}

pub fn secondary_click(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    let secondary = cx.update(|cx| gpui_kit::Modifiers::secondary_key(cx));
    click_with(cx, win, id, secondary);
}

pub fn shift_click(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    click_with(cx, win, id, gpui_kit::Modifiers::shift());
}

/// The sidebar fixture of the multi-selection tests: four sibling requests (names sort a to d),
/// a folder with one request, and an opaque item.
pub fn multi_files() -> Vec<(&'static str, String)> {
    vec![
        ("opencollection.yml", ROOT.to_string()),
        ("a.yml", request("A", "http://x/a")),
        ("b.yml", request("B", "http://x/b")),
        ("c.yml", request("C", "http://x/c")),
        ("d.yml", request("D", "http://x/d")),
        ("f/folder.yml", folder("F")),
        ("f/x.yml", request("X", "http://x/x")),
        ("w.yml", SOCKET.to_string()),
    ]
}

/// Two folders, e with one request and f with two, so a shift range from e's request to f's
/// first one crosses the f row and leaves f's second request unselected.
pub fn range_files() -> Vec<(&'static str, String)> {
    vec![
        ("opencollection.yml", ROOT.to_string()),
        ("e/folder.yml", folder("E")),
        ("e/y.yml", request("Y", "http://x/y")),
        ("f/folder.yml", folder("F")),
        ("f/x.yml", request("X", "http://x/x")),
        ("f/z.yml", request("Z", "http://x/z")),
        ("a.yml", request("A", "http://x/a")),
    ]
}

/// Whether element `id` paints a solid quad of `color` at its own origin.
pub fn filled(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
    color: gpui_kit::Hsla,
) -> bool {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let at = w.find(id.clone()).bounds();
        let scale = w.scale_factor();
        w.painted_quads().into_iter().any(|q| {
            (q.bounds.origin.x.0 / scale - f32::from(at.left())).abs() < 1.
                && (q.bounds.origin.y.0 / scale - f32::from(at.top())).abs() < 1.
                && q.background == gpui_kit::Background::from(color)
        })
    })
    .unwrap()
}

/// A port that refuses every connection and that no other process can take: the socket is
/// bound for the whole process but never listens.
pub fn dead_port() -> u16 {
    use socket2::{Domain, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("socket");
    socket
        .bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into())
        .expect("bind");
    let port = socket
        .local_addr()
        .expect("addr")
        .as_socket()
        .expect("ip")
        .port();
    std::mem::forget(socket);
    port
}

/// A directory holding a `git` that stalls on `verb` (writes its sleeper's pid to `pid`, prints
/// 10% progress, and for `clone` creates the destination) and runs the real git for any other call.
pub fn stalling_git(sb: &crate::git_support::Sandbox, verb: &str) -> std::path::PathBuf {
    const BODY: &str = r#"#!/bin/sh
dir=${0%/*}
case " $* " in
*" VERB "*)
  for last; do :; done
  if [ "VERB" = clone ]; then mkdir -p "$last"; echo partial > "$last/part"; fi
  sleep 60 &
  echo $! > "$dir/pid"
  echo "Receiving objects:  10% (1/10)" >&2
  wait ;;
*) PATH=${PATH#*:}; exec git "$@" ;;
esac
"#;
    sb.fake_git("stalling", &BODY.replace("VERB", verb))
}

/// The pid a `stalling_git` sleeper wrote.
pub fn sleeper_pid(bin: &std::path::Path) -> u32 {
    std::fs::read_to_string(bin.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// A git whose `switch` and `checkout` block until the file `release` appears in the returned
/// directory, so a test holds a switch open and ends it by calling `release_git`; `reached`
/// appears once git is waiting.
pub fn latched_git(sb: &crate::git_support::Sandbox) -> std::path::PathBuf {
    const BODY: &str = r#"#!/bin/sh
dir=${0%/*}
case "$*" in
*switch*|*checkout*)
  : > "$dir/reached"
  while [ ! -e "$dir/release" ]; do sleep 0.02; done ;;
esac
exec /usr/bin/git "$@"
"#;
    sb.fake_git("latched", BODY)
}

/// Lets a `latched_git` carry on.
pub fn release_git(bin: &std::path::Path) {
    std::fs::write(bin.join("release"), "").unwrap();
}

/// Drops a path's permissions for one test and puts the original mode back on drop, so a
/// failing assertion never leaves a temp dir the cleanup cannot remove. Bind it with
/// `let _g =` after the `TempDir` so it drops first.
#[cfg(unix)]
pub struct ModeGuard {
    path: std::path::PathBuf,
    original: u32,
}

#[cfg(unix)]
impl ModeGuard {
    /// Records the current mode, then applies `mode`. The guard exists before the chmod, so a
    /// panic right after still restores.
    pub fn set(path: impl AsRef<std::path::Path>, mode: u32) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let path = path.as_ref().to_path_buf();
        let original = std::fs::metadata(&path).expect("stat").permissions().mode();
        let guard = Self { path, original };
        std::fs::set_permissions(&guard.path, std::fs::Permissions::from_mode(mode))
            .expect("chmod");
        guard
    }

    /// Puts the original mode back now; the drop does it again harmlessly.
    pub fn restore(&self) {
        use std::os::unix::fs::PermissionsExt as _;
        let _ =
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(self.original));
    }
}

#[cfg(unix)]
impl Drop for ModeGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Makes the registered theme `name` the active one, in its own mode.
pub fn use_theme(cx: &mut gpui_kit::TestAppContext, name: &str) {
    use gpui_kit::component::{Theme, ThemeRegistry};
    init(cx);
    cx.update(|cx| {
        let config = ThemeRegistry::global(cx)
            .themes()
            .get(name)
            .cloned()
            .unwrap();
        let mode = config.mode;
        Theme::update(cx, |t| {
            if mode.is_dark() {
                t.dark_theme = config;
            } else {
                t.light_theme = config;
            }
        });
        apinata_app::theme::set_mode(mode, None, cx);
    });
}

/// The WCAG contrast ratio of two opaque colours.
pub fn contrast(a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> f32 {
    let lum = |c: gpui_kit::Hsla| {
        let c = c.to_rgb();
        let ch = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// One left press `dx` pixels inside the left edge of the element `id`, where the kit's input
/// frame has padding rather than text.
pub fn press_in(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
    dx: f32,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, point, px};
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        let b = w.find(id).bounds();
        let p = point(b.left() + px(dx), b.center().y);
        let at = MouseMoveEvent {
            position: p,
            pressed_button: None,
            modifiers: Default::default(),
        };
        w.dispatch_event(at.to_platform_input(), cx);
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: p,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        w.dispatch_event(down.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// A key press as the platform delivers it: key-down (with its text, except Enter and Tab) and
/// key-up, each drawing first only if the window asked for a frame. No forced test frame can
/// paint a focus move that the app never scheduled a redraw for.
pub fn platform_press(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    key: &str,
) {
    use gpui_kit::AppContext as _;
    use gpui_kit::{InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke};
    let k = Keystroke::parse_with_os(key, apinata_app::shortcuts::HOST_OS).unwrap();
    cx.update_window(win, |_, w, cx| {
        if matches!(key, "enter" | "tab") {
            let down = KeyDownEvent {
                keystroke: k.clone(),
                is_held: false,
                prefer_character_input: false,
            };
            w.dispatch_event(down.to_platform_input(), cx);
        } else {
            w.dispatch_keystroke(k.clone(), cx);
        }
        w.dispatch_event(KeyUpEvent { keystroke: k }.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The platform's next frame: the frame callbacks run, then it is drawn only when the window is
/// dirty, as the real frame source does.
pub fn next_frame(cx: &mut gpui_kit::TestAppContext, win: gpui_kit::AnyWindowHandle) {
    use gpui_kit::AppContext as _;
    use gpui_kit::{InputEvent as _, ModifiersChangedEvent};
    cx.update_window(win, |_, w, cx| {
        w.simulate_next_frame(cx);
        w.dispatch_event(ModifiersChangedEvent::default().to_platform_input(), cx);
    })
    .unwrap();
}

/// Whether the last drawn frame shows `id` focused, which is what paints its ring.
pub fn ring_on(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, _| w.try_find(id).and_then(|e| e.focused()))
        .unwrap()
        == Some(true)
}

/// Tabs to `id` with test presses, then presses Enter as the platform does and lets the
/// resulting frame draw: the state a keyboard user sees after activating `id`.
pub fn platform_enter_on(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) {
    tab_to(cx, win, id);
    platform_press(cx, win, "enter");
    next_frame(cx, win);
    next_frame(cx, win);
}

/// Selects `tab`; the Cookies tab opens on its Jar view, which the jar-table tests read.
pub fn jar_tab(
    p: &mut apinata_app::response_pane::ResponsePane,
    tab: apinata_app::response_pane::ResponseTab,
    cx: &mut gpui_kit::Context<apinata_app::response_pane::ResponsePane>,
) {
    use apinata_app::response_pane::{CookiesView, ResponseTab};
    p.set_tab(tab, cx);
    if tab == ResponseTab::Cookies {
        p.set_cookies_view(CookiesView::Jar, cx);
    }
}

/// Whether a table's name or value cell (`kv-name`, `kv-value`) holds focus in a freshly drawn
/// frame, inside the element named `scope` when one is given.
pub fn focus_within(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    scope: Option<&str>,
) -> bool {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let name = scope.map(|s| format!("Name({s:?})"));
    let cell = |id: &gpui_kit::ElementId| {
        let id = format!("{id:?}");
        ["kv-name", "kv-value"]
            .iter()
            .any(|c| id.starts_with(&format!("NamedInteger({c:?}")))
    };
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        gpui_kit::base::test_support::snapshots(w)
            .into_iter()
            .filter(|e| e.path().last().is_some_and(cell))
            .filter(|e| {
                (name.as_ref()).is_none_or(|n| e.path().iter().any(|p| &format!("{p:?}") == n))
            })
            .any(|e| e.focused() == Some(true))
    })
    .unwrap()
}

/// Whether the frame last drawn, not a fresh one, paints a focus ring in the theme's ring
/// colour around element `id`: a bordered quad enclosing it, at most a few pixels outside.
pub fn ring_drawn(
    cx: &mut gpui_kit::TestAppContext,
    win: gpui_kit::AnyWindowHandle,
    id: impl Into<gpui_kit::ElementId>,
) -> bool {
    use gpui_kit::AppContext as _;
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::test::TestWindowExt as _;
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        let ring = cx.theme().ring.to_rgb();
        let scale = w.scale_factor();
        let b = w.find(id).bounds();
        let (l, t) = (b.left().as_f32() * scale, b.top().as_f32() * scale);
        let (r, bt) = (b.right().as_f32() * scale, b.bottom().as_f32() * scale);
        w.painted_quads().into_iter().any(|q| {
            let (x, y) = (q.bounds.origin.x.0, q.bounds.origin.y.0);
            let (x2, y2) = (x + q.bounds.size.width.0, y + q.bounds.size.height.0);
            let c = q.border_color.to_rgb();
            q.border_widths.left.0 > 0.
                && q.border_color.a > 0.
                && (c.r - ring.r).abs() < 0.01
                && (c.g - ring.g).abs() < 0.01
                && (c.b - ring.b).abs() < 0.01
                && x <= l + 1.
                && y <= t + 1.
                && x2 >= r - 1.
                && y2 >= bt - 1.
                && (x - l).abs() < 6. * scale
                && (y - t).abs() < 6. * scale
        })
    })
    .unwrap()
}
