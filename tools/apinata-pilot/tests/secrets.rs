use std::sync::Arc;
use std::time::{Duration, SystemTime};

use apinata_core::{CookieInfo, CookieJar, SecretKey, SecretStore};
use apinata_pilot::secrets::{FILE_NAME, FileSecrets};

fn key() -> SecretKey {
    SecretKey::new("/collections/demo", "local", "token")
}

#[test]
fn a_reused_open_keeps_a_saved_secret() {
    let data = tempfile::tempdir().unwrap();
    FileSecrets::open(data.path(), false)
        .unwrap()
        .set(&key(), "s3cret")
        .unwrap();
    let again = FileSecrets::open(data.path(), true).unwrap();
    assert_eq!(again.get(&key()).unwrap().as_deref(), Some("s3cret"));
}

#[test]
fn a_fresh_open_wipes_the_file() {
    let data = tempfile::tempdir().unwrap();
    FileSecrets::open(data.path(), false)
        .unwrap()
        .set(&key(), "s3cret")
        .unwrap();
    let fresh = FileSecrets::open(data.path(), false).unwrap();
    assert_eq!(fresh.get(&key()).unwrap(), None);
    assert!(!data.path().join(FILE_NAME).exists());
}

#[test]
fn a_reused_open_without_a_file_is_empty() {
    let data = tempfile::tempdir().unwrap();
    let store = FileSecrets::open(data.path(), true).unwrap();
    assert_eq!(store.get(&key()).unwrap(), None);
}

#[test]
fn a_delete_is_kept_across_a_reused_open() {
    let data = tempfile::tempdir().unwrap();
    let store = FileSecrets::open(data.path(), false).unwrap();
    store.set(&key(), "s3cret").unwrap();
    store.delete(&key()).unwrap();
    store.delete(&key()).unwrap();
    let again = FileSecrets::open(data.path(), true).unwrap();
    assert_eq!(again.get(&key()).unwrap(), None);
}

#[test]
fn keys_differing_in_one_part_stay_apart() {
    let data = tempfile::tempdir().unwrap();
    let store = FileSecrets::open(data.path(), false).unwrap();
    store.set(&key(), "a").unwrap();
    store
        .set(&SecretKey::new("/collections/demo", "prod", "token"), "b")
        .unwrap();
    let again = FileSecrets::open(data.path(), true).unwrap();
    assert_eq!(again.get(&key()).unwrap().as_deref(), Some("a"));
    assert_eq!(
        again
            .get(&SecretKey::new("/collections/demo", "prod", "token"))
            .unwrap()
            .as_deref(),
        Some("b")
    );
}

#[test]
fn a_corrupt_file_fails_a_reused_open() {
    let data = tempfile::tempdir().unwrap();
    std::fs::write(data.path().join(FILE_NAME), "not json").unwrap();
    let error = FileSecrets::open(data.path(), true).unwrap_err();
    assert!(format!("{error:#}").contains(FILE_NAME), "{error:#}");
}

#[test]
fn debug_shows_no_value() {
    let data = tempfile::tempdir().unwrap();
    let store = FileSecrets::open(data.path(), false).unwrap();
    store.set(&key(), "s3cret").unwrap();
    assert!(!format!("{store:?}").contains("s3cret"));
}

/// Saves one persistent cookie through a jar keyed by `store`, as the app does.
fn save_cookie(data: &std::path::Path, store: Arc<dyn SecretStore>) {
    let jar = CookieJar::open(data.join("cookies.json"), store);
    jar.load();
    let mut cookie = CookieInfo::new("127.0.0.1", "sid", "abc");
    cookie.expires = Some(SystemTime::now() + Duration::from_secs(3600));
    jar.insert(cookie).unwrap();
    jar.save().unwrap();
}

fn cookie_names(data: &std::path::Path, store: Arc<dyn SecretStore>) -> Vec<String> {
    let jar = CookieJar::open(data.join("cookies.json"), store);
    jar.load();
    jar.list().into_iter().map(|c| c.name).collect()
}

#[test]
fn a_reused_open_keeps_the_cookie_jar() {
    let data = tempfile::tempdir().unwrap();
    save_cookie(
        data.path(),
        Arc::new(FileSecrets::open(data.path(), false).unwrap()),
    );
    let again = Arc::new(FileSecrets::open(data.path(), true).unwrap());
    assert_eq!(cookie_names(data.path(), again), vec!["sid".to_owned()]);
}

#[test]
fn a_fresh_open_loses_the_cookie_jar() {
    let data = tempfile::tempdir().unwrap();
    save_cookie(
        data.path(),
        Arc::new(FileSecrets::open(data.path(), false).unwrap()),
    );
    let fresh = Arc::new(FileSecrets::open(data.path(), false).unwrap());
    assert!(cookie_names(data.path(), fresh).is_empty());
}

#[test]
fn the_launch_store_follows_reused() {
    let data = tempfile::tempdir().unwrap();
    apinata_pilot::launch_secrets(data.path(), false)
        .unwrap()
        .set(&key(), "s3cret")
        .unwrap();
    let kept = apinata_pilot::launch_secrets(data.path(), true).unwrap();
    assert_eq!(kept.get(&key()).unwrap().as_deref(), Some("s3cret"));
    let fresh = apinata_pilot::launch_secrets(data.path(), false).unwrap();
    assert_eq!(fresh.get(&key()).unwrap(), None);
}

#[test]
fn a_corrupt_file_error_quotes_no_value() {
    let data = tempfile::tempdir().unwrap();
    let body =
        r#"[{"collection":"/c","environment":"e","variable":"v","value":"s3cret"},"s3cret2"]"#;
    std::fs::write(data.path().join(FILE_NAME), body).unwrap();
    let error = FileSecrets::open(data.path(), true).unwrap_err();
    let text = format!("{error:#} {error:?}");
    assert!(text.contains(FILE_NAME), "{text}");
    assert!(!text.contains("s3cret"), "{text}");
}

#[test]
fn a_fresh_open_removes_a_stale_temp_file() {
    let data = tempfile::tempdir().unwrap();
    let tmp = data.path().join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, "[]").unwrap();
    FileSecrets::open(data.path(), false).unwrap();
    assert!(!tmp.exists());
}
