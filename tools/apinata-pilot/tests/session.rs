use apinata_app::persist::{Paths, load_json};
use apinata_app::session::Session;
use apinata_pilot::{prepare_session, seed_session};

#[test]
fn absolute_roots_become_the_open_collections() {
    let data = tempfile::tempdir().unwrap();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let paths = Paths::in_dir(data.path());
    let roots = [a.path(), b.path()].map(|p| p.to_str().unwrap().to_owned());
    seed_session(&paths, &roots).unwrap();
    let loaded = load_json::<Session>(&paths.session);
    assert_eq!(loaded.recovered, None);
    assert_eq!(
        loaded.value.open,
        vec![a.path().to_path_buf(), b.path().to_path_buf()]
    );
}

#[test]
fn no_roots_open_nothing() {
    let data = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(data.path());
    seed_session(&paths, &[]).unwrap();
    assert!(load_json::<Session>(&paths.session).value.open.is_empty());
}

#[test]
fn a_relative_root_is_refused_and_writes_nothing() {
    let data = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(data.path());
    let error = seed_session(&paths, &["collections/demo".to_owned()]).unwrap_err();
    assert!(error.to_string().contains("must be absolute"), "{error}");
    assert!(!paths.session.exists());
}

#[test]
fn a_missing_root_is_refused() {
    let data = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(data.path());
    let missing = data.path().join("nope").to_str().unwrap().to_owned();
    let error = seed_session(&paths, &[missing]).unwrap_err();
    assert!(error.to_string().contains("is not a directory"), "{error}");
}

#[test]
fn a_reused_launch_keeps_the_saved_session() {
    let data = tempfile::tempdir().unwrap();
    let (kept, arg) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let paths = Paths::in_dir(data.path());
    seed_session(&paths, &[kept.path().to_str().unwrap().to_owned()]).unwrap();
    let before = std::fs::read(&paths.session).unwrap();
    prepare_session(&paths, &[arg.path().to_str().unwrap().to_owned()], true).unwrap();
    assert_eq!(std::fs::read(&paths.session).unwrap(), before);
}

#[test]
fn a_fresh_launch_seeds_the_session_from_its_roots() {
    let data = tempfile::tempdir().unwrap();
    let (old, arg) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let paths = Paths::in_dir(data.path());
    seed_session(&paths, &[old.path().to_str().unwrap().to_owned()]).unwrap();
    prepare_session(&paths, &[arg.path().to_str().unwrap().to_owned()], false).unwrap();
    let loaded = load_json::<Session>(&paths.session);
    assert_eq!(loaded.value.open, vec![arg.path().to_path_buf()]);
}

#[test]
fn a_reused_launch_keeps_the_saved_expanded_folders() {
    use apinata_app::session::CollectionSession;
    let data = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::in_dir(data.path());
    let mut saved = Session::default();
    saved.collections.insert(
        root.path().to_path_buf(),
        CollectionSession {
            expanded: Some(vec!["".into(), "users".into()]),
            ..CollectionSession::default()
        },
    );
    apinata_app::persist::save_json(&paths.session, &saved).unwrap();
    prepare_session(&paths, &[root.path().to_str().unwrap().to_owned()], true).unwrap();
    assert_eq!(load_json::<Session>(&paths.session).value, saved);
}
