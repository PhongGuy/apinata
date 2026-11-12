//! The cookie jar's own API: adding cookies with validation, and saving them sealed.

use apinata_core::{CookieError, CookieInfo, CookieJar, SameSite};
use std::time::{Duration, SystemTime};

fn secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs()
}

/// The field an insert refused, after checking nothing was stored.
fn refused(cookie: CookieInfo) -> &'static str {
    let jar = CookieJar::default();
    let field = match jar.insert(cookie) {
        Err(CookieError::Invalid { field, .. }) => field,
        other => panic!("accepted or another error: {other:?}"),
    };
    assert!(jar.list().is_empty());
    field
}

#[test]
fn new_is_a_host_only_session_cookie_at_the_root() {
    let c = CookieInfo::new("a.test", "sid", "v");
    assert_eq!(
        (
            c.domain.as_str(),
            c.path.as_str(),
            c.name.as_str(),
            c.value.as_str()
        ),
        ("a.test", "/", "sid", "v")
    );
    assert!(c.host_only && !c.secure && !c.http_only && c.expires.is_none());
}

#[test]
fn an_inserted_cookie_is_listed_with_every_field() {
    let jar = CookieJar::default();
    let at = SystemTime::now() + Duration::from_secs(3600);
    let mut c = CookieInfo::new("A.Test", "sid", "");
    c.path = "/api".into();
    c.expires = Some(at);
    c.secure = true;
    c.http_only = true;
    jar.insert(c).expect("valid");
    let listed = jar.list();
    assert_eq!(listed.len(), 1);
    let got = &listed[0];
    assert_eq!(
        (
            got.domain.as_str(),
            got.path.as_str(),
            got.name.as_str(),
            got.value.as_str()
        ),
        ("a.test", "/api", "sid", "")
    );
    assert!(got.secure && got.http_only && got.host_only);
    assert_eq!(got.expires.map(secs), Some(secs(at)));
}

#[test]
fn insert_replaces_the_same_domain_path_and_name_only() {
    let jar = CookieJar::default();
    jar.insert(CookieInfo::new("a.test", "sid", "1")).unwrap();
    jar.insert(CookieInfo::new("a.test", "sid", "2")).unwrap();
    let mut deeper = CookieInfo::new("a.test", "sid", "3");
    deeper.path = "/x".into();
    jar.insert(deeper).unwrap();
    let values: Vec<(String, String)> = jar.list().into_iter().map(|c| (c.path, c.value)).collect();
    assert_eq!(
        values,
        [
            ("/".to_owned(), "2".to_owned()),
            ("/x".to_owned(), "3".to_owned())
        ]
    );
}

#[test]
fn a_leading_dot_or_host_only_false_makes_the_cookie_domain_wide() {
    let jar = CookieJar::default();
    jar.insert(CookieInfo::new(".Wide.test", "a", "1")).unwrap();
    let mut b = CookieInfo::new("other.test", "b", "2");
    b.host_only = false;
    jar.insert(b).unwrap();
    let shapes: Vec<(String, bool)> = jar
        .list()
        .into_iter()
        .map(|c| (c.domain, c.host_only))
        .collect();
    assert_eq!(
        shapes,
        [
            ("other.test".to_owned(), false),
            ("wide.test".to_owned(), false)
        ]
    );
}

#[test]
fn every_invalid_field_is_named_and_nothing_is_stored() {
    let with = |f: fn(&mut CookieInfo)| {
        let mut c = CookieInfo::new("a.test", "sid", "v");
        f(&mut c);
        c
    };
    for domain in ["", ".", "exa mple.test", "a.test/x", "a.test:80"] {
        assert_eq!(
            refused(CookieInfo::new(domain, "sid", "v")),
            "domain",
            "{domain:?}"
        );
    }
    assert_eq!(refused(with(|c| c.path = "api".into())), "path");
    for name in ["", "a b", "a;b", "a=b", "a\u{7}"] {
        assert_eq!(
            refused(CookieInfo::new("a.test", name, "v")),
            "name",
            "{name:?}"
        );
    }
    for value in ["a;b", "a\nb"] {
        assert_eq!(
            refused(CookieInfo::new("a.test", "sid", value)),
            "value",
            "{value:?}"
        );
    }
    assert_eq!(
        refused(with(
            |c| c.expires = Some(SystemTime::now() - Duration::from_secs(1))
        )),
        "expires"
    );
}

#[test]
fn the_error_names_the_field() {
    let err = CookieJar::default()
        .insert(CookieInfo::new("a.test", "a b", "v"))
        .unwrap_err();
    assert!(
        err.to_string().starts_with("invalid cookie name: "),
        "{err}"
    );
}

#[test]
fn an_ip_address_is_a_valid_host() {
    let jar = CookieJar::default();
    jar.insert(CookieInfo::new("127.0.0.1", "sid", "v"))
        .unwrap();
    assert_eq!(jar.list()[0].domain, "127.0.0.1");
}

use apinata_core::{MemoryStore, Persistence, SecretError, SecretKey, SecretStore};
use std::sync::Arc;
use tempfile::TempDir;

fn opened(dir: &TempDir, keys: Arc<dyn SecretStore>) -> CookieJar {
    let jar = CookieJar::open(dir.path().join("cookies.json"), keys);
    jar.load();
    jar
}

fn names(jar: &CookieJar) -> Vec<(String, String)> {
    jar.list().into_iter().map(|c| (c.name, c.value)).collect()
}

fn pair(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

struct Locked;
impl SecretStore for Locked {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "locked".into(),
        })
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
}

/// A store whose reads take 300 ms, like a keychain prompt.
struct Slow(Arc<dyn SecretStore>);
impl SecretStore for Slow {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        std::thread::sleep(Duration::from_millis(300));
        self.0.get(key)
    }
    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.0.set(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.0.delete(key)
    }
}

#[test]
fn the_default_jar_is_memory_only() {
    let jar = CookieJar::default();
    assert_eq!(
        jar.persistence(),
        Persistence::MemoryOnly {
            reason: "cookies are not saved".into()
        }
    );
    jar.load();
    jar.insert(CookieInfo::new("a.test", "sid", "1")).unwrap();
    assert_eq!(jar.save(), Ok(()));
}

#[test]
fn nothing_is_read_or_written_before_load() {
    let dir = TempDir::new().unwrap();
    let keys = Arc::new(MemoryStore::new());
    let jar = CookieJar::open(dir.path().join("cookies.json"), keys.clone());
    assert_eq!(jar.persistence(), Persistence::NotLoaded);
    jar.insert(CookieInfo::new("a.test", "sid", "1")).unwrap();
    assert_eq!(jar.save(), Ok(()));
    assert!(!dir.path().join("cookies.json").exists());
    assert_eq!(
        keys.get(&SecretKey::new("", "", "cookie-jar-key")).unwrap(),
        None
    );
}

#[test]
fn save_then_load_round_trips_every_field_and_the_file_is_sealed() {
    let dir = TempDir::new().unwrap();
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let jar = opened(&dir, keys.clone());
    assert_eq!(jar.persistence(), Persistence::Encrypted);
    jar.insert(CookieInfo::new("host-k2.test", "sessid-Q7", "v4lue-Z9"))
        .unwrap();
    let mut wide = CookieInfo::new(".wide-m3.test", "pref", "");
    wide.path = "/api".into();
    wide.expires = Some(SystemTime::now() + Duration::from_secs(86_400));
    wide.secure = true;
    wide.http_only = true;
    jar.insert(wide).unwrap();
    jar.save().expect("saved");

    let text = std::fs::read_to_string(dir.path().join("cookies.json")).unwrap();
    for clear in ["sessid-Q7", "v4lue-Z9", "host-k2", "wide-m3"] {
        assert!(!text.contains(clear), "{clear} in {text}");
    }
    let body: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["version"], 1);
    assert!(
        keys.get(&SecretKey::new("", "", "cookie-jar-key"))
            .unwrap()
            .is_some()
    );
    assert!(
        keys.get(&SecretKey::new("", "", "oauth2-token-key"))
            .unwrap()
            .is_none()
    );

    let shape = |c: CookieInfo| {
        (
            c.domain,
            c.path,
            c.name,
            c.value,
            c.expires.map(secs),
            c.secure,
            c.http_only,
            c.host_only,
        )
    };
    let before: Vec<_> = jar.list().into_iter().map(shape).collect();
    let after: Vec<_> = opened(&dir, keys).list().into_iter().map(shape).collect();
    assert_eq!(after, before);
    assert_eq!(after.len(), 2);
}

#[test]
fn a_wrong_key_or_garbage_loads_empty_and_the_next_save_replaces_it() {
    let dir = TempDir::new().unwrap();
    let first = opened(&dir, Arc::new(MemoryStore::new()));
    first.insert(CookieInfo::new("a.test", "sid", "1")).unwrap();
    first.save().unwrap();
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let other = opened(&dir, keys.clone());
    assert_eq!(other.persistence(), Persistence::Encrypted);
    assert!(other.list().is_empty());
    other
        .insert(CookieInfo::new("b.test", "mine", "2"))
        .unwrap();
    other.save().unwrap();
    assert_eq!(names(&opened(&dir, keys.clone())), [pair("mine", "2")]);
    std::fs::write(dir.path().join("cookies.json"), "not json").unwrap();
    assert!(opened(&dir, keys).list().is_empty());
}

#[test]
fn without_a_keychain_the_jar_is_memory_only_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let jar = opened(&dir, Arc::new(Locked));
    match jar.persistence() {
        Persistence::MemoryOnly { reason } => assert!(reason.contains("locked"), "{reason}"),
        other => panic!("{other:?}"),
    }
    jar.insert(CookieInfo::new("a.test", "sid", "1")).unwrap();
    assert_eq!(jar.save(), Ok(()));
    assert!(!dir.path().join("cookies.json").exists());
    assert_eq!(names(&jar), [pair("sid", "1")]);
}

#[test]
fn a_malformed_stored_key_is_memory_only_and_kept() {
    let dir = TempDir::new().unwrap();
    let keys = Arc::new(MemoryStore::new());
    let name = SecretKey::new("", "", "cookie-jar-key");
    keys.set(&name, "not a key").unwrap();
    let jar = opened(&dir, keys.clone());
    assert_eq!(
        jar.persistence(),
        Persistence::MemoryOnly {
            reason: "the stored cookie jar key is not valid".into()
        }
    );
    assert_eq!(keys.get(&name).unwrap().as_deref(), Some("not a key"));
}

#[test]
fn save_writes_only_after_a_change() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cookies.json");
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let jar = opened(&dir, keys.clone());
    jar.insert(CookieInfo::new("a.test", "sid", "1")).unwrap();
    jar.save().unwrap();
    let first = std::fs::read(&path).unwrap();
    jar.save().unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        first,
        "an unchanged jar rewrote the file"
    );
    let again = opened(&dir, keys);
    again.save().unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        first,
        "a load alone is not a change"
    );
    assert!(again.delete("a.test", "/", "sid"));
    again.save().unwrap();
    assert_ne!(std::fs::read(&path).unwrap(), first);
}

#[test]
fn an_expired_cookie_is_not_loaded() {
    let dir = TempDir::new().unwrap();
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let jar = opened(&dir, keys.clone());
    let mut brief = CookieInfo::new("a.test", "brief", "1");
    brief.expires = Some(SystemTime::now() + Duration::from_secs(2));
    jar.insert(brief).unwrap();
    jar.insert(CookieInfo::new("a.test", "session", "2"))
        .unwrap();
    jar.save().unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(names(&opened(&dir, keys)), [pair("session", "2")]);
}

#[test]
fn a_failed_write_is_returned_and_the_next_save_retries() {
    let dir = TempDir::new().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "a file where a directory should be").unwrap();
    let path = blocker.join("cookies.json");
    let jar = CookieJar::open(path.clone(), Arc::new(MemoryStore::new()));
    jar.load();
    jar.insert(CookieInfo::new("a.test", "sid", "1")).unwrap();
    let err = jar.save().unwrap_err();
    assert!(
        err.starts_with("the cookie jar could not be written"),
        "{err}"
    );
    assert_eq!(jar.persistence(), Persistence::Encrypted);
    std::fs::remove_file(&blocker).unwrap();
    jar.save().expect("retried");
    assert!(path.exists());
}

#[test]
fn a_cookie_set_before_load_wins_over_the_saved_copy() {
    let dir = TempDir::new().unwrap();
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let jar = opened(&dir, keys.clone());
    jar.insert(CookieInfo::new("a.test", "sid", "old")).unwrap();
    jar.insert(CookieInfo::new("a.test", "other", "kept"))
        .unwrap();
    jar.save().unwrap();
    let fresh = CookieJar::open(dir.path().join("cookies.json"), keys.clone());
    fresh
        .insert(CookieInfo::new("a.test", "sid", "new"))
        .unwrap();
    fresh.load();
    assert_eq!(names(&fresh), [pair("other", "kept"), pair("sid", "new")]);
    fresh.save().unwrap();
    assert_eq!(
        names(&opened(&dir, keys)),
        [pair("other", "kept"), pair("sid", "new")]
    );
}

#[test]
fn concurrent_saves_leave_the_newest_snapshot_on_disk() {
    let dir = TempDir::new().unwrap();
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let jar = opened(&dir, keys.clone());
    std::thread::scope(|s| {
        for t in 0..8 {
            let jar = &jar;
            s.spawn(move || {
                for i in 0..10 {
                    jar.insert(CookieInfo::new("race.test", format!("c{t}-{i}"), "v"))
                        .unwrap();
                    jar.save().expect("saved");
                }
            });
        }
    });
    assert_eq!(opened(&dir, keys).list().len(), 80);
}

#[test]
fn a_load_during_another_waits_for_it_and_later_loads_do_nothing() {
    let dir = TempDir::new().unwrap();
    let keys: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let jar = Arc::new(CookieJar::open(
        dir.path().join("cookies.json"),
        Arc::new(Slow(keys.clone())),
    ));
    let first = {
        let jar = jar.clone();
        std::thread::spawn(move || jar.load())
    };
    std::thread::sleep(Duration::from_millis(50));
    jar.load();
    assert_eq!(
        jar.persistence(),
        Persistence::Encrypted,
        "returned before the first load finished"
    );
    first.join().unwrap();
    let other = opened(&dir, keys);
    other
        .insert(CookieInfo::new("late.test", "late", "1"))
        .unwrap();
    other.save().unwrap();
    jar.load();
    assert!(jar.list().is_empty(), "a second load read the file again");
}

#[test]
fn check_names_every_invalid_field_in_dialog_order_and_stores_nothing() {
    let mut c = CookieInfo::new("a b", "x y", "v");
    c.path = "no-slash".into();
    c.expires = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
    let fields: Vec<_> = CookieJar::check(&c)
        .into_iter()
        .map(|e| match e {
            CookieError::Invalid { field, .. } => field,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(fields, ["domain", "path", "name", "expires"]);
    assert!(CookieJar::check(&CookieInfo::new("a.test", "sid", "v")).is_empty());
}

#[test]
fn an_inserted_same_site_is_listed_and_round_trips_a_plain_file() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("jar.json");
    let jar = CookieJar::default();
    for (name, same_site) in [
        ("strict", Some(SameSite::Strict)),
        ("lax", Some(SameSite::Lax)),
        ("none", Some(SameSite::None)),
        ("unset", None),
    ] {
        let mut c = CookieInfo::new("same-site.test", name, "1");
        c.same_site = same_site;
        jar.insert(c).unwrap();
    }
    let shape = |jar: &CookieJar| -> Vec<(String, Option<SameSite>)> {
        jar.list()
            .into_iter()
            .map(|c| (c.name, c.same_site))
            .collect()
    };
    let listed = shape(&jar);
    assert_eq!(
        listed,
        [
            ("lax".to_owned(), Some(SameSite::Lax)),
            ("none".to_owned(), Some(SameSite::None)),
            ("strict".to_owned(), Some(SameSite::Strict)),
            ("unset".to_owned(), None),
        ]
    );
    jar.write_file(&file).unwrap();
    let again = CookieJar::default();
    again.read_file(&file).unwrap();
    assert_eq!(shape(&again), listed);
}

#[test]
fn a_file_without_same_site_or_with_an_unknown_one_loads_it_unset() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("jar.json");
    let row = |name: &str, extra: &str| {
        format!(
            r#"{{"domain":"old.test","path":"/","name":"{name}","value":"1","expires":null,"secure":false,"httpOnly":false,"hostOnly":true{extra}}}"#
        )
    };
    let text = format!(
        "[{},{}]",
        row("old", ""),
        row("odd", r#","sameSite":"Sometimes""#)
    );
    std::fs::write(&file, text).unwrap();
    let jar = CookieJar::default();
    jar.read_file(&file).unwrap();
    let listed: Vec<_> = jar
        .list()
        .into_iter()
        .map(|c| (c.name, c.same_site))
        .collect();
    assert_eq!(listed, [("odd".to_owned(), None), ("old".to_owned(), None)]);
}
