//! Scripts built from the export table come back through the Postman importer as the
//! importer maps them, which is not always the original call.

use crate::export_support;

use apinata_core::export::postman;
use apinata_core::import::{ImportedItem, parse_postman};
use export_support::*;

/// Each exported line and what the importer gives back for it.
const TABLE: &[(&str, &str)] = &[
    ("bru.getEnvVar(\"a\");", "bru.getEnvVar(\"a\");"),
    ("bru.setEnvVar(\"a\", 1);", "bru.setEnvVar(\"a\", 1);"),
    (
        "bru.hasEnvVar(\"a\");",
        "bru.getEnvVar(\"a\") !== undefined && bru.getEnvVar(\"a\") !== null;",
    ),
    ("bru.deleteEnvVar(\"a\");", "bru.deleteEnvVar(\"a\");"),
    ("bru.getEnvName();", "bru.getEnvName();"),
    ("bru.getVar(\"a\");", "bru.getVar(\"a\");"),
    ("bru.getFolderVar(\"a\");", "bru.getVar(\"a\");"),
    ("bru.getRequestVar(\"a\");", "bru.getVar(\"a\");"),
    ("bru.deleteVar(\"a\");", "pm.variables.unset(\"a\");"),
    (
        "bru.getCollectionVar(\"a\");",
        "bru.getCollectionVar(\"a\");",
    ),
    ("bru.getGlobalEnvVar(\"a\");", "bru.getGlobalEnvVar(\"a\");"),
    ("bru.interpolate(\"{{a}}\");", "bru.interpolate(\"{{a}}\");"),
    (
        "bru.setNextRequest(\"n\");",
        "bru.runner.setNextRequest(\"n\");",
    ),
    ("bru.runner.stopExecution();", "bru.runner.stopExecution();"),
    ("bru.runner.skipRequest();", "bru.runner.skipRequest();"),
    ("req.getUrl();", "req.getUrl();"),
    ("req.url;", "req.getUrl();"),
    ("req.getHeader(\"h\");", "req.headerList.get(\"h\");"),
    ("req.getName();", "req.getName();"),
    ("req.getHost();", "req.getHost();"),
    ("res.getStatus();", "res.getStatus();"),
    ("res.status;", "res.getStatus();"),
    ("res.getBody();", "res.getBody();"),
    ("res.getHeader(\"h\");", "res.getHeader(\"h\");"),
    ("res.getResponseTime();", "res.getResponseTime();"),
    ("res.statusText;", "res.statusText;"),
    // An assignment to the request URL or method comes back as the setter call.
    (
        "req.setUrl(\"https://x.test/b\");",
        "req.setUrl(\"https://x.test/b\");",
    ),
    ("req.setMethod(\"POST\");", "req.setMethod(\"POST\");"),
    (
        "req.setHeader(\"h\", \"v\");",
        "req.setHeader(\"h\", \"v\");",
    ),
    (
        "req.setBody({a: 1});",
        "req.getBody().update({ mode: \"raw\", raw: JSON.stringify({a: 1}) });",
    ),
    (
        "req.setHeaders({h: \"v\"});",
        "(function() { const _headers = {h: \"v\"}; for (const key in _headers) { req.setHeader(key, _headers[key]); } })();",
    ),
    (
        "JSON.stringify(res.getBody());",
        "JSON.stringify(res.getBody());",
    ),
    (
        "bru.cookies.jar().getCookie(\"https://x.test\", \"n\");",
        "bru.cookies.jar().getCookie(\"https://x.test\", \"n\");",
    ),
    (
        "bru.cookies.jar().getCookies(\"https://x.test\");",
        "bru.cookies.jar().getCookies(\"https://x.test\");",
    ),
    (
        "bru.cookies.jar().setCookie(\"https://x.test\", \"n\", \"v\");",
        "bru.cookies.jar().setCookie(\"https://x.test\", \"n\", \"v\");",
    ),
    (
        "bru.cookies.jar().deleteCookie(\"https://x.test\", \"n\");",
        "bru.cookies.jar().deleteCookie(\"https://x.test\", \"n\");",
    ),
    (
        "bru.cookies.jar().deleteCookies(\"https://x.test\");",
        "bru.cookies.jar().deleteCookies(\"https://x.test\");",
    ),
    ("test(\"t\", function() {});", "test(\"t\", function() {});"),
    ("expect(1).to.equal(1);", "expect(1).to.equal(1);"),
    (
        "bru.sendRequest(\"https://x.test/c\", (err, r) => { console.log(r.data); });",
        "await bru.sendRequest(\"https://x.test/c\", async function(err, r) { console.log(r.data); });",
    ),
];

#[test]
fn table_scripts_come_back_as_the_importer_maps_them() {
    let code: String = TABLE
        .iter()
        .map(|(line, _)| format!("        {line}\n"))
        .collect();
    let a = http("a", "GET", "http://x.test/a", INHERIT)
        + "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n"
        + &code;
    let (_d, c) = open(&[("opencollection.yml", ROOT), ("a.yml", &a)]);
    let back = parse_postman(postman(&c).json.as_bytes()).expect("parses");
    let ImportedItem::Request(r) = &back.items[0] else {
        panic!("a request")
    };
    let got: Vec<&str> = r.request.scripts.pre_request.lines().collect();
    let want: Vec<&str> = TABLE.iter().map(|(_, back)| *back).collect();
    assert_eq!(got, want);
    let unsupported: Vec<&str> = back
        .notes
        .iter()
        .chain(&r.notes)
        .map(|n| n.detail.as_str())
        .filter(|d| d.starts_with("uses "))
        .collect();
    assert_eq!(
        unsupported,
        ["uses pm.variables.unset, which has no counterpart here; the line was kept as written"]
    );
}
