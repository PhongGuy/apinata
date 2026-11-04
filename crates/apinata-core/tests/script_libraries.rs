//! The vendored libraries load on first use and behave as their published versions.

use crate::script_support;

use apinata_core::{Outcome, VarResult};
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn run_code(code: &str, extra: &str) -> apinata_core::RequestResult {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let text = scripted(
        "R",
        &format!("{}/r", s.uri()),
        &script("tests", code),
        extra,
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    send(dir.path(), "r.yml", on()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moment_crypto_js_and_tv4_work() {
    let code = r#"const m = moment.utc("2024-01-02T03:04:05Z");
console.log(m.format("YYYY/MM/DD HH:mm"), require("moment") === moment);
const C = require("crypto-js");
console.log(C.SHA256("abc").toString(), C.enc.Base64.stringify(C.enc.Utf8.parse("hi")), C.HmacSHA256("m", "k").toString());
console.log(tv4.validate({ a: 1 }, { type: "object", properties: { a: { type: "string" } } }), require("tv4") === tv4, typeof globalThis.CryptoJS);"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            "log:2024/01/02 03:04 true",
            "log:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad aGk= b60090e3052297aeb5a080889ce2fc4bca957e756faeb4df7d31800ca1e771ec",
            "log:false true undefined",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn libraries_are_unsupported_in_expressions() {
    let extra = "  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"moment().year()\"\n        method: jsonq\n      variable:\n        name: y\n        scope: runtime\n";
    let result = run_code("1", extra).await;
    match result.post_response_vars[0].result() {
        VarResult::Skipped { reason } => assert!(
            reason.contains("moment is not supported by Apinata"),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uuid_nanoid_crypto_and_base64() {
    let code = r#"const re = /^[0-9a-f]{8}-[0-9a-f]{4}-([1-8])[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const ns = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
console.log([uuid.v1(), uuid.v4(), uuid.v6(), uuid.v7()].map((i) => re.exec(i)[1]).join(""), uuid.v3("hello", ns), uuid.v5("hello", ns));
const bytes = uuid.parse(ns);
console.log(bytes instanceof Uint8Array, bytes.length, uuid.stringify(bytes) === ns, uuid.validate(ns), uuid.validate("x"), uuid.version(ns));
const id = require("nanoid").nanoid(5);
console.log(id.length, /^[A-Za-z0-9_-]+$/.test(id), require("nanoid") === nanoid);
const a = new Uint8Array(32);
let big = "none"; try { crypto.getRandomValues(new Uint8Array(65537)); } catch (e) { big = "threw"; }
console.log(crypto.getRandomValues(a) === a, a.some((b) => b !== 0), big, require("crypto-js").lib.WordArray.random(8).sigBytes);
const rb = crypto.randomBytes(4); console.log(Buffer.isBuffer(rb), rb.length);
console.log(btoa("hi"), atob("aGk="), require("btoa")("hi"), require("atob") === atob, require("uuid") === uuid);"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            "log:1467 0bacede4-4014-3f9d-b720-173f68a1c933 9342d47a-1bab-5709-9869-c840b2eac501",
            "log:true 16 true true false 1",
            "log:21 true true",
            "log:true true threw 8",
            "log:true 4",
            "log:aGk= hi aGk= true true",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_named_uuid_needs_a_canonical_namespace() {
    let code = r#"for (const ns of ["6ba7b8109dad11d180b400c04fd430c8", "{6ba7b810-9dad-11d1-80b4-00c04fd430c8}", "urn:uuid:6ba7b810-9dad-11d1-80b4-00c04fd430c8", "x"]) {
  for (const make of [uuid.v3, uuid.v5]) {
    try { make("hello", ns); console.log("accepted"); } catch (e) { console.log(e instanceof TypeError, e.message); }
  }
}"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(console(&result), ["log:true Invalid UUID"; 8]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_hash_matches_known_vectors() {
    let code = r#"for (const a of ["md5", "sha1", "sha224", "sha256", "sha384", "sha512"]) console.log(a, crypto.createHash(a).update("abc").digest("hex"));
console.log(crypto.createHash("sha256").update("").digest("base64"), crypto.createHash("sha256").update("abc").digest("base64url"));
const b = crypto.createHash("SHA256").update("a").update("b").update("c").digest();
console.log(Buffer.isBuffer(b), b.length, b.toString("hex").slice(0, 8));
const kept = [crypto.createHash("sha256").update("abc").digest("buffer"), crypto.createHmac("sha256", "k").update("m").digest("BUFFER")];
console.log(...kept.map((k) => `${Buffer.isBuffer(k)} ${k.length}`));
console.log(crypto.createHash("sha1").update("\u00e9\u20ac").digest("hex"), crypto.createHash("sha1").update("\u00e9\u20ac", "utf8").digest("hex") === crypto.createHash("sha1").update(Buffer.from("\u00e9\u20ac")).digest("hex"));
const inputs = [new Uint8Array([1, 2, 255]), Buffer.from([1, 2, 255]), new Uint8Array([9, 1, 2, 255]).subarray(1), new Uint8Array([1, 2, 255]).buffer, new DataView(new Uint8Array([9, 1, 2, 255, 9]).buffer, 1, 3)];
console.log(...inputs.map((i) => crypto.createHash("sha256").update(i).digest("hex").slice(0, 8)), crypto.createHash("sha256").update("0102ff", "hex").digest("hex"), crypto.createHash("sha256").update("AQL_", "base64url").digest("hex").slice(0, 8));"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            "log:md5 900150983cd24fb0d6963f7d28e17f72",
            "log:sha1 a9993e364706816aba3e25717850c26c9cd0d89d",
            "log:sha224 23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7",
            "log:sha256 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "log:sha384 cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
            "log:sha512 ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            "log:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU= ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0",
            "log:true 32 ba7816bf",
            "log:true 32 true 32",
            "log:f299a77642b6437431ada56c0643f7b8e99b5622 true",
            "log:0526d0e1 0526d0e1 0526d0e1 0526d0e1 0526d0e1 0526d0e18ea19dfaad9d79166bec1e18d6221ef6b1830385fe9bf67022ed5f96 0526d0e1",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_hmac_matches_rfc_vectors() {
    let code = r#"const d = "what do ya want for nothing?";
for (const a of ["md5", "sha1", "sha256", "sha512"]) console.log(a, crypto.createHmac(a, "Jefe").update(d).digest("hex"));
console.log(crypto.createHmac("sha256", Buffer.from("Jefe")).update(d).digest("base64"), crypto.createHmac("Sha256", new Uint8Array([74, 101, 102, 101])).update("what do ya ").update("want for nothing?").digest().toString("hex").slice(0, 8));"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            "log:md5 750c783e6ab0b503eaa86e310a5db738",
            "log:sha1 effcdf6ae5eb2fa2d27416d5f184df9c259a7c79",
            "log:sha256 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            "log:sha512 164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737",
            "log:W9zBRr9gdU5qBCQmCJV1x1oAPwidJzmDnexYuWTsOEM= 5bdcc146",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crypto_module_names_and_errors() {
    let code = r#"const show = (f) => { try { f(); console.log("no error"); } catch (e) { console.log(e.name, e.message); } };
console.log(require("crypto") === crypto, require("node:crypto") === crypto, typeof crypto.subtle);
show(() => crypto.createHash("sha3-256"));
show(() => crypto.createHmac("whirlpool", "k"));
show(() => crypto.createHash());
show(() => crypto.createHash(undefined));
show(() => crypto.createHmac(1, "k"));
show(() => crypto.createHash("sha256").update(42));
const h = crypto.createHash("md5"); h.digest();
show(() => h.digest());
show(() => h.update("x"));
show(() => crypto.randomBytes("4"));
show(() => crypto.randomBytes(-1));
show(() => crypto.randomBytes(1.5));
show(() => crypto.randomBytes(65537));
console.log(crypto.randomBytes(0).length, crypto.randomBytes(65536).length);
const id = crypto.randomUUID();
console.log(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id), id !== crypto.randomUUID());
let order = [];
const ret = crypto.randomBytes(8, (err, buf) => { order.push("cb"); console.log(err, Buffer.isBuffer(buf), buf.length, order.join(",")); });
order.push("after");
console.log(ret);"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    let unsupported = |alg: &str| {
        format!(
            "log:Error Digest method not supported: \"{alg}\" (supported: md5, sha1, sha224, sha256, sha384, sha512; require('crypto-js') has more)"
        )
    };
    let algorithm =
        "log:TypeError The \"algorithm\" argument must be of type string. Received undefined";
    let range = "log:RangeError The \"size\" argument must be an integer from 0 to 65536";
    assert_eq!(
        console(&result),
        [
            "log:true true undefined".to_string(),
            unsupported("sha3-256"),
            unsupported("whirlpool"),
            algorithm.into(),
            algorithm.into(),
            "log:TypeError The \"hmac\" argument must be of type string. Received type number (1)".into(),
            "log:TypeError The \"data\" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received type number (42)".into(),
            "log:Error Digest already called".into(),
            "log:Error Digest already called".into(),
            "log:TypeError The \"size\" argument must be of type number".into(),
            range.into(),
            range.into(),
            range.into(),
            "log:0 65536".into(),
            "log:true true".into(),
            "log:undefined".into(),
            "log:null true 8 after,cb".into(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_speaks_base64url() {
    let code = r#"const b = Buffer.from([0xfb, 0xff, 0xbf]);
console.log(b.toString("base64url"), b.toString("Base64URL", 1), Buffer.isEncoding("base64url"), Buffer.isEncoding("BASE64URL"));
console.log([...Buffer.from("-_-_", "base64url")].join(","), Buffer.byteLength("-_-_", "base64url"));
const text = Buffer.from("héllo?>", "utf8").toString("base64url");
console.log(text, Buffer.from(text, "base64url").toString("utf8"));
const w = Buffer.alloc(4); console.log(w.write("-_-_", 1, "base64url"), [...w].join(","));
console.log([...Buffer.alloc(3).fill("-w", "base64url")].join(","));
const B = require("buffer").Buffer; console.log(B === Buffer, B.from([0xfb]).toString("base64url"));"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            "log:-_-_ _78 true true",
            "log:251,255,191 3",
            "log:aMOpbGxvPz4 h\u{e9}llo?>",
            "log:3 0,251,255,191",
            "log:251,251,251",
            "log:true -w",
        ]
    );
}

/// Pinned from Node 24.16: a falsy encoding other than `undefined` is unknown, but an empty range is "".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_to_string_rejects_a_falsy_encoding() {
    let code = r#"const b = Buffer.from("hi");
const tryIt = (f) => { try { return JSON.stringify(f()); } catch (e) { return e.name + " " + e.code + " " + JSON.stringify(e.message); } };
for (const e of [null, "", 0, false, NaN]) console.log(tryIt(() => b.toString(e)), tryIt(() => b.toLocaleString(e)));
const ab = Buffer.from("ab");
console.log(tryIt(() => Buffer.alloc(0).toString(null)), tryIt(() => ab.toString(null, 5)), tryIt(() => ab.toString(null, 0, 0)), tryIt(() => ab.toString("", 2)), tryIt(() => ab.toString(null, 1, 0)), tryIt(() => ab.toString(null, -5, 1)));
console.log(tryIt(() => b.toString(undefined)), tryIt(() => b.toString()), tryIt(() => b.toLocaleString(undefined, 1)));"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    let unknown =
        |name: &str| format!("TypeError ERR_UNKNOWN_ENCODING \"Unknown encoding: {name}\"");
    let both = |name: &str| format!("log:{} {}", unknown(name), unknown(name));
    assert_eq!(
        console(&result),
        [
            both("null"),
            both(""),
            both("0"),
            both("false"),
            both("NaN"),
            format!("log:\"\" \"\" \"\" \"\" \"\" {}", unknown("null")),
            "log:\"hi\" \"hi\" \"i\"".to_owned(),
        ]
    );
}

/// Pinned from Node 24.16: any unknown encoding throws, named as util.format's `%s` shows it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_to_string_names_an_unknown_encoding() {
    let code = r#"const b = Buffer.from("hi");
const tryIt = (f) => { try { return JSON.stringify(f()); } catch (e) { return e.name + " " + e.code + " " + JSON.stringify(e.message); } };
class Named { toString() { return "named"; } }
const own = { toString() { return "own"; } };
function foo() { return 1; }
const six = [0, 1, 2, 3, 4, 5];
for (const e of [{}, [], [1, 2], { a: 1 }, { "a-b": "x" }, { a: { b: 1 } }, [[1]], ["s"], 1, true, -0, 1n, "bogus", "raw", foo, own, new Named(), six, { [Symbol("x")]: 1 }, { a: 1, [Symbol("x")]: "s" }]) console.log(tryIt(() => b.toString(e)));
console.log(tryIt(() => b.toLocaleString({})));
console.log(tryIt(() => Buffer.alloc(0).toString({})), tryIt(() => Buffer.from("a").toString("bogus", 5)), tryIt(() => Buffer.from("a").toString("bogus", 0, 0)));
console.log(tryIt(() => Buffer.alloc(0).toString(Symbol())), tryIt(() => Buffer.from("a").toString(Object.create(null), 5)), tryIt(() => Buffer.from("a").toString(Object.create(null), 0, 0)));
console.log(tryIt(() => b.toString(Symbol("s"))), tryIt(() => b.toString(Object.create(null))));
console.log(tryIt(() => b.toString(new String("hex"))), tryIt(() => b.toString("UTF8")));"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    let unknown =
        |name: &str| format!("log:TypeError ERR_UNKNOWN_ENCODING \"Unknown encoding: {name}\"");
    let mut expected: Vec<String> = [
        "{}",
        "[]",
        "[ 1, 2 ]",
        "{ a: 1 }",
        "{ 'a-b': 'x' }",
        "{ a: [Object] }",
        "[ [Array] ]",
        "[ 's' ]",
        "1",
        "true",
        "-0",
        "1n",
        "bogus",
        "raw",
        "function foo() { return 1; }",
        "own",
        "named",
        "[ 0, 1, 2, 3, 4, 5 ]",
        "{ Symbol(x): 1 }",
        "{ a: 1, Symbol(x): 's' }",
        "{}",
    ]
    .map(unknown)
    .into();
    expected.extend([
        "log:\"\" \"\" \"\"".to_owned(),
        "log:\"\" \"\" \"\"".to_owned(),
        "log:TypeError undefined \"Cannot convert a Symbol value to a string\" TypeError undefined \"Cannot convert object to primitive value\"".to_owned(),
        "log:\"6869\" \"hi\"".to_owned(),
    ]);
    assert_eq!(console(&result), expected);
}

/// Pinned from Node 24.16 (recorded by running Node itself): an unknown encoding that is a builtin, a
/// class instance or a long array is named as util.format's `%s` inspects it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_to_string_inspects_builtins_in_an_unknown_encoding() {
    let code = r#"const b = Buffer.from("hi");
const tryIt = (f) => { try { return JSON.stringify(f()); } catch (e) { return e.name + " " + e.code + " " + JSON.stringify(e.message); } };
class C {}
class D { constructor() { this.a = 1; } }
class Named { toString() { return "named"; } }
const sparse = [1, , 3];
const tagged = Object.assign([1], { x: 1 });
const inherited = Object.create({ a: 1 });
const both = Object.create({ a: 1 });
both.b = 2;
const cases = [
  new Date(0), new Date(NaN), new Map(), new Map([[1, "a"], ["k", { z: 1 }]]), new Set(), new Set([1, "s"]), /x/gi,
  [0, 1, 2, 3, 4, 5, 6, 7], ["a", "b", "c", "d", "e", "f", "g", "h"], Array.from({ length: 30 }, (_, i) => i),
  [1.5, 22, 333, 4444, 55555, 666666, 7777777, 88888888], ["aa", "bb", "cc", "dd", "ee", "ff", "gg"],
  [1, "a", 2, "b", 3, "c", 4], Array.from({ length: 101 }, (_, i) => i),
  new C(), new D(), inherited, both, { get k() { return 5; } }, { get a() { return 1; }, set a(v) {} }, { set s(v) {} },
  sparse, tagged, { a: new Date(0), b: /x/, c: new Map(), d: new Set(), e: new C(), f: [1] }, [new Map(), new D(), new Date(0)],
  { a: "x".repeat(60), b: "y".repeat(60) }, new Named(),
];
for (const e of cases) console.log(tryIt(() => b.toString(e)));"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: 1970-01-01T00:00:00.000Z""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: Invalid Date""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: Map(0) {}""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: Map(2) { 1 => 'a', 'k' => [Object] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: Set(0) {}""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: Set(2) { 1, 's' }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: /x/gi""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [\n  0, 1, 2, 3,\n  4, 5, 6, 7\n]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [\n  'a', 'b', 'c',\n  'd', 'e', 'f',\n  'g', 'h'\n]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [\n   0,  1,  2,  3,  4,  5,  6,  7,  8,\n   9, 10, 11, 12, 13, 14, 15, 16, 17,\n  18, 19, 20, 21, 22, 23, 24, 25, 26,\n  27, 28, 29\n]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [\n      1.5,       22,\n      333,     4444,\n    55555,   666666,\n  7777777, 88888888\n]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [\n  'aa', 'bb',\n  'cc', 'dd',\n  'ee', 'ff',\n  'gg'\n]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [\n  1, 'a', 2, 'b',\n  3, 'c', 4\n]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [\n   0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11,\n  12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,\n  24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35,\n  36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,\n  48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59,\n  60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71,\n  72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83,\n  84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95,\n  96, 97, 98, 99,\n  ... 1 more item\n]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: C {}""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: D { a: 1 }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: {}""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { b: 2 }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Getter] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { a: [Getter/Setter] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { s: [Setter] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [ 1, <1 empty item>, 3 ]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [ 1, x: 1 ]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: {\n  a: 1970-01-01T00:00:00.000Z,\n  b: /x/,\n  c: Map(0) {},\n  d: Set(0) {},\n  e: C {},\n  f: [Array]\n}""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [ Map(0) {}, [D], 1970-01-01T00:00:00.000Z ]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: {\n  a: 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx',\n  b: 'yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy'\n}""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: named""#,
        ]
    );
}

/// Pinned from Node 24.16 (recorded by running Node itself): the encoding name of a sparse array, a
/// capped Map or Set, a script-set `Symbol.toStringTag` and an escaped string, and an element getter that
/// the formatter must not run again (both runs read it once, in the conversion to a string).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_to_string_names_odd_encodings_safely() {
    let code = r#"const b = Buffer.from("hi");
const show = (e) => { try { b.toString(e); return "no error"; } catch (x) { const m = x.message.replace("Unknown encoding: ", ""); return m.length > 120 ? m.length + " " + JSON.stringify(m.slice(0, 40)) + ".." + JSON.stringify(m.slice(-40)) : JSON.stringify(m); } };
const huge = []; huge.length = 2 ** 32 - 1;
const gap = []; gap[5] = 1; gap[10 ** 6] = 2;
const mid = [1]; mid.length = 50; mid[40] = 2;
const lone = []; lone[3] = "x"; lone.extra = 1;
let calls = 0;
const getterAt = []; Object.defineProperty(getterAt, 0, { get() { calls++; return 7; }, enumerable: true });
const getterNum = [1, 2, 3, 4, 5, 6, 7]; Object.defineProperty(getterNum, 3, { get() { calls++; return 4; }, enumerable: true });
const ctorGetter = { get constructor() { throw new Error("ran"); } };
const bigMap = new Map(Array.from({ length: 120 }, (_, i) => [i, i]));
const bigSet = new Set(Array.from({ length: 120 }, (_, i) => i));
class Sub extends Map {}
const cases = [gap, mid, lone, getterAt, getterNum, ctorGetter,
  { [Symbol.toStringTag]: "Map" }, { [Symbol.toStringTag]: "Date" }, { [Symbol.toStringTag]: "Set" }, { [Symbol.toStringTag]: "RegExp" },
  bigMap, bigSet, new Sub([[1, 2]]), { a: "x\ny" }, [["it's\ttab"]], { "k\nx": 1 }, Object.setPrototypeOf([1], null), { a: [new Array(3)] }];
for (const e of cases) console.log(show(e));
console.log(calls);"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            r#"log:"[ <5 empty items>, 1, <999994 empty items>, 2 ]""#,
            r#"log:"[ 1, <39 empty items>, 2, <9 empty items> ]""#,
            r#"log:"[ <3 empty items>, 'x', extra: 1 ]""#,
            r#"log:"[ [Getter] ]""#,
            r#"log:"[ 1, 2, 3, [Getter], 5, 6, 7 ]""#,
            r#"log:"{ constructor: [Getter] }""#,
            r#"log:"{ Symbol(Symbol.toStringTag): 'Map' }""#,
            r#"log:"{ Symbol(Symbol.toStringTag): 'Date' }""#,
            r#"log:"{ Symbol(Symbol.toStringTag): 'Set' }""#,
            r#"log:"{ Symbol(Symbol.toStringTag): 'RegExp' }""#,
            r#"log:1212 "Map(120) {\n  0 => 0,\n  1 => 1,\n  2 => 2,".."=> 98,\n  99 => 99,\n  ... 20 more items\n}""#,
            r#"log:622 "Set(120) {\n  0,\n  1,\n  2,\n  3,\n  4,\n  5,".."\n  97,\n  98,\n  99,\n  ... 20 more items\n}""#,
            r#"log:"Sub(1) [Map] { 1 => 2 }""#,
            r#"log:"{ a: 'x\\ny' }""#,
            r#"log:"[ [Array] ]""#,
            r#"log:"{ 'k\\nx': 1 }""#,
            r#"log:"Cannot convert object to primitive value""#,
            r#"log:"{ a: [Array] }""#,
            r#"log:2"#,
        ]
    );
}

/// Pinned from Node 24.16 (recorded by running Node itself): a null-prototype array, Map, Set, Date and
/// RegExp, a subclassed Date, RegExp and Map, a hidden or shown Symbol.toStringTag, and a Proxy around a Map,
/// Date, RegExp, array or object, which Node formats from its target without running a trap.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_to_string_inspects_null_prototypes_subclasses_and_proxies() {
    let code = r#"const b = Buffer.from("hi");
const tryIt = (f) => { try { return JSON.stringify(f()); } catch (e) { return e.name + " " + e.code + " " + JSON.stringify(e.message); } };
const np = (o) => Object.setPrototypeOf(o, null);
const hide = (o, t) => Object.defineProperty(o, Symbol.toStringTag, { value: t });
const show = (o, t) => Object.defineProperty(o, Symbol.toStringTag, { value: t, enumerable: true });
class MyDate extends Date {}
class MyRe extends RegExp {}
class MyMap extends Map {}
class Foo extends Map {}
class XMapY extends Map {}
const revoked = Proxy.revocable(new Map(), {});
revoked.revoke();
const cases = [
  np([1, 2]), np([]), Object.assign(np([1]), { x: 1 }), np(new Map([[1, 2]])), np(new Map()), np(new Set([1])), np(new Set()),
  np(new Date(0)), np(new Date(NaN)), np(/x/g), Object.assign(np(new Date(0)), { x: 1 }),
  np({ a: 1 }), np({}), show(np({}), "T"), hide(np({ a: 1 }), "T"), hide(np({}), "T"), hide(np(new Map([[1, 2]])), "T"), hide(np(new Date(0)), "T"),
  new MyDate(0), new MyDate(NaN), new MyRe("x", "g"), new MyMap(), Object.assign(new Date(0), { x: 1 }),
  new Foo([[1, 2]]), new Foo(), new XMapY([[1, 2]]),
  new Proxy(new Map(), {}), new Proxy(new Map([[1, 2]]), {}), new Proxy(new Date(0), {}), new Proxy(/x/g, {}), new Proxy([1, 2], {}), new Proxy({ a: 1 }, {}),
  revoked.proxy,
  new Proxy(new Map(), { ownKeys() { throw new Error("ok"); }, getOwnPropertyDescriptor() { throw new Error("gopd"); }, getPrototypeOf() { throw new Error("gp"); } }),
];
for (const e of cases) console.log(tryIt(() => b.toString({ k: e })));
for (const e of [new MyDate(0), new MyRe("x"), new MyMap([[1, 2]]), new Foo([[1, 2]]), new XMapY([[1, 2]]), new Proxy(new Map(), {}), new Proxy(new Map([[1, 2]]), {}), new Proxy([1, 2], {}), new Proxy({ a: 1 }, {})]) console.log(tryIt(() => b.toString(e)));"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Array: null prototype] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Array(0): null prototype] [] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Array: null prototype] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Map: null prototype] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Map(0): null prototype] {} }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Set: null prototype] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Set(0): null prototype] {} }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Date: null prototype] 1970-01-01T00:00:00.000Z }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Date: null prototype] Invalid Date }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [RegExp: null prototype] /x/g }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Date: null prototype] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Object: null prototype] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Object: null prototype] {} }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [T: null prototype] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Object: null prototype] [T] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Object: null prototype] [T] {} }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Map: null prototype] [T] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Date: null prototype] [T] 1970-01-01T00:00:00.000Z }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: MyDate 1970-01-01T00:00:00.000Z }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: MyDate Invalid Date }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: MyRe /x/g }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: MyMap(0) {} }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Date] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Foo [Map]] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: Foo(0) [Map] {} }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [XMapY] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: Map(0) {} }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Map] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: 1970-01-01T00:00:00.000Z }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: /x/g }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Array] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: [Object] }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: <Revoked Proxy> }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { k: Map(0) {} }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: MyDate 1970-01-01T00:00:00.000Z""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: MyRe /x/""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: MyMap(1) { 1 => 2 }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: Foo(1) [Map] { 1 => 2 }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: XMapY(1) { 1 => 2 }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: Map(0) {}""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: Map(1) { 1 => 2 }""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: [ 1, 2 ]""#,
            r#"log:TypeError ERR_UNKNOWN_ENCODING "Unknown encoding: { a: 1 }""#,
        ]
    );
}

/// Pinned from Node 24.16: joining a sparse array of 2^32 - 1 holes fails at once, not after a walk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_to_string_rejects_a_huge_sparse_array_at_once() {
    let code = r#"const a = []; a.length = 2 ** 32 - 1;
try { Buffer.from("x").toString(a); } catch (e) { console.log(e.name, e.message); }"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(console(&result), ["log:RangeError Invalid string length"]);
}

/// Pinned from Node 24.16: each maximal invalid UTF-8 subpart becomes one U+FFFD, not each byte.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_utf8_replaces_each_broken_sequence_once() {
    let code = r#"const show = (s) => JSON.stringify(s) + " " + s.length;
const cases = [[0xf0, 0x9f], [0xe2, 0x82, 0x61], [0xc3], [0xf0, 0x9f, 0x98], [0x80, 0x80, 0x61], [0xe0, 0x80], [0xed, 0xa0, 0x80], [0xff], [0xc0, 0x80], [0xf4, 0x90, 0x80, 0x80], [0x61, 0xf0, 0x9f, 0x98, 0x80, 0x62]];
for (const c of cases) { const b = Buffer.from(c); console.log(show(b.toString()), show(b.toString("utf-8")), b.toString("UTF8") === b.toString(), b.toLocaleString() === b.toString(), String(b) === b.toString()); }
const e = Buffer.from("a😀€b");
console.log(show(e.toString("utf8", 1, 3)), show(e.toString("utf8", 3, 7)), show(e.toString(undefined, 2)), show(e.toString("utf8", 5, 5)), show(e.toString("utf8", -1, 100)), show(e.subarray(1, 4).toString()), e.toString("latin1", 0, 2).length, e.toString("hex", 1, 3));"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    assert_eq!(
        console(&result),
        [
            "log:\"\u{fffd}\" 1 \"\u{fffd}\" 1 true true true",
            "log:\"\u{fffd}a\" 2 \"\u{fffd}a\" 2 true true true",
            "log:\"\u{fffd}\" 1 \"\u{fffd}\" 1 true true true",
            "log:\"\u{fffd}\" 1 \"\u{fffd}\" 1 true true true",
            "log:\"\u{fffd}\u{fffd}a\" 3 \"\u{fffd}\u{fffd}a\" 3 true true true",
            "log:\"\u{fffd}\u{fffd}\" 2 \"\u{fffd}\u{fffd}\" 2 true true true",
            "log:\"\u{fffd}\u{fffd}\u{fffd}\" 3 \"\u{fffd}\u{fffd}\u{fffd}\" 3 true true true",
            "log:\"\u{fffd}\" 1 \"\u{fffd}\" 1 true true true",
            "log:\"\u{fffd}\u{fffd}\" 2 \"\u{fffd}\u{fffd}\" 2 true true true",
            "log:\"\u{fffd}\u{fffd}\u{fffd}\u{fffd}\" 4 \"\u{fffd}\u{fffd}\u{fffd}\u{fffd}\" 4 true true true",
            "log:\"a\u{1f600}b\" 4 \"a\u{1f600}b\" 4 true true true",
            "log:\"\u{fffd}\" 1 \"\u{fffd}\u{fffd}\u{fffd}\" 3 \"\u{fffd}\u{fffd}\u{fffd}\u{20ac}b\" 5 \"\" 0 \"a\u{1f600}\u{20ac}b\" 5 \"\u{fffd}\" 1 2 f09f",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_utf8_decodes_a_four_megabyte_body_quickly() {
    // Ten bytes per unit (ASCII, a three-byte and a four-byte character), 400 000 units. The
    // script times only the decode; the budget is generous so a loaded debug build never flakes.
    let code = r#"const b = Buffer.from("abc\u20ac\u{1f600}".repeat(400000));
const t = Date.now();
const s = b.toString();
const ms = Date.now() - t;
console.log(b.length, s.length, s.slice(0, 6) === "abc\u20ac\u{1f600}", s.slice(-6) === "abc\u20ac\u{1f600}", s.charCodeAt(2000002).toString(16));
console.log(ms);"#;
    let result = run_code(code, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", result.tests);
    let lines = console(&result);
    assert_eq!(lines[0], "log:4000000 2400000 true true d83d");
    let ms: u64 = lines[1].trim_start_matches("log:").parse().unwrap();
    assert!(ms < 4_000, "4 MB utf8 decode took {ms} ms");
}
