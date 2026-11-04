//! The esbuild bundles (buffer, ajv) load lazily and behave as their published versions.

use crate::script_support;

use apinata_core::Outcome;
use script_support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const VECTORS: &str = include_str!("fixtures/jwt/vectors.json");

async fn run_code(code: &str) -> apinata_core::RequestResult {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let text = scripted("R", &format!("{}/r", s.uri()), &script("tests", code), "");
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", tests(&result));
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn buffer_is_the_npm_buffer_and_signs_like_node() {
    let vectors: serde_json::Value = serde_json::from_str(VECTORS).expect("vectors");
    let code = format!(
        r#"console.log(typeof Object.getOwnPropertyDescriptor(globalThis, "Buffer").get);
const b = Buffer.from([0, 1, 2, 250]);
console.log(Buffer.from("hi").toString("base64"), Buffer.from("aGk=", "base64").toString(), b.toString("hex"), b.readUInt32BE(0), Buffer.byteLength("é"), Buffer.isBuffer(b), Buffer.concat([Buffer.from("a"), Buffer.from("b")]).toString(), b instanceof Uint8Array, JSON.stringify(Buffer.from("hi")));
console.log(require("buffer").Buffer === Buffer, typeof Object.getOwnPropertyDescriptor(globalThis, "Buffer").get);
const P = {{ sub: "1234567890", name: "Jörg ✓", admin: true, iat: 1700000000 }};
const key = Buffer.from(Array.from({{ length: 32 }}, (_, i) => i));
console.log(jwt.sign(P, key) === {}, jwt.verify({}, key).admin);"#,
        vectors["HS256_buffer"], vectors["HS256_buffer"]
    );
    let result = run_code(&code).await;
    assert_eq!(
        console(&result),
        [
            "log:function",
            r#"log:aGk= hi 000102fa 66298 2 true ab true {"type":"Buffer","data":[104,105]}"#,
            "log:true undefined",
            "log:true true",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ajv_validates_with_formats_and_logs_to_the_console() {
    let code = r#"console.log([typeof Object.getOwnPropertyDescriptor(globalThis, "Ajv").get, typeof Object.getOwnPropertyDescriptor(globalThis, "addFormats").get].join());
const ajv = new Ajv({ allErrors: true });
addFormats(ajv);
const v = ajv.compile({ type: "object", properties: { email: { type: "string", format: "email" }, at: { type: "string", format: "date-time" }, n: { type: "integer", minimum: 1 } }, required: ["email"] });
console.log(v({ email: "a@b.co", at: "2024-01-02T03:04:05Z", n: 2 }), v({ email: "nope", at: "x", n: 0 }), JSON.stringify(v.errors.map((e) => e.instancePath + " " + e.message)));
try { new Ajv().compile({ type: "string", format: "email" }); } catch (e) { console.log(e.message); }
new Ajv({ strict: false }).compile({ type: "string", format: "email" });
const A = require("ajv"), F = require("ajv-formats");
console.log(A === Ajv, A.default === Ajv, F === addFormats, F.default === addFormats, typeof Object.getOwnPropertyDescriptor(globalThis, "Ajv").get);"#;
    let result = run_code(code).await;
    assert_eq!(
        console(&result),
        [
            "log:function,function",
            r#"log:true false ["/email must match format \"email\"","/at must match format \"date-time\"","/n must be >= 1"]"#,
            r##"log:unknown format "email" ignored in schema at path "#""##,
            r##"warn:unknown format "email" ignored in schema at path "#""##,
            r##"warn:unknown format "email" ignored in schema at path "#""##,
            "log:true true true true undefined",
        ]
    );
}
