//! A new request file and the importers agree on the default timeout.

use apinata_core::import::parse_curl;
use apinata_core::{Collection, ItemPath, RequestFile, RequestSettings, Timeout};
use tempfile::TempDir;

#[test]
fn new_file_settings_inherit_the_timeout() {
    assert_eq!(RequestSettings::for_new_file().timeout, Timeout::Inherit);
}

#[test]
fn a_created_request_file_reads_back_the_new_file_timeout() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("c");
    let mut collection = Collection::create(&root, "c").expect("creates");
    collection
        .create_request(&ItemPath::root(), "r")
        .expect("creates request");
    let file = RequestFile::load(root.join("r.yml")).expect("loads");
    assert_eq!(
        file.settings().timeout,
        RequestSettings::for_new_file().timeout
    );
}

#[test]
fn the_curl_importer_uses_the_new_file_timeout() {
    let parsed = parse_curl("curl https://x.test/a").expect("parses");
    let settings = parsed.request.settings.expect("settings");
    assert_eq!(settings.timeout, RequestSettings::for_new_file().timeout);
}

#[test]
fn a_created_request_file_reads_back_the_new_file_settings() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("c");
    let mut collection = Collection::create(&root, "c").expect("creates");
    collection
        .create_request(&ItemPath::root(), "r")
        .expect("creates request");
    let file = RequestFile::load(root.join("r.yml")).expect("loads");
    assert_eq!(file.settings(), RequestSettings::for_new_file());
}

#[test]
fn new_files_do_not_forward_authorization_but_an_absent_key_does() {
    // The reference writes false for a request it creates and reads a missing key as true, so
    // files from before the setting existed keep their behavior.
    assert!(!RequestSettings::for_new_file().forward_authorization_header);
    assert!(RequestSettings::new().forward_authorization_header);
}
