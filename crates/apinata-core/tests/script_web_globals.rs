//! The web globals and lodash available to scripts.

use crate::script_support;

use apinata_core::Outcome;
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Runs `code` as a tests script and returns its console lines.
async fn lines(code: &str) -> Vec<String> {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let text = scripted("R", &format!("{}/r", s.uri()), &script("tests", code), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(
        result.outcome,
        Outcome::Passed,
        "{:?} {:?}",
        result.tests,
        console(&result)
    );
    console(&result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn url_search_params_matches_the_standard() {
    let code = r#"const p = new URLSearchParams("a=1&b=2&a=3&c=%20x+y");
console.log(p.get("a"), p.getAll("a").join(","), p.get("c"), p.has("b"), p.get("zz"));
p.append("d","é &=");p.set("a","9");p.delete("b");
console.log(p.toString());
console.log([...p].map(([k,v])=>k+"="+v).join("|"), p.size);
console.log(new URLSearchParams({x:"1",y:"2"}).toString(), new URLSearchParams([["m","n"]]).toString(), new URLSearchParams("?z=1").toString());
{const s=new URLSearchParams("b=2&a=1&b=1&%zz=%41"); s.sort(); console.log(s.toString());}
console.log(new URLSearchParams({k:"a b*-._~!'()"}).toString());
const f=[]; new URLSearchParams("a=1&b=2").forEach((v,k,o)=>f.push(k+v+(o instanceof URLSearchParams))); console.log(f.join(","));
console.log(new URLSearchParams("a=1&a=2").has("a","2"), new URLSearchParams("a=1&a=2&a=3").toString());
const d=new URLSearchParams("a=1&a=2&b=3"); d.delete("a","2"); console.log(d.toString());
console.log([...new URLSearchParams("a=1&b=2").keys()].join(), [...new URLSearchParams("a=1&b=2").values()].join(), Object.prototype.toString.call(new URLSearchParams()));
console.log(String(new URLSearchParams("a")), new URLSearchParams("a=").get("a")==="", new URLSearchParams("=x").get(""));"#;
    assert_eq!(
        lines(code).await,
        [
            "log:1 1,3  x y true null",
            "log:a=9&c=+x+y&d=%C3%A9+%26%3D",
            "log:a=9|c= x y|d=é &= 3",
            "log:x=1&y=2 m=n z=1",
            "log:%25zz=A&a=1&b=2&b=1",
            "log:k=a+b*-._%7E%21%27%28%29",
            "log:a1true,b2true",
            "log:true a=1&a=2&a=3",
            "log:a=1&b=3",
            "log:a,b 1,2 [object URLSearchParams]",
            "log:a= true x",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn url_search_params_decodes_raw_astral_characters() {
    let code = r#"const a = new URLSearchParams("a=😀&😀=x&l=\ud800z");
console.log(a.get("a"), a.get("😀"), a.get("l"));
console.log(a.toString());"#;
    assert_eq!(
        lines(code).await,
        [
            "log:😀 x \u{fffd}z",
            "log:a=%F0%9F%98%80&%F0%9F%98%80=x&l=%EF%BF%BDz",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn url_parses_serialises_and_updates() {
    let code = r##"const u = new URL("https://user:pw@Example.COM:8443/a/./b/../c?x=1&y=2#frag");
console.log(u.href);
console.log(u.protocol,u.username,u.password,u.host,u.hostname,u.port,u.pathname,u.search,u.hash,u.origin);
u.searchParams.append("z","a b"); u.searchParams.set("x","9"); console.log(u.search, u.href);
u.pathname="/new path"; u.hash=""; u.port="443"; console.log(u.href, JSON.stringify(u.port));
u.search="?q=1"; console.log(u.searchParams.get("q"), u.searchParams.has("x"));
u.searchParams.delete("q"); console.log(JSON.stringify(u.search), u.href);
console.log(new URL("../x?y#z","http://h.test/a/b/c").href, new URL("http://h.test").href, new URL("HTTP://h.test:80/").href);
try{new URL("nope")}catch(e){console.log(e instanceof TypeError, e.name, e.message)}
console.log(URL.canParse("http://a"), URL.canParse("x"), URL.canParse("/x","http://a"));
console.log(JSON.stringify({u:new URL("http://a/b")}), `${new URL("http://a/b")}`);
const v=new URL("http://[::1]:3000/p"); console.log(v.host,v.hostname,v.port);
console.log(URL.canParse("http://ex ample.com"));
const w=new URL("file:///etc/hosts"); console.log(w.pathname,w.origin,w.host===""); 
const m=new URL("mailto:a@b.c"); console.log(m.pathname,m.origin,m.protocol);
console.log(new URL("http://münchen.de/ä?ü=ö#é").href);
const z=new URL("http://a/b"); z.protocol="https"; z.hostname="c.d"; z.username="me"; z.password="p w"; z.host="e.f:99"; console.log(z.href);
z.href="http://new/"; console.log(z.href); try{z.href="bad"}catch(e){console.log(e.name)}
console.log(new URL("//x.y/p","https://q/").href, new URL("?k=v","http://a/b?old#h").href, new URL("#h2","http://a/b?old#h").href);
console.log(Object.prototype.toString.call(z), z.toJSON(), z.searchParams===z.searchParams);
console.log(new URL("http://a/?a=1&b=%20x").searchParams.get("b"), new URL("http://a/b c").pathname, new URL("http://a/?q=a b").search);"##;
    assert_eq!(
        lines(code).await,
        [
            "log:https://user:pw@example.com:8443/a/c?x=1&y=2#frag",
            "log:https: user pw example.com:8443 example.com 8443 /a/c ?x=1&y=2 #frag https://example.com:8443",
            "log:?x=9&y=2&z=a+b https://user:pw@example.com:8443/a/c?x=9&y=2&z=a+b#frag",
            "log:https://user:pw@example.com/new%20path?x=9&y=2&z=a+b \"\"",
            "log:1 false",
            "log:\"\" https://user:pw@example.com/new%20path",
            "log:http://h.test/a/x?y#z http://h.test/ http://h.test/",
            "log:true TypeError Invalid URL",
            "log:true false true",
            "log:{\"u\":\"http://a/b\"} http://a/b",
            "log:[::1]:3000 [::1] 3000",
            "log:false",
            "log:/etc/hosts null true",
            "log:a@b.c null mailto:",
            "log:http://xn--mnchen-3ya.de/%C3%A4?%C3%BC=%C3%B6#%C3%A9",
            "log:https://me:p%20w@e.f:99/b",
            "log:http://new/",
            "log:TypeError",
            "log:https://x.y/p http://a/b?k=v http://a/b?old#h2",
            "log:[object URL] http://new/ true",
            "log: x /b%20c ?q=a%20b",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn url_survives_hostile_input_and_can_be_replaced() {
    let code = r#"const big = "http://h/" + "a".repeat(200000);
console.log(new URL(big).pathname.length);
for (const bad of ["", "http://", "http://[::1", "http://a:99999/", "\u0000", "http://%zz/"]) console.log(JSON.stringify(bad), URL.canParse(bad));
try { new URL("/x", "nope") } catch (e) { console.log(e.name) }
URL = 5; console.log(typeof URL);"#;
    assert_eq!(
        lines(code).await,
        [
            "log:200001",
            "log:\"\" false",
            "log:\"http://\" false",
            "log:\"http://[::1\" false",
            "log:\"http://a:99999/\" false",
            "log:\"\\u0000\" false",
            "log:\"http://%zz/\" false",
            "log:TypeError",
            "log:number",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_encoding_round_trips() {
    let code = r#"const e=new TextEncoder(); const b=e.encode("héllo €😀"); console.log(b.length, b instanceof Uint8Array, e.encoding, Array.from(b.slice(0,3)).join());
const d=new TextDecoder(); console.log(d.encoding, d.decode(b), d.decode(new Uint8Array([0xEF,0xBB,0xBF,104,105])), JSON.stringify(d.decode(new Uint8Array([104,0xff,105]))));
console.log(JSON.stringify(new TextDecoder("utf-8",{ignoreBOM:true}).decode(new Uint8Array([0xEF,0xBB,0xBF,104]))), d.decode(b.buffer), d.decode(), new TextDecoder("UTF8").encoding);
try{new TextDecoder("utf-8",{fatal:true}).decode(new Uint8Array([0xff]))}catch(x){console.log(x instanceof TypeError, x.name)}
try{new TextDecoder("klingon")}catch(x){console.log(x.name)}
const t=new Uint8Array(4); console.log(JSON.stringify(e.encodeInto("a€b",t)), Array.from(t).join());
console.log(d.decode(new Uint8Array([0xF0,0x9F])), JSON.stringify(e.encode()), e.encode(undefined).length, e.encode(String(12)).length);
console.log(d.decode(Buffer.from("h\u00e9")));"#;
    assert_eq!(
        lines(code).await,
        [
            "log:14 true utf-8 104,195,169",
            "log:utf-8 héllo €😀 hi \"h\u{fffd}i\"",
            "log:\"\u{feff}h\" héllo €😀  utf-8",
            "log:true TypeError",
            "log:RangeError",
            "log:{\"read\":2,\"written\":4} 97,226,130,172",
            "log:\u{fffd} {} 0 2",
            "log:hé",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structured_clone_copies_deeply() {
    let code = r#"const o={n:1,d:new Date(5),r:/a+/gi,m:new Map([[1,{x:1}]]),s:new Set([1,2]),t:new Uint8Array([1,2]),u:undefined,nested:{a:[1,{b:2}]},err:new RangeError("boom")};
o.self=o; const c=structuredClone(o);
console.log(c!==o, c.self===c, c.d instanceof Date, c.d.getTime(), c.r.source, c.r.flags, c.m.get(1).x, c.m.get(1)!==o.m.get(1), [...c.s].join(), c.t instanceof Uint8Array, c.t.buffer!==o.t.buffer, "u" in c, c.nested.a[1].b, c.nested!==o.nested, c.err instanceof RangeError, c.err.message);
try{structuredClone({f(){}})}catch(x){console.log(x.name, x instanceof Error, x.code)}
try{structuredClone(Symbol())}catch(x){console.log(x.name)}
class K{constructor(){this.a=1}get g(){return 2}} const k=structuredClone(new K()); console.log(k instanceof K, JSON.stringify(k), Object.getPrototypeOf(k)===Object.prototype);
console.log(structuredClone(5), structuredClone("s"), structuredClone(null), structuredClone([1,[2]]).length, typeof structuredClone(undefined));"#;
    assert_eq!(
        lines(code).await,
        [
            "log:true true true 5 a+ gi 1 true 1,2 true true true 2 true true boom",
            "log:DataCloneError true 25",
            "log:DataCloneError",
            "log:false {\"a\":1} true",
            "log:5 s null 2 undefined",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lodash_is_global_and_a_module() {
    let code = r#"console.log(typeof _, require("lodash") === _, _.VERSION);
console.log(JSON.stringify(_.chunk([1,2,3,4,5], 2)), _.get({a:[{b:3}]}, "a[0].b"), _.get({}, "x.y", "dflt"));
console.log(JSON.stringify(_.merge({a:{b:1}}, {a:{c:2}})), _.isEqual({a:[1,{b:2}]}, {a:[1,{b:2}]}), JSON.stringify(_.groupBy([1.2,1.5,2.1], Math.floor)));
console.log(JSON.stringify(_([3,1,2]).sortBy().map((n) => n * 2).value()), _.template("hi <%= n %>")({n: "x"}), JSON.stringify(_.cloneDeep({d:[1]})));
console.log(_.camelCase("foo bar-baz"), _.uniq([1,1,2]).length, JSON.stringify(_.pick({a:1,b:2}, ["b"])));
_ = 7; console.log(_);"#;
    assert_eq!(
        lines(code).await,
        [
            "log:function true 4.17.21",
            "log:[[1,2],[3,4],[5]] 3 dflt",
            "log:{\"a\":{\"b\":1,\"c\":2}} true {\"1\":[1.2,1.5],\"2\":[2.1]}",
            "log:[2,4,6] hi x {\"d\":[1]}",
            "log:fooBarBaz 2 {\"b\":2}",
            "log:7",
        ]
    );
}

#[test]
fn third_party_lists_the_vendored_lodash_with_its_hash() {
    use sha2::Digest;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let listing = std::fs::read_to_string(root.join("THIRD-PARTY.md")).expect("THIRD-PARTY.md");
    let bytes =
        std::fs::read(root.join("crates/apinata-core/src/script/vendor/lodash-4.17.21.min.js"))
            .expect("lodash");
    let hash: String = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(
        listing.contains("lodash") && listing.contains("4.17.21") && listing.contains(&hash),
        "{hash}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn url_and_querystring_modules() {
    let code = r#"const url = require("url"), qs = require("node:querystring");
console.log(url.URL === URL, url.URLSearchParams === URLSearchParams, require("node:url") === url);
const q = qs.parse("a=1&b=2&a=3&c=x+y%20z&d");
console.log(Object.getPrototypeOf(q) === null, JSON.stringify(q));
console.log(qs.stringify({a: [1, 2], b: "x y", c: true, d: null, e: {}}), qs.stringify({a: 1, b: 2}, ";", ":"));
console.log(qs.escape("a b&c/é"), qs.unescape("a%20b%E0%A4%A"), qs.decode === qs.parse);
console.log(JSON.stringify(qs.parse("a:1;b:2", ";", ":")));"#;
    assert_eq!(
        lines(code).await,
        [
            "log:true true true",
            "log:true {\"a\":[\"1\",\"3\"],\"b\":\"2\",\"c\":\"x y z\",\"d\":\"\"}",
            "log:a=1&a=2&b=x%20y&c=true&d=&e= a:1;b:2",
            "log:a%20b%26c%2F%C3%A9 a b\u{fffd}%A true",
            "log:{\"a\":\"1\",\"b\":\"2\"}",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sandbox_stays_closed() {
    let code = r#"for (const name of ["fs", "node:fs", "http", "https", "child_process", "net", "os"]) {
  try { require(name); console.log(name, "loaded"); } catch (e) { console.log(name, e.message); }
}"#;
    assert_eq!(
        lines(code).await,
        [
            "log:fs Cannot find module fs",
            "log:node:fs Cannot find module node:fs",
            "log:http Cannot find module http",
            "log:https Cannot find module https",
            "log:child_process Cannot find module child_process",
            "log:net Cannot find module net",
            "log:os Cannot find module os",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structured_clone_keeps_a_proto_key_as_own_data() {
    let code = r#"const src = JSON.parse('{"__proto__":{"a":1},"n":{"__proto__":[2]},"l":[{"__proto__":3}]}');
const c = structuredClone(src);
const own = (o) => Object.hasOwn(o, "__proto__");
console.log(own(c), JSON.stringify(Object.getOwnPropertyDescriptor(c, "__proto__").value), Object.getPrototypeOf(c) === Object.prototype, c.a);
console.log(own(c.n), JSON.stringify(c.n["__proto__"]), Object.getPrototypeOf(c.n) === Object.prototype);
console.log(own(c.l[0]), c.l[0]["__proto__"], Array.isArray(c.l), Object.getPrototypeOf(c.l[0]) === Object.prototype);
const d = Object.getOwnPropertyDescriptor(c, "__proto__");
console.log(d.enumerable, d.writable, d.configurable, c["__proto__"] !== src["__proto__"]);"#;
    assert_eq!(
        lines(code).await,
        [
            "log:true {\"a\":1} true undefined",
            "log:true [2] true",
            "log:true 3 true true",
            "log:true true true true",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structured_clone_of_an_error_ignores_table_names() {
    let code = r#"for (const name of ["constructor", "__proto__", "toString", "TypeError"]) {
  const e = new Error("boom");
  Object.defineProperty(e, "name", { value: name, configurable: true, writable: true });
  const c = structuredClone(e);
  console.log(name, c instanceof Error, c instanceof TypeError, c.message, c.name);
}"#;
    assert_eq!(
        lines(code).await,
        [
            "log:constructor true false boom Error",
            "log:__proto__ true false boom Error",
            "log:toString true false boom Error",
            "log:TypeError true true boom TypeError",
        ]
    );
}
