//! Every tool's `structuredContent` validates against the `outputSchema` it lists, so a client
//! that checks results against the schema never rejects a real answer.

use crate::common;
use crate::git_support;

use std::collections::{BTreeMap, BTreeSet};

use common::{MOCK_ITEMS, TestServer, http_get, port_of, request};
use git_support::Sandbox;
use rmcp::model::Tool;
use serde_json::{Map, Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Keywords that only annotate a schema and constrain nothing the checker needs to see.
const ANNOTATIONS: &[&str] = &[
    "$schema",
    "$defs",
    "title",
    "description",
    "format",
    "default",
    "examples",
    "readOnly",
    "writeOnly",
    "deprecated",
];

/// A strict checker for the JSON Schema subset schemars emits: an unknown keyword fails the
/// test, so a construct the checker cannot judge is never silently passed.
struct Schema<'a> {
    root: &'a Map<String, Value>,
}

impl Schema<'_> {
    fn check(&self, schema: &Value, value: &Value, at: &str) -> Result<(), String> {
        let schema = match schema {
            Value::Bool(true) => return Ok(()),
            Value::Bool(false) => return Err(format!("{at}: schema `false` admits nothing")),
            Value::Object(map) => map,
            other => return Err(format!("{at}: not a schema: {other}")),
        };
        for (key, rule) in schema {
            match key.as_str() {
                "$ref" => self.check(self.resolve(rule, at)?, value, at)?,
                "type" => check_type(rule, value, at)?,
                "enum" => {
                    let options = rule.as_array().expect("enum array");
                    if !options.contains(value) {
                        return Err(format!("{at}: {value} not in enum {rule}"));
                    }
                }
                "const" if rule != value => {
                    return Err(format!("{at}: {value} is not the const {rule}"));
                }
                "const" => {}
                "required" => {
                    if let Some(object) = value.as_object() {
                        for name in rule.as_array().expect("required array") {
                            let name = name.as_str().expect("required name");
                            if !object.contains_key(name) {
                                return Err(format!("{at}: required `{name}` missing in {value}"));
                            }
                        }
                    }
                }
                "properties" => {
                    if let Some(object) = value.as_object() {
                        for (name, sub) in rule.as_object().expect("properties map") {
                            if let Some(field) = object.get(name) {
                                self.check(sub, field, &format!("{at}.{name}"))?;
                            }
                        }
                    }
                }
                "additionalProperties" => {
                    if let Some(object) = value.as_object() {
                        let known = schema.get("properties").and_then(Value::as_object);
                        for (name, field) in object {
                            if known.is_none_or(|k| !k.contains_key(name)) {
                                self.check(rule, field, &format!("{at}.{name}"))?;
                            }
                        }
                    }
                }
                "items" => {
                    if let Some(items) = value.as_array() {
                        for (i, item) in items.iter().enumerate() {
                            self.check(rule, item, &format!("{at}[{i}]"))?;
                        }
                    }
                }
                "minimum" => {
                    if let (Some(n), Some(min)) = (value.as_f64(), rule.as_f64())
                        && n < min
                    {
                        return Err(format!("{at}: {n} below minimum {min}"));
                    }
                }
                "maximum" => {
                    if let (Some(n), Some(max)) = (value.as_f64(), rule.as_f64())
                        && n > max
                    {
                        return Err(format!("{at}: {n} above maximum {max}"));
                    }
                }
                "allOf" => {
                    for sub in rule.as_array().expect("allOf array") {
                        self.check(sub, value, at)?;
                    }
                }
                "anyOf" => {
                    let subs = rule.as_array().expect("anyOf array");
                    if !subs.iter().any(|s| self.check(s, value, at).is_ok()) {
                        return Err(format!("{at}: {value} matches no anyOf branch"));
                    }
                }
                "oneOf" => {
                    let subs = rule.as_array().expect("oneOf array");
                    let hits = subs
                        .iter()
                        .filter(|s| self.check(s, value, at).is_ok())
                        .count();
                    if hits != 1 {
                        return Err(format!("{at}: {value} matches {hits} oneOf branches"));
                    }
                }
                other if ANNOTATIONS.contains(&other) => {}
                other => panic!("{at}: the checker does not know `{other}`"),
            }
        }
        Ok(())
    }

    /// Follow a local `#/$defs/<name>` reference; schemars emits no other kind.
    fn resolve(&self, reference: &Value, at: &str) -> Result<&Value, String> {
        let reference = reference.as_str().expect("$ref string");
        let name = reference
            .strip_prefix("#/$defs/")
            .ok_or_else(|| format!("{at}: unsupported $ref {reference}"))?;
        self.root
            .get("$defs")
            .and_then(|defs| defs.get(name))
            .ok_or_else(|| format!("{at}: dangling $ref {reference}"))
    }
}

fn check_type(rule: &Value, value: &Value, at: &str) -> Result<(), String> {
    let is = |name: &str| match name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        other => panic!("{at}: unknown type `{other}`"),
    };
    let ok = match rule {
        Value::String(name) => is(name),
        Value::Array(names) => names.iter().any(|n| is(n.as_str().expect("type name"))),
        other => panic!("{at}: bad type {other}"),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{at}: {value} is not of type {rule}"))
    }
}

/// Calls tools, validates each result against its listed schema, and records which tools ran.
struct Exercise {
    schemas: BTreeMap<String, Map<String, Value>>,
    seen: BTreeSet<String>,
    failures: Vec<String>,
}

impl Exercise {
    fn new(tools: &[Tool]) -> Exercise {
        let schemas = tools
            .iter()
            .filter_map(|t| {
                let schema = t.output_schema.as_ref()?;
                Some((t.name.to_string(), (**schema).clone()))
            })
            .collect();
        Exercise {
            schemas,
            seen: BTreeSet::new(),
            failures: Vec::new(),
        }
    }

    /// Call `tool` on `server`, require success, and validate its structured content.
    async fn run(&mut self, server: &TestServer, tool: &str, args: Value) -> Value {
        let got = server.ok(tool, args.clone()).await;
        let schema = self
            .schemas
            .get(tool)
            .unwrap_or_else(|| panic!("{tool} lists no outputSchema"));
        let checker = Schema { root: schema };
        if let Err(why) = checker.check(&Value::Object(schema.clone()), &got, tool) {
            self.failures.push(format!("{tool} {args}: {why}"));
        }
        self.seen.insert(tool.to_owned());
        got
    }
}

const DEV: &str = "name: dev\nvariables:\n  - name: base\n    value: http://api.test/v1\n  - name: token\n    secret: true\n";

const SPEC: &str = r#"{"openapi":"3.0.0","info":{"title":"Petstore","version":"1"},
"paths":{"/pets":{"get":{"summary":"List pets","tags":["pets"]}}}}"#;

/// SPEC with one more operation, for an update.
const SPEC_V2: &str = r#"{"openapi":"3.0.0","info":{"title":"Petstore","version":"2"},
"paths":{"/pets":{"get":{"summary":"List pets","tags":["pets"]}},
"/pets/{id}":{"get":{"summary":"Get pet","tags":["pets"]}}}}"#;

/// SPEC with its one operation replaced, so an update with prune removes `List pets`.
const SPEC_V3: &str = r#"{"openapi":"3.0.0","info":{"title":"Petstore","version":"3"},
"paths":{"/owners":{"get":{"summary":"List owners","tags":["owners"]}}}}"#;

/// A request with an assertion, a `set-cookie` answer and an example, aimed at `uri`.
fn send_target(uri: &str) -> String {
    format!(
        concat!(
            "info:\n  name: r\n  type: http\n  seq: 3\nhttp:\n  method: GET\n  url: \"{uri}/r?a=1\"\n",
            "  params:\n    - name: b\n      value: \"2\"\n      type: query\n",
            "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n",
            "examples:\n  - name: saved\n    description: first\n    response:\n      status: 200\n"
        ),
        uri = uri
    )
}

/// A request whose client-credentials OAuth 2.0 token comes from `uri`.
fn oauth_target(uri: &str) -> String {
    format!(
        concat!(
            "info:\n  name: o\n  type: http\n  seq: 4\nhttp:\n  method: GET\n  url: \"{uri}/r\"\n",
            "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"{uri}/token\"\n",
            "    credentials:\n      clientId: app\n      clientSecret: cs\n    scope: read\n",
            "    tokenConfig:\n      id: main\n      placement:\n        header: Bearer\n"
        ),
        uri = uri
    )
}

/// An introspection answer with every root, so each optional field of the result is filled.
const GRAPHQL_SCHEMA: &str = r#"{"data":{"__schema":{"queryType":{"name":"Q"},"mutationType":{"name":"M"},"subscriptionType":{"name":"S"},"types":[{"kind":"OBJECT","name":"Q","fields":[{"name":"a","args":[],"type":{"kind":"SCALAR","name":"Int"}}]},{"kind":"OBJECT","name":"M","fields":[{"name":"b","args":[],"type":{"kind":"SCALAR","name":"Int"}}]},{"kind":"OBJECT","name":"S","fields":[{"name":"c","args":[],"type":{"kind":"SCALAR","name":"Int"}}]},{"kind":"SCALAR","name":"Int"}]}}}"#;

async fn upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "tok",
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "rt",
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(GRAPHQL_SCHEMA))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("plain words", "text/plain"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "sid=abc; Path=/")
                .set_body_raw("{\"ok\":true}", "application/json"),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn every_tool_result_matches_its_output_schema() {
    let up = upstream().await;
    let sb = Sandbox::new();
    let target = send_target(&up.uri());
    let oauth = oauth_target(&up.uri());
    let mut files: Vec<(&str, &str)> = MOCK_ITEMS
        .iter()
        .copied()
        .filter(|(rel, _)| *rel != "environments/dev.yml")
        .collect();
    let a = request("a", 1, "GET", "http://127.0.0.1:9/a");
    let plain = request("p", 6, "GET", &format!("{}/plain", up.uri()));
    let graphql = format!(
        "info:\n  name: q\n  type: graphql\n  seq: 7\ngraphql:\n  url: \"{}/graphql\"\n",
        up.uri()
    );
    files.extend([
        ("environments/dev.yml", DEV),
        ("r.yml", target.as_str()),
        ("o.yml", oauth.as_str()),
        ("p.yml", plain.as_str()),
        ("q.yml", graphql.as_str()),
        (
            "f/folder.yml",
            "info:\n  name: f\n  type: folder\n  seq: 5\n",
        ),
        ("f/a.yml", a.as_str()),
    ]);
    let server = TestServer::start_with(&files, |options| {
        let git = sb
            .options()
            .env("GIT_CEILING_DIRECTORIES", options.root().parent().unwrap());
        options.git(git)
    })
    .await;
    let mut ex = Exercise::new(&server.tools().await);

    // Reads, first with the optional parts empty, then filled where a second call can fill them.
    ex.run(&server, "get_collection", json!({})).await;
    ex.run(&server, "list_items", json!({})).await;
    ex.run(&server, "search", json!({"query": "r"})).await;
    ex.run(&server, "search", json!({"query": "r", "limit": 1}))
        .await;
    ex.run(&server, "check_collection", json!({})).await;
    ex.run(
        &server,
        "replace",
        json!({"pattern": "nothing-matches", "replacement": "x", "dry_run": true}),
    )
    .await;
    ex.run(
        &server,
        "replace",
        json!({"pattern": "127.0.0.1:9", "replacement": "127.0.0.1:8", "dry_run": true}),
    )
    .await;
    ex.run(&server, "list_trusted_hosts", json!({})).await;
    ex.run(&server, "read_request", json!({"path": "r.yml"}))
        .await;
    ex.run(&server, "read_request", json!({"path": "f/a.yml"}))
        .await;
    ex.run(&server, "read_folder", json!({"path": ""})).await;
    ex.run(&server, "read_folder", json!({"path": "f"})).await;
    ex.run(&server, "read_environment", json!({"name": "dev"}))
        .await;
    ex.run(
        &server,
        "generate_code",
        json!({"path": "r.yml", "target": "curl"}),
    )
    .await;

    // Git, outside a repository and then inside one with an edit.
    ex.run(&server, "git_status", json!({})).await;
    sb.git(server.root(), &["init", "-q", "-b", "main"]);
    sb.commit_all(server.root(), "base");
    server.write("f/a.yml", &a.replace("/a", "/b"));
    ex.run(&server, "git_status", json!({})).await;
    ex.run(&server, "git_diff", json!({"path": "f/a.yml"}))
        .await;
    ex.run(
        &server,
        "git_commit",
        json!({"message": "edit", "paths": ["f/a.yml"]}),
    )
    .await;

    // The mock, idle and then with a hit.
    let started = ex
        .run(&server, "mock_start", json!({"env": "dev", "port": 0}))
        .await;
    ex.run(&server, "mock_hits", json!({})).await;
    http_get(port_of(started["url"].as_str().expect("url")), "/ping").await;
    ex.run(&server, "mock_hits", json!({})).await;
    ex.run(&server, "mock_stop", json!({})).await;

    // Runtime state: empty, then set, then cleared.
    ex.run(&server, "list_runtime_variables", json!({})).await;
    ex.run(
        &server,
        "set_runtime_variable",
        json!({"name": "n", "value": "v"}),
    )
    .await;
    ex.run(&server, "list_runtime_variables", json!({})).await;
    ex.run(&server, "clear_runtime_variables", json!({})).await;
    ex.run(&server, "list_cookies", json!({})).await;

    // Network, all against the local upstream.
    ex.run(&server, "send_request", json!({"path": "r.yml"}))
        .await;
    ex.run(
        &server,
        "send_request",
        json!({"path": "p.yml", "body_filter": "$.a"}),
    )
    .await;
    ex.run(&server, "list_cookies", json!({})).await;
    ex.run(&server, "clear_cookies", json!({})).await;
    ex.run(&server, "graphql_schema", json!({"path": "q.yml"}))
        .await;
    ex.run(
        &server,
        "graphql_schema",
        json!({"path": "q.yml", "format": "json", "max_bytes": 20}),
    )
    .await;
    ex.run(
        &server,
        "save_example",
        json!({"path": "r.yml", "name": "one"}),
    )
    .await;
    ex.run(&server, "export_har", json!({"path": "r.yml"}))
        .await;
    ex.run(&server, "export_openapi", json!({})).await;
    ex.run(
        &server,
        "export_openapi",
        json!({"format": "yaml", "max_bytes": 20}),
    )
    .await;
    ex.run(&server, "export_postman", json!({})).await;
    ex.run(&server, "run_folder", json!({"path": ""})).await;
    ex.run(&server, "run_folder", json!({"path": "f"})).await;
    for action in ["status", "fetch", "status", "refresh", "clear"] {
        ex.run(
            &server,
            "oauth2_token",
            json!({"path": "o.yml", "action": action}),
        )
        .await;
    }

    // Writes to the tree.
    ex.run(
        &server,
        "update_request",
        json!({"path": "f/a.yml", "name": "a2"}),
    )
    .await;
    ex.run(
        &server,
        "update_folder",
        json!({"path": "", "docs": "# Shop"}),
    )
    .await;
    ex.run(
        &server,
        "create_request",
        json!({"parent": "", "name": "Fresh"}),
    )
    .await;
    ex.run(
        &server,
        "import_request",
        json!({"text": "curl http://127.0.0.1:9/ping", "format": "curl", "parent": "f", "name": "ping"}),
    )
    .await;
    let imported = ex
        .run(&server, "import_collection", json!({"text": SPEC}))
        .await;
    // An update dry run fills the flattened update report.
    let folder = imported["collections"][0]["path"]
        .as_str()
        .expect("imported folder");
    let update = ex
        .run(
            &server,
            "import_collection",
            json!({"text": SPEC_V2, "update": true, "into": folder, "dry_run": true}),
        )
        .await;
    assert_eq!(update["dry_run"], true, "{update}");
    // An update of a collection without a baseline fills `notes`, so its item schema is checked.
    let noted = ex
        .run(
            &server,
            "import_collection",
            json!({"text": SPEC_V2, "update": true}),
        )
        .await;
    assert!(
        noted["notes"].as_array().is_some_and(|n| !n.is_empty()),
        "{noted}"
    );
    // A real update of the imported folder prunes its one request, so a whole prune restores.
    let pruned = ex
        .run(
            &server,
            "import_collection",
            json!({"text": SPEC_V3, "update": true, "into": folder, "prune": true}),
        )
        .await;
    assert_eq!(
        pruned["pruned"].as_array().map(Vec::len),
        Some(1),
        "{pruned}"
    );
    ex.run(&server, "restore_pruned", json!({})).await;
    ex.run(&server, "scrub_example_userinfo", json!({})).await;
    ex.run(&server, "create_folder", json!({"parent": "", "name": "g"}))
        .await;
    ex.run(
        &server,
        "clone_request",
        json!({"path": "Fresh.yml", "name": "Copy"}),
    )
    .await;
    ex.run(
        &server,
        "clone_folder",
        json!({"path": "f", "name": "f copy"}),
    )
    .await;
    ex.run(
        &server,
        "rename_item",
        json!({"path": "Copy.yml", "name": "Moved"}),
    )
    .await;
    ex.run(
        &server,
        "move_item",
        json!({"path": "Moved.yml", "inside": "g"}),
    )
    .await;
    ex.run(&server, "delete_item", json!({"path": "g/Moved.yml"}))
        .await;
    ex.run(&server, "restore_item", json!({})).await;
    ex.run(&server, "delete_item", json!({"path": "g/Moved.yml"}))
        .await;
    // A locked later sibling makes the delete report a warning; root ignores the lock.
    #[cfg(unix)]
    {
        let locked = server.root().join("g");
        let guard = common::ModeGuard::set(&locked, 0o555);
        let enforced = std::fs::write(locked.join("probe"), "").is_err();
        let out = ex
            .run(&server, "delete_item", json!({"path": "Fresh.yml"}))
            .await;
        guard.restore();
        if enforced {
            assert!(
                out["warnings"].as_array().is_some_and(|w| !w.is_empty()),
                "{out}"
            );
        }
    }

    // Environments.
    ex.run(&server, "create_environment", json!({"name": "tmp"}))
        .await;
    ex.run(&server, "duplicate_environment", json!({"name": "tmp"}))
        .await;
    ex.run(
        &server,
        "rename_environment",
        json!({"from": "tmp copy", "to": "tmp2"}),
    )
    .await;
    ex.run(
        &server,
        "update_environment",
        json!({"name": "tmp2", "set": [{"name": "a", "value": "b"}]}),
    )
    .await;
    ex.run(&server, "delete_environment", json!({"name": "tmp2"}))
        .await;

    let listed: BTreeSet<String> = ex.schemas.keys().cloned().collect();
    let missed: Vec<&String> = listed.difference(&ex.seen).collect();
    assert!(
        missed.is_empty(),
        "tools with an outputSchema this test never called: {missed:?}"
    );
    assert!(
        ex.failures.is_empty(),
        "results that break their schema:\n{}",
        ex.failures.join("\n")
    );
}
