//! The on-disk schema cache: restarts, hashed file names, scrubbing, caps and eviction.

use crate::support;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use apinata_app::graphql_schema_store::{SchemaState, SchemaStore, state};
use apinata_app::request_editor::Section;
use apinata_app::schema_cache::SchemaCache;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};
use support::*;
use wiremock::ResponseTemplate;

const SECRET: &str = "s3cret-value-42";

fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../apinata-core/tests/fixtures/graphql/schema.json"
    ))
    .unwrap()
}

// The fixture with a description that echoes `SECRET`, as a server reflecting a credential would.
fn echoing_fixture() -> String {
    let text = fixture().replace("The root query.", &format!("The root query for {SECRET}."));
    assert!(text.contains(SECRET));
    text
}

fn cache_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(read) => read.map(|e| e.unwrap().path()).collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    files
}

fn age(path: &Path, seconds_ago: u64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(seconds_ago))
        .unwrap();
}

#[test]
fn a_stored_schema_reads_back_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let url = "http://api.test/graphql";
    assert!(
        SchemaCache::new(dir.path().to_path_buf())
            .store(url, &fixture(), &[])
            .unwrap()
    );
    let fresh = SchemaCache::new(dir.path().to_path_buf());
    let schema = fresh.load(url).expect("the kept schema");
    assert_eq!(schema.query_type(), Some("Query"));
    assert!(fresh.load("http://other.test/graphql").is_none());
}

#[test]
fn file_names_hash_the_url_and_no_secret_reaches_disk() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("https://user:hunter2@api.test/graphql?token={SECRET}");
    let cache = SchemaCache::new(dir.path().to_path_buf());
    assert!(
        cache
            .store(&url, &echoing_fixture(), &[SECRET.to_owned()])
            .unwrap()
    );
    let files = cache_files(dir.path());
    assert_eq!(files.len(), 1);
    let name = files[0].file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(name, SchemaCache::file_name(&url));
    let stem = name.strip_suffix(".json").expect("a json file");
    assert_eq!(stem.len(), 64);
    assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
    let content = std::fs::read_to_string(&files[0]).unwrap();
    for leak in [SECRET, "hunter2", "api.test", "token="] {
        assert!(!content.contains(leak), "{leak} reached the cache file");
    }
    assert!(content.contains("[redacted]"));
    assert!(cache.load(&url).is_some());
    assert_ne!(
        SchemaCache::file_name(&url),
        SchemaCache::file_name("https://api.test/graphql")
    );
}

#[test]
fn a_corrupt_file_is_ignored_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let cache = SchemaCache::new(dir.path().to_path_buf());
    for (url, junk) in [
        ("http://a.test/graphql", "{ not json".to_owned()),
        (
            "http://b.test/graphql",
            r#"{"version":1,"text":"<html></html>"}"#.to_owned(),
        ),
        (
            "http://c.test/graphql",
            r#"{"version":99,"text":""}"#.to_owned(),
        ),
        (
            "http://d.test/graphql",
            String::from_utf8_lossy(&[0xff, 0xfe]).into(),
        ),
    ] {
        let path = dir.path().join(SchemaCache::file_name(url));
        std::fs::write(&path, junk).unwrap();
        assert!(cache.load(url).is_none(), "{url} loaded");
        assert!(!path.exists(), "{url} was kept");
    }
}

#[test]
fn the_per_schema_cap_is_enforced_on_write_and_read() {
    let dir = tempfile::tempdir().unwrap();
    let url = "http://big.test/graphql";
    let small = SchemaCache::with_limits(dir.path().to_path_buf(), 1024, 1 << 20);
    assert!(!small.store(url, &fixture(), &[]).unwrap());
    assert!(cache_files(dir.path()).is_empty());
    // A file written under a larger cap is too big for this one: dropped and removed on read.
    let roomy = SchemaCache::with_limits(dir.path().to_path_buf(), 1 << 20, 1 << 20);
    assert!(roomy.store(url, &fixture(), &[]).unwrap());
    assert!(small.load(url).is_none());
    assert!(cache_files(dir.path()).is_empty());
    // An oversized refresh does not leave the older schema behind.
    assert!(roomy.store(url, &fixture(), &[]).unwrap());
    assert!(!small.store(url, &fixture(), &[]).unwrap());
    assert!(roomy.load(url).is_none());
}

#[test]
fn the_total_cap_evicts_the_oldest_schemas_first() {
    let dir = tempfile::tempdir().unwrap();
    let one = SchemaCache::new(dir.path().to_path_buf());
    one.store("http://a.test/", &fixture(), &[]).unwrap();
    let size = std::fs::metadata(dir.path().join(SchemaCache::file_name("http://a.test/")))
        .unwrap()
        .len();
    // Room for two files, not three.
    let cache = SchemaCache::with_limits(dir.path().to_path_buf(), size, size * 2 + size / 2);
    cache.store("http://b.test/", &fixture(), &[]).unwrap();
    age(
        &dir.path().join(SchemaCache::file_name("http://a.test/")),
        300,
    );
    age(
        &dir.path().join(SchemaCache::file_name("http://b.test/")),
        100,
    );
    cache.store("http://c.test/", &fixture(), &[]).unwrap();
    assert_eq!(cache_files(dir.path()).len(), 2);
    assert!(cache.load("http://a.test/").is_none());
    assert!(cache.load("http://b.test/").is_some());
    assert!(cache.load("http://c.test/").is_some());
}

#[test]
fn a_schema_scrubbing_breaks_is_not_kept() {
    let dir = tempfile::tempdir().unwrap();
    let cache = SchemaCache::new(dir.path().to_path_buf());
    cache.store("http://q.test/", &fixture(), &[]).unwrap();
    // A secret equal to `__schema` scrubs the answer into one that no longer reads as a schema,
    // and the older kept copy goes too, so a restart never shows a stale schema.
    let kept = cache
        .store("http://q.test/", &fixture(), &["__schema".to_owned()])
        .unwrap();
    assert!(!kept);
    assert!(cache_files(dir.path()).is_empty());
}

fn gql(url: &str) -> String {
    format!("info:\n  name: a\n  type: graphql\n  seq: 1\ngraphql:\n  url: \"{url}\"\n")
}

fn query_tab(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
}

fn loaded(cx: &mut TestAppContext, url: &str) -> bool {
    matches!(cx.update(|cx| state(cx, url)), Some(SchemaState::Loaded(_)))
}

fn use_cache(cx: &mut TestAppContext, dir: &Path) {
    let cache = SchemaCache::new(dir.to_path_buf());
    cx.update(|cx| cx.set_global(SchemaStore::with_cache(cache)));
}

// A fetched schema lands on disk scrubbed of the request's secrets, and a fresh store (a
// restart) shows it on first use without contacting the server again.
#[gpui_kit::test]
fn a_fetched_schema_is_kept_scrubbed_and_restored_after_a_restart(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(
        cx,
        ResponseTemplate::new(200).set_body_string(echoing_fixture()),
    );
    let url = format!("{}/graphql?key={{{{process.env.KEY}}}}", server.uri());
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (".env", &format!("KEY={SECRET}\n")),
        ("a.yml", &gql(&url)),
    ]);
    let data = tempfile::tempdir().unwrap();
    use_cache(cx, data.path());
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _) = editor_for(cx, &ws, &root, "a.yml", false);
    query_tab(cx, win);
    assert!(present(cx, win, "graphql-schema-load"));
    cx.update_window(win, |_, w, cx| w.click("graphql-schema-load", cx))
        .unwrap();
    wait_real(cx, |cx| loaded(cx, &url));
    wait_real(cx, |_| cache_files(data.path()).len() == 1);
    let file = &cache_files(data.path())[0];
    assert_eq!(
        file.file_name().unwrap().to_str().unwrap(),
        SchemaCache::file_name(&url)
    );
    let content = std::fs::read_to_string(file).unwrap();
    assert!(!content.contains(SECRET));
    assert!(content.contains("[redacted]"));
    assert_eq!(served(cx, &server).len(), 1);

    use_cache(cx, data.path());
    assert!(cx.update(|cx| state(cx, &url)).is_none());
    let (win, _) = editor_for(cx, &ws, &root, "a.yml", false);
    query_tab(cx, win);
    wait_real(cx, |cx| loaded(cx, &url));
    cx.update_window(win, |_, w, cx| w.render_frame(cx))
        .unwrap();
    assert!(present(cx, win, "graphql-schema-refresh"));
    assert_eq!(served(cx, &server).len(), 1, "the restart fetched again");
}

// A schema file picked for a URL is kept too, so it is back after a restart.
#[gpui_kit::test]
fn a_file_loaded_schema_is_kept(cx: &mut TestAppContext) {
    init(cx);
    let data = tempfile::tempdir().unwrap();
    use_cache(cx, data.path());
    let file = tempfile::tempdir().unwrap();
    let path = file.path().join("s.graphql");
    std::fs::write(&path, "type Query { me: String }").unwrap();
    cx.update(|cx| apinata_app::schema_load::load_from_file("u".into(), path, cx));
    wait_for(cx, "the file load", |cx| loaded(cx, "u"));
    wait_for(cx, "the cache write", |_| {
        cache_files(data.path()).len() == 1
    });
    let schema = SchemaCache::new(data.path().to_path_buf())
        .load("u")
        .expect("the kept file schema");
    assert_eq!(schema.query_type(), Some("Query"));
}

// Without a cache the store never touches disk; with one, a user's load wins over a slow read.
#[test]
fn a_restore_never_overwrites_a_started_load() {
    let dir = tempfile::tempdir().unwrap();
    let cache = SchemaCache::new(dir.path().to_path_buf());
    cache.store("u", &fixture(), &[]).unwrap();
    let mut plain = SchemaStore::default();
    assert!(plain.probe("u").is_none());
    let mut s = SchemaStore::with_cache(cache);
    let probe = s.probe("u").expect("the first probe");
    assert!(s.probe("u").is_none(), "a second probe of the same URL");
    let generation = s.begin("u");
    assert!(!s.restore("u", probe.load("u").unwrap()));
    assert!(matches!(s.state("u"), Some(SchemaState::Loading)));
    assert!(s.finish("u", generation, Err("boom".into())));
    let other = s.probe("v").expect("another URL");
    assert!(other.load("v").is_none());
}
