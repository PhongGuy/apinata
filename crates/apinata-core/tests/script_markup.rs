//! The cheerio and xml2js bundles, and the `xml2Json` global, load lazily and behave as their published versions.

use crate::script_support;

use apinata_core::{Outcome, RunOptions, ScriptOptions};
use script_support::*;
use sha2::Digest;
use std::time::{Duration, Instant};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn run_with(code: &str, body: &str, opts: RunOptions) -> apinata_core::RequestResult {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body.to_owned(), "text/html"))
        .mount(&s)
        .await;
    let text = scripted("R", &format!("{}/r", s.uri()), &script("tests", code), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    send(dir.path(), "r.yml", opts).await
}

async fn run_code(code: &str) -> apinata_core::RequestResult {
    let result = run_with(code, "", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", tests(&result));
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cheerio_loads_selects_and_mutates_a_whole_document() {
    let code = r#"console.log(typeof Object.getOwnPropertyDescriptor(globalThis, "cheerio").get);
const c = require("cheerio");
const $ = c.load('<h2 class="title">Hello world</h2><p>a</p><p>b</p>');
$("h2.title").text("Hello there!");
$("h2").addClass("welcome");
console.log($.html());
console.log($("p").length, $("p").last().text(), $("h2").hasClass("welcome"));
console.log(c === cheerio, typeof c.load, typeof c.contains, typeof c.merge);
for (const name of ["fromURL", "loadBuffer", "stringStream", "decodeStream"]) {
  try { c[name]("http://127.0.0.1:1/"); } catch (e) { console.log(e.message); }
}"#;
    let result = run_code(code).await;
    assert_eq!(
        console(&result),
        [
            "log:function",
            r#"log:<html><head></head><body><h2 class="title welcome">Hello there!</h2><p>a</p><p>b</p></body></html>"#,
            "log:2 b true",
            "log:true function function function",
            "log:cheerio.fromURL is not available here",
            "log:cheerio.loadBuffer is not available here",
            "log:cheerio.stringStream is not available here",
            "log:cheerio.decodeStream is not available here",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xml2js_parses_synchronously_by_promise_and_builds() {
    let code = r#"const xml2js = require("xml2js");
let parsed;
xml2js.parseString('<root a="1"><b>x</b></root>', (err, res) => { parsed = res; });
console.log(JSON.stringify(parsed));
const p = await xml2js.parseStringPromise("<a><n>1</n></a>", { explicitArray: false });
console.log(JSON.stringify(p));
console.log(JSON.stringify(new xml2js.Builder().buildObject({ root: { a: "1" } })));
console.log(typeof xml2js.Parser, require("xml2js") === xml2js);"#;
    let result = run_code(code).await;
    assert_eq!(
        console(&result),
        [
            r#"log:{"root":{"$":{"a":"1"},"b":["x"]}}"#,
            r#"log:{"a":{"n":"1"}}"#,
            r#"log:"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<root>\n  <a>1</a>\n</root>""#,
            "log:function true",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xml2js_async_mode_parses_in_chunks_over_the_engine_timer() {
    let code = r#"const xml2js = require("xml2js");
const xml = "<r>" + "<i>x</i>".repeat(5000) + "</r>";
const opts = { async: true, chunkSize: 1000, explicitArray: false };
let sync = "unset";
const done = new Promise((resolve, reject) => {
  new xml2js.Parser(opts).parseString(xml, (err, res) => (err ? reject(err) : resolve(res)));
});
sync = "returned first";
const res = await done;
console.log(sync, res.r.i.length);
const viaPromise = await xml2js.parseStringPromise("<a>1</a>", { async: true });
console.log(JSON.stringify(viaPromise));"#;
    let result = run_code(code).await;
    assert_eq!(
        console(&result),
        ["log:returned first 5000", r#"log:{"a":"1"}"#]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xml2json_answers_as_postman_does_for_good_bad_and_empty_xml() {
    let code = r#"console.log(typeof Object.getOwnPropertyDescriptor(globalThis, "xml2Json").get);
console.log(JSON.stringify(xml2Json('<root id="7"><name> Ann </name><tag>a</tag><tag>b</tag></root>')));
console.log(String(xml2Json("<a><b></a>")), String(xml2Json("")));"#;
    let result = run_code(code).await;
    assert_eq!(
        console(&result),
        [
            "log:function",
            r#"log:{"root":{"$":{"id":"7"},"name":"Ann","tag":["a","b"]}}"#,
            "log:undefined null",
        ]
    );
}

/// Scripts on with a 16 MiB memory limit, loose enough in time that only memory can stop them.
fn small_heap() -> RunOptions {
    RunOptions::new().scripts(
        ScriptOptions::new()
            .enabled(true)
            .memory_limit(16 << 20)
            .timeout(Duration::from_secs(20)),
    )
}

const LOAD_LIST: &str = "const $ = cheerio.load(res.getBody()); console.log($('li').length);";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_small_body_parses_under_the_16_mib_limit() {
    let body = format!("<ul>{}</ul>", "<li class=\"x\">item</li>".repeat(100));
    let result = run_with(LOAD_LIST, &body, small_heap()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", tests(&result));
    assert_eq!(console(&result), ["log:100"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_large_body_ends_at_the_memory_limit_not_in_a_hang() {
    let body = format!("<ul>{}</ul>", "<li class=\"x\">item</li>".repeat(100_000));
    let start = Instant::now();
    let result = run_with(LOAD_LIST, &body, small_heap()).await;
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(
        tests(&result),
        ["tests Test script error: fail script exceeded the 16 MiB memory limit"]
    );
}

fn listed_with_hash(file: &str, name: &str, version: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let listing = std::fs::read_to_string(root.join("THIRD-PARTY.md")).expect("THIRD-PARTY.md");
    let bytes = std::fs::read(
        root.join("crates/apinata-core/src/script/vendor")
            .join(file),
    )
    .expect("bundle");
    let hash: String = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let row = listing
        .lines()
        .find(|l| l.contains(&format!("vendor/{file}`")))
        .unwrap_or_default();
    assert!(
        row.contains(&format!("{name} {version}")) && row.contains(&hash),
        "{hash}"
    );
}

#[test]
fn third_party_lists_the_cheerio_bundle_with_its_hash() {
    listed_with_hash("cheerio-1.0.0.min.js", "cheerio", "1.0.0");
}

#[test]
fn third_party_lists_the_xml2js_bundle_with_its_hash() {
    listed_with_hash("xml2js-0.6.2.min.js", "xml2js", "0.6.2");
}

#[test]
fn third_party_names_both_licences_in_the_sax_file() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let listing = std::fs::read_to_string(root.join("THIRD-PARTY.md")).expect("THIRD-PARTY.md");
    let notice = std::fs::read_to_string(root.join("licenses/sax-ISC.txt")).expect("sax");
    assert!(notice.contains("ISC License") && notice.contains("MIT"));
    assert!(
        listing
            .contains("ISC and MIT (its `String.fromCodePoint` polyfill), `licenses/sax-ISC.txt`"),
        "the xml2js row must name sax's MIT part"
    );
}
