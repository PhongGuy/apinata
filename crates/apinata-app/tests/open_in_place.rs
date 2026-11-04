//! "Show in file manager" and "Open in terminal" on sidebar rows. Both go through injected
//! seams, so nothing real is ever launched.

use crate::support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use apinata_app::commands::registry;
use apinata_app::open_external::{self, FileOpener};
use apinata_app::sidebar::menu::{MenuEntry, entry_dir, menu_entries};
use apinata_app::sidebar::model::NodeKind;
use apinata_app::sidebar::{OpenInTerminal, ShowInFileManager};
use apinata_app::terminal::{self, Launch, Platform, Setup, resolve};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use support::*;

fn no_env(_: &str) -> Option<String> {
    None
}

fn launch(program: &str, args: &[&str], cwd: &str) -> Launch {
    Launch {
        program: program.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: PathBuf::from(cwd),
    }
}

#[test]
fn linux_prefers_terminal_then_the_fallback_order() {
    let dir = Path::new("/w/c");
    let env = |k: &str| (k == "TERMINAL").then(|| "kitty".to_string());
    let have = |names: &'static [&'static str]| move |p: &str| names.contains(&p);
    assert_eq!(
        resolve(Platform::Linux, dir, &env, &have(&["kitty", "xterm"])),
        Some(launch("kitty", &[], "/w/c"))
    );
    // TERMINAL set but not installed falls through to the list.
    assert_eq!(
        resolve(Platform::Linux, dir, &env, &have(&["konsole", "xterm"])),
        Some(launch("konsole", &[], "/w/c"))
    );
    assert_eq!(
        resolve(
            Platform::Linux,
            dir,
            &no_env,
            &have(&["xterm", "gnome-terminal"])
        ),
        Some(launch("gnome-terminal", &[], "/w/c"))
    );
    assert_eq!(
        resolve(
            Platform::Linux,
            dir,
            &no_env,
            &have(&["x-terminal-emulator", "xterm"])
        ),
        Some(launch("x-terminal-emulator", &[], "/w/c"))
    );
    assert_eq!(
        resolve(Platform::Linux, dir, &no_env, &have(&["xterm"])),
        Some(launch("xterm", &[], "/w/c"))
    );
    assert_eq!(resolve(Platform::Linux, dir, &no_env, &have(&[])), None);
    // An empty TERMINAL counts as unset.
    let empty = |k: &str| (k == "TERMINAL").then(String::new);
    assert_eq!(resolve(Platform::Linux, dir, &empty, &have(&[])), None);
}

#[test]
fn terminal_with_arguments_keeps_them_ahead_of_the_launch_args() {
    let dir = Path::new("/w/c");
    let env = |k: &str| (k == "TERMINAL").then(|| "kitty -1  --title x".to_string());
    let have = |names: &'static [&'static str]| move |p: &str| names.contains(&p);
    assert_eq!(
        resolve(Platform::Linux, dir, &env, &have(&["kitty", "xterm"])),
        Some(launch("kitty", &["-1", "--title", "x"], "/w/c"))
    );
    // The first token is what must exist; a missing one falls through to the list.
    assert_eq!(
        resolve(
            Platform::Linux,
            dir,
            &env,
            &have(&["x-terminal-emulator", "xterm"])
        ),
        Some(launch("x-terminal-emulator", &[], "/w/c"))
    );
    // Whitespace only counts as unset.
    let blank = |k: &str| (k == "TERMINAL").then(|| "  ".to_string());
    assert_eq!(resolve(Platform::Linux, dir, &blank, &have(&[])), None);
}

#[test]
fn macos_opens_the_terminal_app_on_the_directory() {
    let got = resolve(Platform::MacOs, Path::new("/w/c"), &no_env, &|_| false);
    assert_eq!(
        got,
        Some(launch("open", &["-a", "Terminal", "/w/c"], "/w/c"))
    );
}

#[test]
fn windows_prefers_wt_then_cmd() {
    let dir = Path::new("C:/w");
    assert_eq!(
        resolve(Platform::Windows, dir, &no_env, &|p| p == "wt"),
        Some(launch("wt", &["-d", "C:/w"], "C:/w"))
    );
    assert_eq!(
        resolve(Platform::Windows, dir, &no_env, &|_| false),
        Some(launch("cmd", &["/c", "start", "cmd", "/k"], "C:/w"))
    );
}

#[test]
fn the_directory_is_the_folder_or_the_parent() {
    let root = Path::new("/w/c");
    let at = |k, p: &str| entry_dir(k, root, &ip(p));
    assert_eq!(at(NodeKind::Collection, ""), Some(PathBuf::from("/w/c")));
    assert_eq!(
        at(NodeKind::Folder, "users"),
        Some(PathBuf::from("/w/c/users"))
    );
    assert_eq!(
        at(NodeKind::Request, "health.yml"),
        Some(PathBuf::from("/w/c"))
    );
    assert_eq!(
        at(NodeKind::Request, "users/list.yml"),
        Some(PathBuf::from("/w/c/users"))
    );
    assert_eq!(at(NodeKind::Opaque, "ws.yml"), Some(PathBuf::from("/w/c")));
    assert_eq!(at(NodeKind::Draft, "x.yml"), None);
}

#[test]
fn the_menu_offers_both_entries_on_item_rows_only() {
    for kind in [
        NodeKind::Collection,
        NodeKind::Folder,
        NodeKind::Request,
        NodeKind::Opaque,
    ] {
        let e = menu_entries(kind);
        assert!(e.contains(&MenuEntry::ShowInFileManager), "{kind:?}");
        assert!(e.contains(&MenuEntry::OpenInTerminal), "{kind:?}");
    }
    for kind in [NodeKind::Example, NodeKind::Draft, NodeKind::Invalid] {
        let e = menu_entries(kind);
        assert!(!e.contains(&MenuEntry::ShowInFileManager), "{kind:?}");
        assert!(!e.contains(&MenuEntry::OpenInTerminal), "{kind:?}");
    }
    assert_eq!(MenuEntry::ShowInFileManager.label(), "Show in file manager");
    assert_eq!(MenuEntry::OpenInTerminal.label(), "Open in terminal");
}

#[test]
fn the_palette_registry_has_both_commands_unbound() {
    for (id, label) in [
        ("request.show_in_file_manager", "Show in file manager"),
        ("request.open_in_terminal", "Open in terminal"),
    ] {
        let c = registry().into_iter().find(|c| c.id == id).expect(id);
        assert_eq!(c.label, label);
        assert!(c.default_chords.is_empty(), "{id} has no default shortcut");
    }
}

type Opened = Arc<Mutex<Vec<PathBuf>>>;
type Launched = Arc<Mutex<Vec<(String, Vec<String>, PathBuf)>>>;

fn install_opener(cx: &mut TestAppContext) -> (Opened, tempfile::TempDir) {
    let seen = Opened::default();
    let log = seen.clone();
    let opener: FileOpener = Arc::new(move |p, _| log.lock().unwrap().push(p.to_path_buf()));
    let base = tempfile::tempdir().unwrap();
    let b = base.path().to_path_buf();
    cx.update(|cx| open_external::install(opener, b, cx));
    (seen, base)
}

fn install_terminal(cx: &mut TestAppContext, found: &'static [&'static str]) -> Launched {
    let seen = Launched::default();
    let log = seen.clone();
    cx.update(|cx| {
        terminal::install(
            Setup {
                platform: Platform::Linux,
                env: Arc::new(no_env),
                exists: Arc::new(move |p| found.contains(&p)),
                launch: Arc::new(move |prog, args, cwd| {
                    log.lock()
                        .unwrap()
                        .push((prog.into(), args.to_vec(), cwd.into()));
                    Ok(())
                }),
            },
            cx,
        )
    });
    seen
}

fn toasts(cx: &mut TestAppContext, h: &SidebarHarness) -> usize {
    cx.update_window(h.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn choose(cx: &mut TestAppContext, h: &SidebarHarness, path: &str, kind: NodeKind, e: MenuEntry) {
    let ix = menu_entries(kind).iter().position(|x| *x == e).unwrap();
    cx.update_window(h.win, |_, w, cx| w.right_click(row(path), cx))
        .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn show_in_file_manager_opens_a_directory_for_each_row_kind(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let (seen, _base) = install_opener(cx);
    let show = MenuEntry::ShowInFileManager;
    choose(cx, &h, "", NodeKind::Collection, show);
    choose(cx, &h, "users", NodeKind::Folder, show);
    choose(cx, &h, "health.yml", NodeKind::Request, show);
    choose(cx, &h, "ws.yml", NodeKind::Opaque, show);
    assert_eq!(
        *seen.lock().unwrap(),
        [
            h.root.clone(),
            h.root.join("users"),
            h.root.clone(),
            h.root.clone()
        ]
    );
}

#[gpui_kit::test]
fn show_in_file_manager_without_an_opener_toasts_an_error(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let before = toasts(cx, &h);
    choose(
        cx,
        &h,
        "users",
        NodeKind::Folder,
        MenuEntry::ShowInFileManager,
    );
    assert_eq!(toasts(cx, &h), before + 1);
}

#[gpui_kit::test]
fn open_in_terminal_launches_with_program_args_and_cwd(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = install_terminal(cx, &["xterm"]);
    let before = toasts(cx, &h);
    choose(cx, &h, "users", NodeKind::Folder, MenuEntry::OpenInTerminal);
    choose(
        cx,
        &h,
        "status.yml",
        NodeKind::Request,
        MenuEntry::OpenInTerminal,
    );
    assert_eq!(
        *seen.lock().unwrap(),
        [
            ("xterm".to_string(), vec![], h.root.join("users")),
            ("xterm".to_string(), vec![], h.root.clone()),
        ]
    );
    assert_eq!(toasts(cx, &h), before);
}

#[gpui_kit::test]
fn no_terminal_found_toasts_an_error_and_launches_nothing(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = install_terminal(cx, &[]);
    let before = toasts(cx, &h);
    choose(cx, &h, "", NodeKind::Collection, MenuEntry::OpenInTerminal);
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(toasts(cx, &h), before + 1);
}

#[gpui_kit::test]
fn a_failed_spawn_toasts_an_error(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update(|cx| {
        terminal::install(
            Setup {
                platform: Platform::Linux,
                env: Arc::new(no_env),
                exists: Arc::new(|_| true),
                launch: Arc::new(|_, _, _| Err("boom".into())),
            },
            cx,
        )
    });
    let before = toasts(cx, &h);
    choose(cx, &h, "", NodeKind::Collection, MenuEntry::OpenInTerminal);
    assert_eq!(toasts(cx, &h), before + 1);
}

#[gpui_kit::test]
fn the_palette_actions_act_on_the_selected_row(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let (opened, _base) = install_opener(cx);
    let launched = install_terminal(cx, &["xterm"]);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| {
            s.show_in_file_manager(&ShowInFileManager, w, cx);
            s.open_in_terminal(&OpenInTerminal, w, cx);
        })
    })
    .unwrap();
    assert_eq!(*opened.lock().unwrap(), [h.root.join("users")]);
    assert_eq!(launched.lock().unwrap()[0].2, h.root.join("users"));
}
