//! The bundled demo collection: its embedded copy, the copy helper and the empty-state entry.

use crate::support;

use apinata_app::demo_collection;
use apinata_app::session::Session;
use apinata_app::shell::Shell;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions};
use std::collections::BTreeMap;
use std::path::Path;
use support::*;

fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, base, out);
        } else {
            let rel = path.strip_prefix(base).unwrap();
            out.insert(
                rel.to_string_lossy().replace('\\', "/"),
                std::fs::read(&path).unwrap(),
            );
        }
    }
}

fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn the_embedded_copy_matches_examples_demo_byte_for_byte() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo");
    let mut expected = tree(&source);
    expected.retain(|name, _| {
        let file = name.rsplit('/').next().unwrap();
        file != ".env" && !file.starts_with(".env.") && !file.starts_with(".git")
    });
    let embedded: BTreeMap<_, _> = demo_collection::files().into_iter().collect();
    assert!(embedded.contains_key("opencollection.yml"));
    assert_eq!(embedded, expected);
}

#[test]
fn install_writes_the_whole_tree_under_a_fresh_name() {
    let parent = tempfile::tempdir().unwrap();
    let root = demo_collection::install(parent.path()).unwrap();
    assert_eq!(root, parent.path().join("Apinata Demo"));
    let written = tree(&root);
    let embedded: BTreeMap<_, _> = demo_collection::files().into_iter().collect();
    assert_eq!(written, embedded);
}

#[test]
fn install_never_overwrites_an_existing_folder() {
    let parent = tempfile::tempdir().unwrap();
    let taken = parent.path().join("Apinata Demo");
    std::fs::create_dir(&taken).unwrap();
    std::fs::write(taken.join("mine.txt"), "keep").unwrap();
    let root = demo_collection::install(parent.path()).unwrap();
    assert_eq!(root, parent.path().join("Apinata Demo - 1"));
    assert_eq!(
        std::fs::read_to_string(taken.join("mine.txt")).unwrap(),
        "keep"
    );
    assert!(!taken.join("opencollection.yml").exists());
    assert!(root.join("opencollection.yml").exists());
}

#[test]
fn install_leaves_no_staging_folder_behind() {
    let parent = tempfile::tempdir().unwrap();
    demo_collection::install(parent.path()).unwrap();
    let names: Vec<_> = std::fs::read_dir(parent.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["Apinata Demo".to_string()]);
}

fn welcome(
    cx: &mut TestAppContext,
    start: Session,
) -> (
    gpui_kit::AnyWindowHandle,
    gpui_kit::Entity<Shell>,
    tempfile::TempDir,
) {
    support::init(cx);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| Shell::new(ws, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view, state)
}

#[gpui_kit::test]
fn the_welcome_screen_offers_the_demo_when_there_are_no_recents(cx: &mut TestAppContext) {
    let (win, _view, _state) = welcome(cx, Session::default());
    assert!(present(cx, win, "demo-hint"));
    let label = cx
        .update_window(win, |_, w, _| {
            w.within("no-collection-actions")
                .find(("empty-action", 5usize))
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Open the demo collection…"));
}

#[gpui_kit::test]
fn a_recent_collection_hides_the_demo_offer(cx: &mut TestAppContext) {
    let col = collection();
    let start = Session {
        recent: vec![col.path().to_path_buf()],
        ..Session::default()
    };
    let (win, _view, _state) = welcome(cx, start);
    assert!(!present(cx, win, "demo-hint"));
    let has = cx
        .update_window(win, |_, w, _| {
            w.within("no-collection-actions")
                .try_find(("empty-action", 5usize))
                .is_some()
        })
        .unwrap();
    assert!(!has);
}

#[gpui_kit::test]
fn choosing_a_folder_copies_the_demo_there_and_opens_it(cx: &mut TestAppContext) {
    let (win, view, _state) = welcome(cx, Session::default());
    let parent = tempfile::tempdir().unwrap();
    cx.update_window(win, |_, w, cx| {
        w.within("no-collection-actions")
            .click(("empty-action", 5usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    let chosen = parent.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen.clone()]));
    for _ in 0..50 {
        cx.run_until_parked();
        if view.read_with(cx, |v, cx| v.workspace().read(cx).collections().len()) == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        parent
            .path()
            .join("Apinata Demo/opencollection.yml")
            .exists()
    );
    assert_eq!(
        view.read_with(cx, |v, cx| v.workspace().read(cx).collections().len()),
        1
    );
}

#[gpui_kit::test]
fn recents_that_no_longer_exist_still_get_the_demo_offer(cx: &mut TestAppContext) {
    let gone = tempfile::tempdir().unwrap().path().join("deleted");
    let start = Session {
        recent: vec![gone],
        ..Session::default()
    };
    let (win, _view, _state) = welcome(cx, start);
    assert!(present(cx, win, "demo-hint"));
}

#[cfg(unix)]
#[test]
fn installed_folders_get_the_mode_of_a_normally_created_directory() {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let parent = tempfile::tempdir().unwrap();
    let normal = parent.path().join("normal");
    std::fs::create_dir(&normal).unwrap();
    let root = demo_collection::install(parent.path()).unwrap();
    let mut dirs = vec![root.clone()];
    let mut i = 0;
    while i < dirs.len() {
        for e in std::fs::read_dir(&dirs[i]).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            }
        }
        i += 1;
    }
    assert!(dirs.len() > 1, "the demo has subfolders");
    for d in dirs {
        assert_eq!(mode(&d), mode(&normal), "{}", d.display());
    }
}

#[gpui_kit::test]
fn the_palette_command_starts_the_same_flow_as_the_empty_state_button(cx: &mut TestAppContext) {
    let col = collection();
    let start = Session {
        recent: vec![col.path().to_path_buf()],
        ..Session::default()
    };
    let (win, view, _state) = welcome(cx, start);
    let entry = apinata_app::palette::palette_items()
        .into_iter()
        .find(|e| e.label == "Open the demo collection…")
        .expect("the palette lists the demo command");
    cx.update_window(win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.did_prompt_for_paths());
    let parent = tempfile::tempdir().unwrap();
    let chosen = parent.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen.clone()]));
    for _ in 0..50 {
        cx.run_until_parked();
        if parent
            .path()
            .join("Apinata Demo/opencollection.yml")
            .exists()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        parent
            .path()
            .join("Apinata Demo/opencollection.yml")
            .exists()
    );
    let _ = view;
}

// The demo has one environment and no saved choice yet, so it opens with that one selected.
#[gpui_kit::test]
fn the_demo_opens_with_its_environment_selected(cx: &mut TestAppContext) {
    let (win, view, _state) = welcome(cx, Session::default());
    let parent = tempfile::tempdir().unwrap();
    cx.update_window(win, |_, w, cx| {
        w.within("no-collection-actions")
            .click(("empty-action", 5usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    let chosen = parent.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen.clone()]));
    let root = parent.path().join("Apinata Demo");
    let selected = |cx: &mut TestAppContext| {
        view.read_with(cx, |v, cx| {
            v.workspace().read(cx).selected_environment(&root, cx)
        })
    };
    for _ in 0..100 {
        cx.run_until_parked();
        if selected(cx).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(selected(cx).as_deref(), Some("JSONPlaceholder"));
}
