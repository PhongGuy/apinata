//! Local modules and the path module in scripts.

use crate::script_support;

use apinata_core::{Outcome, VarResult};
use script_support::*;
use std::path::Path;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Sends `r.yml` from the collection at `col` after writing it with `scripts` and `extra` YAML.
async fn send_in(col: &Path, scripts: &str, extra: &str) -> apinata_core::RequestResult {
    let s = MockServer::start().await;
    Mock::given(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    std::fs::write(col.join("opencollection.yml"), ROOT).expect("root");
    let text = scripted("R", &format!("{}/r", s.uri()), scripts, extra);
    std::fs::write(col.join("r.yml"), text).expect("request");
    send(col, "r.yml", on()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn path_is_posix_with_the_collection_root_as_working_directory() {
    let code = r#"console.log(typeof Object.getOwnPropertyDescriptor(globalThis, "path").get);
const R = bru.cwd();
const show = (v) => (typeof v === "string" ? v.split(R).join("<R>") : v);
const p = require("path");
console.log(p === path, typeof p.resolve);
console.log(JSON.stringify([
  p.resolve("a/b", "../c"), p.resolve("/x", "y"), p.resolve(), p.resolve("a/./b/"),
  p.join("a", "b/../c", "./d"), p.join(""), p.join("/a/", "/b"), p.join("a", "..", ".."),
  p.dirname("/a/b/c.js"), p.dirname("a"), p.dirname("/a"),
  p.basename("/a/b/c.js"), p.basename("/a/b/c.js", ".js"), p.basename("/a/b/"),
  p.extname("x.tar.gz"), p.extname(".hidden"), p.extname("a."), p.extname("a"),
  p.relative(R, R + "/a/b"), p.relative(R + "/a/b", R + "/x"), p.relative("a", "b"),
  p.normalize("/a//b/../c/."), p.normalize("a/../.."), p.normalize(""), p.normalize("a/b/"),
  p.isAbsolute("/a"), p.isAbsolute("a"), p.sep].map(show)));"#;
    let dir = write_tree(&[]);
    let result = send_in(dir.path(), &script("tests", code), "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", tests(&result));
    assert_eq!(
        console(&result),
        [
            "log:function",
            "log:true function",
            r#"log:["<R>/a/c","/x/y","<R>","<R>/a/b","a/c/d",".","/a/b","..","/a/b",".","/","c.js","c","b",".gz","",".","","a/b","../../x","../b","/a/c","..",".","a/b/",true,false,"/"]"#,
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn path_stays_unsupported_in_expressions() {
    let extra = "  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: \"path.resolve('a')\"\n        method: jsonq\n      variable:\n        name: y\n        scope: runtime\n";
    let dir = write_tree(&[]);
    let result = send_in(dir.path(), &script("tests", "1"), extra).await;
    match result.post_response_vars[0].result() {
        VarResult::Skipped { reason } => {
            assert!(
                reason.contains("path is not supported by Apinata"),
                "{reason}"
            )
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_modules_resolve_against_the_root_and_their_own_directory() {
    let dir = write_tree(&[
        (
            "lib/math.js",
            "const util = require(\"./util\");\nmodule.exports = { twice: (n) => util.add(n, n), file: typeof __filename };",
        ),
        ("lib/util.js", "exports.add = (a, b) => a + b;"),
        ("util.js", "exports.add = () => \"root util\";"),
        ("lib/conf.cjs", "module.exports = \"cjs\";"),
        (
            "lib/env.js",
            "module.exports = () => [typeof bru, typeof req, typeof res, typeof console, typeof expect].join(\",\");",
        ),
    ]);
    let code = r#"const math = require("./lib/math");
console.log(math.twice(21), math.file, require("./util").add(), require("./lib/conf.cjs"));
console.log(require(bru.cwd() + "/lib/util.js").add(1, 2), require("./lib/env")(), typeof __filename);"#;
    let result = send_in(dir.path(), &script("tests", code), "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", tests(&result));
    assert_eq!(
        console(&result),
        [
            "log:42 undefined root util cjs",
            "log:3 object,object,function,object,function undefined",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_unreadable_and_foreign_names_throw() {
    let big = format!("//{}", "x".repeat(10 * 1024 * 1024));
    let dir = write_tree(&[("lib.d/x.txt", "x"), ("big.js", big.as_str())]);
    let code = r#"for (const name of ["./nope", "./nope.js", bru.cwd() + "/nope", "/definitely/not/here", "./lib.d", "../outside", "./big"]) {
  try { require(name); console.log("loaded", name); } catch (e) { console.log(e.message.split(bru.cwd()).join("<R>")); }
}"#;
    let result = send_in(dir.path(), &script("tests", code), "").await;
    assert_eq!(
        console(&result),
        [
            "log:Cannot find module ./nope",
            "log:Cannot find module ./nope.js",
            "log:Cannot find module <R>/nope",
            "log:Cannot find module /definitely/not/here",
            "log:Error loading local module ./lib.d",
            "log:Cannot find module ../outside",
            "log:Error loading local module ./big",
        ]
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modules_outside_the_root_are_refused() {
    let dir = write_tree(&[
        ("secret.js", "module.exports = \"leaked\";"),
        (
            "col/sub/inner.js",
            "module.exports = require(\"../../secret\");",
        ),
    ]);
    std::os::unix::fs::symlink(dir.path().join("secret.js"), dir.path().join("col/link.js"))
        .expect("symlink");
    let code = r#"for (const name of ["../secret", "./link", "./sub/inner"]) {
  try { console.log(require(name)); } catch (e) { console.log(e.message); }
}"#;
    let result = send_in(&dir.path().join("col"), &script("tests", code), "").await;
    assert_eq!(
        console(&result),
        ["log:Access to files outside of the collectionPath is not allowed."; 3]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cache_is_per_key_and_per_phase() {
    let dir = write_tree(&[
        (
            "count.js",
            "globalThis.loads = (globalThis.loads || 0) + 1;\nmodule.exports = { n: globalThis.loads };",
        ),
        ("a/x.js", "module.exports = require(\"./util\");"),
        ("a/z.js", "module.exports = require(\"../a/util\");"),
        (
            "a/util.js",
            "globalThis.utilLoads = (globalThis.utilLoads || 0) + 1;\nmodule.exports = \"a\";",
        ),
        ("b/y.js", "module.exports = require(\"./util\");"),
        ("b/util.js", "module.exports = \"b\";"),
    ]);
    let scripts = script(
        "before-request",
        "require(\"./count\");\nconsole.log(\"pre\", globalThis.loads);",
    ) + &script(
        "tests",
        r#"const first = require("./count");
console.log(first === require("./count"), globalThis.loads, require("./count.js").n, globalThis.loads, require("./a/x"), require("./b/y"), require("./a/z"), globalThis.utilLoads);"#,
    );
    let result = send_in(dir.path(), &scripts, "").await;
    assert_eq!(result.outcome, Outcome::Passed, "{:?}", tests(&result));
    assert_eq!(console(&result), ["log:pre 1", "log:true 1 2 2 a b a 1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_circular_require_ends_the_phase_with_an_error() {
    let dir = write_tree(&[
        ("a.js", "module.exports = require(\"./b\");"),
        ("b.js", "module.exports = require(\"./a\");"),
    ]);
    let s = MockServer::start().await;
    std::fs::write(dir.path().join("opencollection.yml"), ROOT).expect("root");
    let text = scripted(
        "R",
        &format!("{}/r", s.uri()),
        &script("before-request", "require(\"./a\");"),
        "",
    );
    std::fs::write(dir.path().join("r.yml"), text).expect("request");
    let sent = send(dir.path(), "r.yml", limited(10_000));
    let result = tokio::time::timeout(std::time::Duration::from_secs(60), sent)
        .await
        .expect("the phase ends");
    // The overflow may surface in a call or while parsing a module body, so only the outcome is pinned.
    assert_eq!(result.outcome, Outcome::Errored);
    assert!(!script_error(&result).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_module_that_throws_is_attributed_to_the_requiring_script() {
    let dir = write_tree(&[("boom.js", "throw new Error(\"boom\");")]);
    let code = "const x = 1;\nrequire(\"./boom\");";
    let result = send_in(dir.path(), &script("before-request", code), "").await;
    assert_eq!(result.outcome, Outcome::Errored);
    let message = script_error(&result);
    assert_eq!(message, "this request's script, line 2: Error: boom");
}
