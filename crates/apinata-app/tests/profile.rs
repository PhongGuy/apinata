//! The profile directory override for the app's files.

use apinata_app::persist::Paths;

#[test]
fn a_profile_dir_puts_every_file_inside_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let paths = Paths::for_profile(Some(dir.path().into())).unwrap();
    assert_eq!(paths, Paths::in_dir(dir.path()));
}
