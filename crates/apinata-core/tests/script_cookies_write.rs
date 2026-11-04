//! Script cookie writes: `bru.cookies.add/remove/clear` and `bru.cookies.jar()` store into the
//! executor's jar under the same rules as a response cookie from that URL.

use crate::script_support;

use apinata_core::{
    CancelHandle, Collection, ContextOptions, ExecOptions, Executor, ItemPath, RequestResult,
    VarContext, send_one,
};
use script_support::*;
use std::path::Path;
use std::sync::Arc;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A server answering 200 on `/r` and `/s`.
async fn server() -> MockServer {
    let s = MockServer::start().await;
    for at in ["/r", "/s"] {
        Mock::given(path(at))
            .respond_with(ResponseTemplate::new(200))
            .mount(&s)
            .await;
    }
    s
}

/// Sends `r.yml` (a GET to `/r` running `scripts`) through a fresh in-memory executor, which is
/// returned so a test can read its jar.
async fn send_scripts(s: &MockServer, scripts: &str) -> (RequestResult, Arc<Executor>) {
    let text = scripted("R", &format!("{}/r", s.uri()), scripts, "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    let result = send_with(dir.path(), "r.yml", &exec).await;
    (result, exec)
}

async fn send_with(dir: &Path, rel: &str, exec: &Arc<Executor>) -> RequestResult {
    let c = Collection::open(dir).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    let request = apinata_core::RequestFile::load(dir.join(rel)).expect("loads");
    let item = ItemPath::new(rel).expect("item path");
    send_one(item, &request, &ctx, exec, &on(), &CancelHandle::new()).await
}

/// The Cookie header each request to `at` carried.
async fn cookie_headers(s: &MockServer, at: &str) -> Vec<Option<String>> {
    s.received_requests()
        .await
        .expect("recorded")
        .iter()
        .filter(|r| r.url.path() == at)
        .map(|r| {
            r.headers
                .get("cookie")
                .map(|v| v.to_str().expect("ascii").to_owned())
        })
        .collect()
}

fn jar_names(exec: &Executor) -> Vec<String> {
    exec.cookies()
        .list()
        .into_iter()
        .map(|c| format!("{} {}", c.domain, c.name))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_request_cookie_is_sent_on_the_same_request_and_kept_in_the_jar() {
    let s = server().await;
    let code = r#"await bru.cookies.add({ key: "sid", value: "s1" });
const c = bru.cookies.one("sid");
console.log(bru.cookies.get("sid"), c.hostOnly, c.path);"#;
    let (result, exec) = send_scripts(&s, &script("before-request", code)).await;
    assert_eq!(console(&result), ["log:s1 true /"]);
    assert_eq!(cookie_headers(&s, "/r").await, [Some("sid=s1".to_owned())]);
    assert_eq!(jar_names(&exec), ["127.0.0.1 sid"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clear_removes_only_what_the_request_url_sees() {
    let s = server().await;
    let before = script(
        "before-request",
        r#"await bru.cookies.add({ key: "a", value: "1" });
await bru.cookies.upsert({ key: "b", value: "2" });
await bru.cookies.jar().setCookie("http://other.test/", "o", "3");"#,
    );
    let after = script(
        "after-response",
        r#"await bru.cookies.remove("a");
console.log(Object.keys(bru.cookies.toObject()).join(","));
await bru.cookies.clear();
console.log(bru.cookies.count());"#,
    );
    let (result, exec) = send_scripts(&s, &format!("{before}{after}")).await;
    assert_eq!(console(&result), ["log:b", "log:0"]);
    assert_eq!(jar_names(&exec), ["other.test o"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_jar_handle_sets_reads_and_deletes_cookies_for_any_url() {
    let s = server().await;
    let code = r#"bru.setVar("host", "api.test");
const jar = bru.cookies.jar();
const u = "http://{{host}}/a/b";
await jar.setCookie(u, "n1", "v1");
await jar.setCookie(u, { key: "n2", value: "v2", path: "/", httpOnly: true });
const c = await jar.getCookie(u, "n1");
console.log(c.value, c.domain, c.path, c.hostOnly, await jar.hasCookie(u, "n2"), await jar.hasCookie(u, "zz"), (await jar.getCookies(u)).length, await jar.getCookie(u, "zz"));
await jar.deleteCookie(u, "n1");
console.log((await jar.getCookies(u)).map((c) => c.key).join(","));
await jar.setCookie(u, { key: "d", value: "root", path: "/" });
await jar.setCookie(u, { key: "d", value: "deep", path: "/a" });
await jar.deleteCookie(u, "d");
console.log((await jar.getCookie(u, "d")).value);
await jar.setCookies(u, [{ key: "x", value: "1" }, { name: "y", value: "2" }]);
console.log((await jar.getCookies(u)).length);
await jar.deleteCookies(u);
console.log((await jar.getCookies(u)).length);
await jar.setCookie("http://keep.test/", "k", "1");
await jar.clear();
console.log((await jar.getCookies("http://keep.test/")).length);"#;
    let (result, exec) = send_scripts(&s, &script("before-request", code)).await;
    assert_eq!(
        console(&result),
        [
            "log:v1 api.test /a false true false 2 null",
            "log:n2",
            "log:root",
            "log:4",
            "log:0",
            "log:0",
        ]
    );
    assert!(exec.cookies().list().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_cookie_resolves_and_warns_without_its_value() {
    let s = server().await;
    let code = r#"const jar = bru.cookies.jar();
await bru.cookies.add({ key: "evil1", value: "hunter2", domain: "evil.example" });
await jar.setCookie("http://localhost/", { key: "evil2", value: "hunter2", domain: "evil.example" });
await jar.setCookie("http://a.test/", { key: "label", value: "hunter2", domain: "test" });
await jar.setCookie("http://a.test/", { key: "__Secure-s", value: "hunter2" });
await jar.setCookie("https://a.test/", { key: "__Host-h", value: "hunter2", secure: true, path: "/", domain: "a.test" });
await jar.setCookie("ftp://a.test/", "scheme", "hunter2");
await jar.setCookie("http://a.test/", "semi", "hunter2;x");
await jar.setCookie("http://api.a.test/", { key: "parent", value: "1", domain: "a.test" });
await jar.setCookie("https://a.test/", { key: "__Host-ok", value: "1", secure: true, path: "/" });
console.log("resolved");"#;
    let (result, exec) = send_scripts(&s, &script("before-request", code)).await;
    let lines = console(&result);
    assert_eq!(lines.last().map(String::as_str), Some("log:resolved"));
    let warns: Vec<&String> = lines.iter().filter(|l| l.starts_with("warn:")).collect();
    assert_eq!(warns.len(), 7, "{lines:?}");
    for (warn, name) in warns.iter().zip([
        "evil1",
        "evil2",
        "label",
        "__Secure-s",
        "__Host-h",
        "scheme",
        "semi",
    ]) {
        assert!(warn.contains(name), "{warn}");
        assert!(!warn.contains("hunter2"), "{warn}");
    }
    let stored: Vec<(String, String, bool)> = exec
        .cookies()
        .list()
        .into_iter()
        .map(|c| (c.domain, c.name, c.host_only))
        .collect();
    assert_eq!(
        stored,
        [
            ("a.test".to_owned(), "__Host-ok".to_owned(), true),
            ("a.test".to_owned(), "parent".to_owned(), false),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_arguments_reject_with_the_reference_messages() {
    let s = server().await;
    let code = r#"const jar = bru.cookies.jar();
const msg = async (p) => { try { await p; return "resolved"; } catch (e) { return e.message; } };
const out = [
  await msg(jar.setCookie("", "a", "b")),
  await msg(jar.setCookie("http://a.test/", "", "b")),
  await msg(jar.setCookie("http://a.test/", { value: "b" })),
  await msg(jar.setCookie("http://a.test/", 5)),
  await msg(jar.setCookies("http://a.test/", "nope")),
  await msg(jar.getCookie("http://a.test/")),
  await msg(jar.getCookies("")),
  await msg(bru.cookies.add(null)),
  await msg(bru.cookies.remove("")),
];
console.log(out.join(" | "));
const partial = await msg(jar.setCookies("http://a.test/", [{ key: "one", value: "1" }, { value: "2" }]));
console.log(partial, (await jar.getCookies("http://a.test/")).map((c) => c.key).join(","));"#;
    let (result, _) = send_scripts(&s, &script("before-request", code)).await;
    assert_eq!(
        console(&result),
        [
            "log:URL is required | Cookie name is required | cookieObject.key (name) is required | Invalid arguments passed to setCookie | setCookies expects an array of cookie objects | URL and cookie name are required | URL is required | cookieObj must be a non-null object | resolved",
            "log:cookieObject.key (name) is required one",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trailing_callback_gets_null_and_the_result() {
    let s = server().await;
    let code = r#"const jar = bru.cookies.jar();
await jar.setCookie("http://a.test/", "cb", "v", (err, r) => console.log("set", err, r));
await jar.getCookie("http://a.test/", "cb", (err, c) => console.log("get", err, c.value));
await jar.getCookie("", "x", (err, c) => console.log("bad", err.message, c));
await bru.cookies.add({ key: "k", value: "1" }, (err) => console.log("add", err));"#;
    let (result, _) = send_scripts(&s, &script("before-request", code)).await;
    assert_eq!(
        console(&result),
        [
            "log:set null undefined",
            "log:get null v",
            "log:bad URL and cookie name are required null",
            "log:add null",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_post_response_cookie_is_sent_by_the_next_request() {
    let s = server().await;
    let u = s.uri();
    let first = scripted(
        "A",
        &format!("{u}/r"),
        &script(
            "after-response",
            r#"await bru.cookies.add({ key: "next", value: "n1" });"#,
        ),
        "",
    );
    let second = scripted("B", &format!("{u}/s"), &script("tests", "1"), "");
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &first),
        ("b.yml", &second.replace("seq: 1", "seq: 2")),
    ]);
    let exec = Arc::new(Executor::new(ExecOptions::new()).expect("client"));
    send_with(dir.path(), "a.yml", &exec).await;
    send_with(dir.path(), "b.yml", &exec).await;
    assert_eq!(cookie_headers(&s, "/r").await, [None]);
    assert_eq!(cookie_headers(&s, "/s").await, [Some("next=n1".to_owned())]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expires_and_max_age_set_an_expiry_and_max_age_zero_removes_the_cookie() {
    let s = server().await;
    let code = r#"const jar = bru.cookies.jar();
const u = "http://a.test/";
await jar.setCookie(u, { key: "e", value: "1", expires: new Date(Date.now() + 3600e3).toISOString() });
await jar.setCookie(u, { key: "m", value: "1", maxAge: 3600 });
await jar.setCookie(u, { key: "s", value: "1", expires: "not a date" });
const at = async (n) => (await jar.getCookie(u, n)).expires !== null;
console.log(await at("e"), await at("m"), await at("s"));
await jar.setCookie(u, { key: "m", value: "1", maxAge: 0 });
console.log(await jar.hasCookie(u, "m"));"#;
    let (result, _) = send_scripts(&s, &script("before-request", code)).await;
    assert_eq!(console(&result), ["log:true true false", "log:false"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn out_of_range_expiry_inputs_clamp_instead_of_overflowing() {
    let s = server().await;
    let code = r#"const jar = bru.cookies.jar();
const u = "http://a.test/";
await jar.setCookie(u, { key: "neg", value: "1", maxAge: -1e300 });
await jar.setCookie(u, { key: "big", value: "1", maxAge: 1e300 });
await jar.setCookie(u, { key: "inf", value: "1", maxAge: Infinity });
await jar.setCookie(u, { key: "nan", value: "1", maxAge: NaN });
await jar.setCookie(u, { key: "far", value: "1", expires: 8.64e15 });
await jar.setCookie(u, { key: "past", value: "1", expires: -8.64e15 });
const show = async (n) => { const c = await jar.getCookie(u, n); return c === null ? "gone" : c.expires === null ? "session" : c.expires.slice(0, 4); };
console.log(await show("neg"), await show("big"), await show("inf"), await show("nan"), await show("far"), await show("past"));"#;
    let (result, _) = send_scripts(&s, &script("before-request", code)).await;
    assert_eq!(
        console(&result),
        ["log:gone 9999 session session 9999 gone"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trailing_dot_does_not_hide_a_single_label_domain() {
    let s = server().await;
    let code = r#"const jar = bru.cookies.jar();
await jar.setCookie("http://a.example.com./", { key: "tld", value: "1", domain: "com." });
console.log((await jar.getCookies("http://a.example.com./")).length);"#;
    let (result, exec) = send_scripts(&s, &script("before-request", code)).await;
    let lines = console(&result);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("warn:") && lines[0].contains("tld"),
        "{lines:?}"
    );
    assert_eq!(lines[1], "log:0");
    assert!(exec.cookies().list().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_name_is_escaped_and_cut_in_the_warning() {
    let s = server().await;
    let code = r#"const jar = bru.cookies.jar();
await jar.setCookie("http://a.test/", "bad\nname", "1");
await jar.setCookie("http://a.test/", "x".repeat(500) + ";", "1");"#;
    let (result, _) = send_scripts(&s, &script("before-request", code)).await;
    let lines = console(&result);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(!lines[0].contains('\n'), "{:?}", lines[0]);
    assert!(lines[0].contains(r"bad\nname"), "{:?}", lines[0]);
    assert!(lines[1].len() < 200, "{}", lines[1].len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_cookie_on_a_path_tie_removes_the_first_in_sort_order() {
    let s = server().await;
    let code = r#"const jar = bru.cookies.jar();
const u = "http://x.a.test/p/q";
await jar.setCookie("http://x.a.test/", { key: "d", value: "host", path: "/p" });
await jar.setCookie("http://x.a.test/", { key: "d", value: "parent", path: "/p", domain: "a.test" });
console.log((await jar.getCookie(u, "d")).domain);
await jar.deleteCookie(u, "d");
console.log((await jar.getCookies(u)).map((c) => c.domain).join(","));"#;
    for _ in 0..8 {
        let (result, _) = send_scripts(&s, &script("before-request", code)).await;
        assert_eq!(console(&result), ["log:a.test", "log:x.a.test"]);
    }
}
