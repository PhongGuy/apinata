//! The context's default timeout fills requests that inherit theirs.

use crate::common;

use apinata_core::import::{add_request, parse_curl, parse_postman, write_collection};
use apinata_core::{
    ContextOptions, ExecError, ItemPath, MemoryStore, RequestKind, VarContext, resolve,
};
use common::{at, request, setup};
use std::fs;
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn http(url: &str, top: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{top}")
}

#[test]
fn inherited_timeouts_take_the_default_and_written_ones_win() {
    let u = "https://x.test/";
    let files = [
        ("opencollection.yml", ROOT.to_owned()),
        ("absent.yml", http(u, "")),
        (
            "other.yml",
            http(u, "settings:\n  followRedirects: false\n"),
        ),
        ("null.yml", http(u, "settings:\n  timeout: null\n")),
        ("inherit.yml", http(u, "settings:\n  timeout: inherit\n")),
        ("zero.yml", http(u, "settings:\n  timeout: 0\n")),
        ("negative.yml", http(u, "settings:\n  timeout: -5\n")),
        ("set.yml", http(u, "settings:\n  timeout: 2500\n")),
        ("junk.yml", http(u, "settings:\n  timeout: \"100\"\n")),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let (_dir, c, plain) = setup(&files);
    let seven = Some(Duration::from_secs(7));
    let ctx = plain.with_default_timeout(seven);
    let t = |rel: &str| at(&c, &ctx, rel).expect(rel).timeout();
    assert_eq!(t("absent.yml"), seven);
    assert_eq!(t("other.yml"), seven);
    assert_eq!(t("null.yml"), seven);
    assert_eq!(t("inherit.yml"), seven);
    assert_eq!(t("zero.yml"), None);
    assert_eq!(t("negative.yml"), None);
    assert_eq!(t("set.yml"), Some(Duration::from_millis(2500)));
    assert_eq!(t("junk.yml"), None);
    // Without a default, and with an explicit None, nothing changes from today.
    assert_eq!(at(&c, &plain, "absent.yml").unwrap().timeout(), None);
    let none = plain.with_default_timeout(None);
    assert_eq!(at(&c, &none, "inherit.yml").unwrap().timeout(), None);
    // The default lives on the clone only.
    assert_eq!(at(&c, &plain, "inherit.yml").unwrap().timeout(), None);
}

// Every way a request file comes to exist without a timeout of its own writes `inherit`, so
// the default reaches it rather than the explicit no limit `0` would mean.
#[test]
fn new_and_imported_requests_take_the_default_timeout() {
    let (dir, mut c, _) = setup(&[("opencollection.yml", ROOT)]);
    let root = ItemPath::root();
    // A URL is set on each so it resolves; the settings stay as created.
    let made = c.create_request(&root, "made").expect("creates");
    let query = c
        .create_request_of_kind(&root, "query", RequestKind::Graphql)
        .expect("creates");
    for item in [made, query] {
        let r = c.request_mut(&item).expect("created");
        r.set_url("https://x.test/n");
        r.save().expect("saves");
    }
    let draft = c.create_draft(&root).expect("drafts");
    c.request_mut(&draft)
        .expect("draft")
        .set_url("https://x.test/d");
    c.save_draft(&draft, &root, "drafted").expect("saves");
    let curl = parse_curl("curl https://x.test/a").expect("parses");
    add_request(&mut c, &root, "curl", &curl.request).expect("imports");
    let postman = parse_postman(
        br#"{"info": {"name": "p", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
            "item": [{"name": "r", "request": {"method": "GET", "url": "https://x.test/p"}}]}"#,
    )
    .expect("parses");
    let (imported, _) =
        write_collection(dir.path(), &postman, &[], &MemoryStore::new()).expect("writes");
    let seven = Some(Duration::from_secs(7));
    for (c, rel) in [
        (&c, "made.yml"),
        (&c, "query.yml"),
        (&c, "drafted.yml"),
        (&c, "curl.yml"),
        (&imported, "r.yml"),
    ] {
        let text = fs::read_to_string(c.root().join(rel)).expect(rel);
        assert!(text.contains("\n  timeout: inherit\n"), "{rel}:\n{text}");
        let ctx = VarContext::new(c, ContextOptions::new())
            .expect("context")
            .with_default_timeout(seven);
        let resolved = resolve(request(c, rel), &ctx).expect(rel);
        assert_eq!(resolved.timeout(), seven, "{rel}");
    }
}

#[tokio::test]
async fn the_default_limits_a_send_to_a_slow_server() {
    let server = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
        .mount(&server)
        .await;
    let file = http(&format!("{}/slow", server.uri()), "");
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("r.yml", &file)]);
    let ctx = ctx.with_default_timeout(Some(Duration::from_millis(100)));
    let req = at(&c, &ctx, "r.yml").expect("resolves");
    let err = common::executor().send(&req).await.expect_err("times out");
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
}
