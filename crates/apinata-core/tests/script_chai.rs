//! chai 4.5.0 through test/expect/assert, the json extensions and require.

use crate::script_support;

use apinata_core::Outcome;
use script_support::*;
use sha2::Digest;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chai_tests_pass_and_fail_with_chais_messages() {
    let s = MockServer::start().await;
    Mock::given(path("/j"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":7,"tags":["a","b"]}"#))
        .mount(&s)
        .await;
    let code = r#"test("ok", () => expect(res.status).to.equal(200));
test("bad", () => expect(res.status).to.equal(201));
test("assert", () => assert.strictEqual(res.body.id, 7));
test("json", () => expect(res.body).to.be.json);
test("jsonBody", () => expect(res.body).to.have.jsonBody("tags[1]", "b"));
test("jsonBody miss", () => expect(res.body).to.have.jsonBody("nope"));
test("schema", () => expect(res.body).to.have.jsonSchema({}));
const c = require("chai"); console.log(typeof c.expect, c.expect === expect);
try { require("fs") } catch (e) { console.log(e.message) }
console.log(typeof require("ajv"))
try { require("./lib.js") } catch (e) { console.log(e.message) }"#;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/j", s.uri()), &script("tests", code), ""),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(result.outcome, Outcome::Failed);
    assert_eq!(
        tests(&result),
        [
            "tests ok: pass",
            "tests bad: fail expected 200 to equal 201",
            "tests assert: pass",
            "tests json: pass",
            "tests jsonBody: pass",
            "tests jsonBody miss: fail expected body to have nested property 'nope'",
            "tests schema: pass",
        ]
    );
    assert_eq!(
        console(&result),
        [
            "log:function true",
            "log:Cannot find module fs",
            "log:function",
            "log:Cannot find module ./lib.js",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expect_works_in_the_pre_request_phase() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = "test('pre', () => expect(req.getMethod()).to.equal('GET'))";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted(
                "R",
                &format!("{}/ok", s.uri()),
                &script("before-request", code),
                "",
            ),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(tests(&result), ["pre-request pre: pass"]);
}

#[test]
fn third_party_lists_the_vendored_chai_with_its_hash() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let listing = std::fs::read_to_string(root.join("THIRD-PARTY.md")).expect("THIRD-PARTY.md");
    let bytes = std::fs::read(root.join("crates/apinata-core/src/script/vendor/chai-4.5.0.js"))
        .expect("chai");
    let hash: String = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(
        listing.contains("chai") && listing.contains("4.5.0") && listing.contains(&hash),
        "{hash}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_top_level_assertion_failure_takes_the_scripts_line_not_chais() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let code = "console.log(typeof chai);\nexpect(1).to.equal(2);";
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/ok", s.uri()), &script("tests", code), ""),
        ),
    ]);
    let result = send(dir.path(), "r.yml", on()).await;
    assert_eq!(console(&result), ["log:undefined"]);
    assert_eq!(
        tests(&result),
        [
            "tests Test script error: fail this request's tests, line 2: AssertionError: expected 1 to equal 2"
        ]
    );
}

/// The body every `jsonSchema` case below validates.
const SCHEMA_BODY: &str =
    r#"{"id":7,"email":"a@b.co","at":"2024-01-15T10:30:00Z","big":2147483648,"zip":"62701"}"#;

/// Runs `code` as the tests script of a request whose response body is [`SCHEMA_BODY`].
async fn schema_tests(code: &str) -> Vec<String> {
    let s = MockServer::start().await;
    Mock::given(path("/j"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SCHEMA_BODY))
        .mount(&s)
        .await;
    let dir = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &scripted("R", &format!("{}/j", s.uri()), &script("tests", code), ""),
        ),
    ]);
    tests(&send(dir.path(), "r.yml", on()).await)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_schema_matches_and_mismatches_like_the_reference() {
    let code = r#"const body = res.getBody();
test("match", () => expect(body).to.have.jsonSchema({ type: "object", required: ["id"] }));
test("mismatch", () => expect(body).to.have.jsonSchema({ properties: { id: { type: "string" } } }));
test("not", () => expect(body).to.not.have.jsonSchema({ type: "array" }));
test("not.to", () => expect(body).not.to.have.jsonSchema({ type: "array" }));
test("to.have.not", () => expect(body).to.have.not.jsonSchema({ type: "array" }));
test("not on a match", () => expect(body).to.not.have.jsonSchema({ type: "object" }));"#;
    assert_eq!(
        schema_tests(code).await,
        [
            "tests match: pass",
            &format!(
                "tests mismatch: fail expected {SCHEMA_BODY} to match JSON schema, validation errors: \
                 [{{\"instancePath\":\"/id\",\"schemaPath\":\"#/properties/id/type\",\"keyword\":\"type\",\
                 \"params\":{{\"type\":\"string\"}},\"message\":\"must be string\"}}]"
            ),
            "tests not: pass",
            "tests not.to: pass",
            "tests to.have.not: pass",
            &format!("tests not on a match: fail expected {SCHEMA_BODY} to not match JSON schema"),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_schema_checks_formats_and_takes_ajv_options() {
    let code = r#"const body = res.getBody();
const one = (key, schema) => ({ type: "object", properties: { [key]: schema }, required: [key] });
test("email", () => expect(body).to.have.jsonSchema(one("email", { type: "string", format: "email" })));
test("email rejected", () => expect(body).to.not.have.jsonSchema(one("zip", { type: "string", format: "email" })));
test("date-time", () => expect(body).to.have.jsonSchema(one("at", { type: "string", format: "date-time" })));
test("date-time rejected", () => expect(body).to.not.have.jsonSchema(one("zip", { type: "string", format: "date-time" })));
test("int32", () => expect(body).to.have.jsonSchema(one("id", { type: "integer", format: "int32" })));
test("int32 rejected", () => expect(body).to.not.have.jsonSchema(one("big", { type: "integer", format: "int32" })));
test("coerceTypes", () => {
  const schema = one("zip", { type: "integer" });
  expect(body).to.not.have.jsonSchema(schema);
  expect(body).to.have.jsonSchema(schema, { coerceTypes: true });
});
test("strict", () => expect(body).to.have.jsonSchema(one("id", { type: "number", customKeyword: true })));
test("strict false", () => expect(body).to.have.jsonSchema(one("id", { type: "number", customKeyword: true }), { strict: false }));
test("draft 2020-12", () => expect(body).to.have.jsonSchema({ $schema: "https://json-schema.org/draft/2020-12/schema", type: "object" }));
test("draft-07", () => expect(body).to.have.jsonSchema({ $schema: "http://json-schema.org/draft-07/schema#", type: "object" }));"#;
    assert_eq!(
        schema_tests(code).await,
        [
            "tests email: pass",
            "tests email rejected: pass",
            "tests date-time: pass",
            "tests date-time rejected: pass",
            "tests int32: pass",
            "tests int32 rejected: pass",
            "tests coerceTypes: pass",
            "tests strict: fail JSON schema compile error: strict mode: unknown keyword: \"customKeyword\"",
            "tests strict false: pass",
            "tests draft 2020-12: fail Unsupported JSON Schema version: \"https://json-schema.org/draft/2020-12/schema\". \
             Apinata currently only supports Draft-07 (http://json-schema.org/draft-07/schema#). \
             Please update your schema to be Draft-07 compatible and remove the $schema property.",
            "tests draft-07: pass",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_schema_ignores_a_script_rebinding_require_and_ajv() {
    let code = r#"globalThis.require = () => { throw new Error("rebound"); };
Ajv = null; addFormats = null;
test("rebound", () => expect(res.getBody()).to.have.jsonSchema({ type: "object" }, { allErrors: false }));
test("default", () => expect(res.getBody()).to.have.jsonSchema({ type: "object" }));"#;
    assert_eq!(
        schema_tests(code).await,
        ["tests rebound: pass", "tests default: pass"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_schema_reports_every_error_unless_options_say_otherwise() {
    let code = r#"const body = res.getBody();
const schema = { properties: { id: { type: "string" }, email: { type: "integer" } } };
test("all errors", () => expect(body).to.have.jsonSchema(schema));
test("first error", () => expect(body).to.have.jsonSchema(schema, { allErrors: false }));
test("not over a compile error", () => expect(body).to.not.have.jsonSchema({ type: "bogus" }));"#;
    let id = r##"{"instancePath":"/id","schemaPath":"#/properties/id/type","keyword":"type","params":{"type":"string"},"message":"must be string"}"##;
    let email = r##"{"instancePath":"/email","schemaPath":"#/properties/email/type","keyword":"type","params":{"type":"integer"},"message":"must be integer"}"##;
    let fail = |name: &str, errors: &str| {
        format!(
            "tests {name}: fail expected {SCHEMA_BODY} to match JSON schema, validation errors: [{errors}]"
        )
    };
    // A negated compile error passes its assert and falls through to the missing validator.
    assert_eq!(
        schema_tests(code).await,
        [
            fail("all errors", &format!("{id},{email}")),
            fail("first error", id),
            "tests not over a compile error: fail not a function".to_owned(),
        ]
    );
}
