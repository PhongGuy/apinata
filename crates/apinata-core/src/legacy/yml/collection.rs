//! Folder and collection-root documents: `parseBruCollection` with the migration's strip, then
//! `stringifyFolder` for `folder.yml` and, with the `bruno.json` transform, `stringifyCollection`
//! for `opencollection.yml`.

use super::auth::map_auth;
use super::item::{oauth2_additional_parameters, strip};
use super::rows::{self, HTTP_SCRIPT_KEYS};
use super::value::{
    ensure_string, get, is_non_empty_string, js_or, number, to_mapping, to_number, truthy,
};
use crate::js::js_trim;
use serde_json::{Map, Value, json};
use yaml_serde::Mapping;

/// JavaScript `x?.length` read as truthy: a non-empty string or array.
fn has_length(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        _ => false,
    }
}

/// Inserts a copy of `value` under `key` unless it is `undefined`.
fn put(out: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value {
        out.insert(key.into(), value.clone());
    }
}

/// `normalizeTags`: trimmed non-empty strings, first occurrence kept.
fn normalize_tags(tags: Option<&Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for tag in tags.and_then(Value::as_array).into_iter().flatten() {
        if let Some(tag) = tag.as_str().map(js_trim).filter(|t| !t.is_empty()) {
            let tag = Value::from(tag);
            if !out.contains(&tag) {
                out.push(tag);
            }
        }
    }
    out
}

/// `parseBruCollection` followed by the migration's strip of `.bru` from `bru.runRequest`; only
/// the keys the folder and collection writers read are kept.
fn parse_root(parsed: &Value) -> Value {
    let get_or = |path, default: Value| get(parsed, path).cloned().unwrap_or(default);
    let mut script = get_or("script", json!({}));
    if let Value::Object(script) = &mut script {
        for key in ["req", "res"] {
            if let Some(code) = script.get_mut(key) {
                strip(code);
            }
        }
    }
    let mut tests = get_or("tests", json!(""));
    strip(&mut tests);
    let mut auth = get_or("auth", json!({}));
    if let Some(params) = oauth2_additional_parameters(parsed)
        && let Some(Value::Object(oauth2)) = auth.get_mut("oauth2")
    {
        oauth2.insert("additionalParameters".into(), params);
    }
    let mut root = Map::new();
    root.insert(
        "request".into(),
        json!({
            "headers": get_or("headers", json!([])),
            "auth": auth,
            "script": script,
            "vars": get_or("vars", json!({})),
            "tests": tests,
        }),
    );
    root.insert("docs".into(), get_or("docs", json!("")));
    if let Some(meta) = parsed.get("meta").filter(|m| truthy(Some(m))) {
        let mut out = Map::new();
        put(&mut out, "name", get(meta, "name"));
        // `!isNaN(seq) ? Number(seq) : 1`; a non-finite seq is never written, so it is dropped.
        if let Some(seq) = get(meta, "seq") {
            let seq = to_number(Some(seq));
            let seq = if seq.is_nan() { 1.0 } else { seq };
            if seq.is_finite() {
                out.insert("seq".into(), number(seq));
            }
        }
        let tags = normalize_tags(get(meta, "tags"));
        if !tags.is_empty() {
            out.insert("tags".into(), Value::Array(tags));
        }
        root.insert("meta".into(), Value::Object(out));
    }
    Value::Object(root)
}

/// The `request` defaults block shared by folders and the collection; `auth_gate` is the
/// writer's own `hasRequestAuth`, which differs between the two.
fn request_defaults(root: &Value, auth_gate: fn(Option<&Value>) -> bool) -> Option<Value> {
    let request = get(root, "request");
    let field = |path| request.and_then(|r| get(r, path));
    let has_auth = auth_gate(field("auth.mode"));
    let has_scripts =
        truthy(field("script.req")) || truthy(field("script.res")) || truthy(field("tests"));
    let has_defaults = has_length(field("headers"))
        || has_length(field("vars.req"))
        || has_length(field("vars.res"))
        || has_scripts
        || has_auth;
    if !has_defaults {
        return None;
    }
    let mut out = Map::new();
    let mut put_owned = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            out.insert(key.into(), value);
        }
    };
    if has_length(field("headers")) {
        put_owned("headers", rows::http_headers(field("headers")));
    }
    if has_auth {
        put_owned("auth", map_auth(field("auth")));
    }
    if has_length(field("vars.req")) {
        put_owned("variables", rows::variables(field("vars")));
    }
    if has_length(field("vars.res")) {
        put_owned("actions", rows::actions(field("vars.res")));
    }
    if has_scripts {
        put_owned("scripts", rows::scripts(request, HTTP_SCRIPT_KEYS));
    }
    Some(Value::Object(out))
}

/// The `docs` block when the root's docs have non-blank text.
fn docs(root: &Value) -> Option<Value> {
    get(root, "docs")
        .filter(|d| is_non_empty_string(Some(d)))
        .map(|content| json!({"content": content, "type": "text/markdown"}))
}

/// `folder.yml` for a `collectionBruToJson` parse of `folder.bru`.
pub(crate) fn map_folder(parsed: &Value) -> Mapping {
    let root = parse_root(parsed);
    let mut info = Map::new();
    info.insert(
        "name".into(),
        js_or(get(&root, "meta.name"), json!("Untitled Folder")),
    );
    info.insert("type".into(), json!("folder"));
    put(&mut info, "seq", get(&root, "meta.seq"));
    let tags = normalize_tags(get(&root, "meta.tags"));
    if !tags.is_empty() {
        info.insert("tags".into(), Value::Array(tags));
    }
    let mut doc = Map::new();
    doc.insert("info".into(), Value::Object(info));
    // `mode !== 'none'`: a folder without an auth block still opens `request`.
    let folder_auth = |mode: Option<&Value>| mode != Some(&json!("none"));
    if let Some(request) = request_defaults(&root, folder_auth) {
        doc.insert("request".into(), request);
    }
    if let Some(docs) = docs(&root) {
        doc.insert("docs".into(), docs);
    }
    to_mapping(Value::Object(doc))
}

/// `transformProxyConfig`: the legacy `enabled` shape becomes `{inherit, config, disabled}`.
fn transform_proxy(proxy: &Value) -> Value {
    if !proxy.as_object().is_some_and(|p| p.contains_key("enabled")) {
        return proxy.clone();
    }
    let text = |path| js_or(get(proxy, path), json!(""));
    let mut auth = Map::new();
    auth.insert("username".into(), text("auth.username"));
    auth.insert("password".into(), text("auth.password"));
    if get(proxy, "auth.enabled") == Some(&Value::Bool(false)) {
        auth.insert("disabled".into(), json!(true));
    }
    let (inherit, disabled) = match get(proxy, "enabled") {
        Some(Value::Bool(true)) => (false, false),
        Some(Value::Bool(false)) => (false, true),
        _ => (true, false),
    };
    let mut out = Map::new();
    out.insert("inherit".into(), json!(inherit));
    out.insert(
        "config".into(),
        json!({
            "protocol": js_or(get(proxy, "protocol"), json!("http")),
            "hostname": text("hostname"),
            "port": js_or(get(proxy, "port"), Value::Null),
            "auth": auth,
            "bypassProxy": text("bypassProxy"),
        }),
    );
    if disabled {
        out.insert("disabled".into(), json!(true));
    }
    Value::Object(out)
}

/// The migration's `bruno.json` transform: the format `version` is dropped, `collectionVersion`
/// becomes `version`, and a proxy is moved to the current shape.
fn migrate_config(bruno_json: &Value) -> Value {
    let mut config = bruno_json.as_object().cloned().unwrap_or_default();
    config.remove("version");
    if let Some(version) = config.remove("collectionVersion")
        && truthy(Some(&version))
    {
        config.insert("version".into(), version);
    }
    if let Some(proxy) = config.get_mut("proxy").filter(|p| truthy(Some(p))) {
        *proxy = transform_proxy(proxy);
    }
    Value::Object(config)
}

/// A proxy in the current `{inherit, config: {..}}` shape, the only one the writer keeps.
fn current_proxy(proxy: Option<&Value>) -> Option<&Value> {
    proxy.filter(|p| {
        p.as_object().is_some_and(|p| p.contains_key("inherit"))
            && get(p, "config").is_some_and(|c| c.is_object() || c.is_array())
    })
}

/// The proxy block of `config`.
fn proxy_block(proxy: &Value) -> Value {
    let text = |path, fallback: &str| js_or(get(proxy, path), json!(fallback));
    let mut auth = Map::new();
    auth.insert("username".into(), text("config.auth.username", ""));
    auth.insert("password".into(), text("config.auth.password", ""));
    if get(proxy, "config.auth.disabled") == Some(&Value::Bool(true)) {
        auth.insert("disabled".into(), json!(true));
    }
    let mut out = Map::new();
    put(&mut out, "inherit", get(proxy, "inherit"));
    out.insert(
        "config".into(),
        json!({
            "protocol": text("config.protocol", "http"),
            "hostname": text("config.hostname", ""),
            "port": text("config.port", ""),
            "auth": auth,
            "bypassProxy": text("config.bypassProxy", ""),
        }),
    );
    if get(proxy, "disabled") == Some(&Value::Bool(true)) {
        out.insert("disabled".into(), json!(true));
    }
    Value::Object(out)
}

/// One client certificate; types other than `cert` and `pfx` are dropped.
fn certificate(cert: &Value) -> Option<Value> {
    let (kind, paths): (&str, &[(&str, &str)]) = match get(cert, "type").and_then(Value::as_str) {
        Some("cert") => (
            "pem",
            &[
                ("certificateFilePath", "certFilePath"),
                ("privateKeyFilePath", "keyFilePath"),
            ],
        ),
        Some("pfx") => ("pkcs12", &[("pkcs12FilePath", "pfxFilePath")]),
        _ => return None,
    };
    let mut out = Map::new();
    put(&mut out, "domain", get(cert, "domain"));
    out.insert("type".into(), json!(kind));
    for (to, from) in paths {
        put(&mut out, to, get(cert, from));
    }
    put(
        &mut out,
        "passphrase",
        get(cert, "passphrase").filter(|p| truthy(Some(p))),
    );
    if get(cert, "disabled") == Some(&Value::Bool(true)) {
        out.insert("disabled".into(), json!(true));
    }
    Some(Value::Object(out))
}

/// The `config` block, present when protobuf files, a current-shape proxy or certificates are set.
fn collection_config(config: &Value) -> Option<Value> {
    let proto_files = get(config, "protobuf.protoFiles");
    let import_paths = get(config, "protobuf.importPaths");
    let has_protobuf = has_length(proto_files) || has_length(import_paths);
    let proxy = current_proxy(get(config, "proxy"));
    let certs = get(config, "clientCertificates.certs");
    if !(has_protobuf || proxy.is_some() || has_length(certs)) {
        return None;
    }
    let mut out = Map::new();
    if has_protobuf {
        let mut protobuf = Map::new();
        if let Some(files) = proto_files
            .and_then(Value::as_array)
            .filter(|f| !f.is_empty())
        {
            let files = files.iter().map(|file| {
                let mut entry = Map::new();
                entry.insert("type".into(), json!("file"));
                put(&mut entry, "path", get(file, "path"));
                Value::Object(entry)
            });
            protobuf.insert("protoFiles".into(), files.collect());
        }
        if let Some(paths) = import_paths
            .and_then(Value::as_array)
            .filter(|p| !p.is_empty())
        {
            let paths = paths.iter().map(|import| {
                let mut entry = Map::new();
                put(&mut entry, "path", get(import, "path"));
                if get(import, "enabled") == Some(&Value::Bool(false)) {
                    entry.insert("disabled".into(), json!(true));
                }
                Value::Object(entry)
            });
            protobuf.insert("importPaths".into(), paths.collect());
        }
        out.insert("protobuf".into(), Value::Object(protobuf));
    }
    if let Some(proxy) = proxy {
        out.insert("proxy".into(), proxy_block(proxy));
    }
    if let Some(certs) = certs.and_then(Value::as_array).filter(|c| !c.is_empty()) {
        let certs = certs.iter().filter_map(certificate).collect();
        out.insert("clientCertificates".into(), Value::Array(certs));
    }
    Some(Value::Object(out))
}

/// `normalizeOpenApiSyncConfigs`: entries with a non-empty `sourceUrl`, defaults filled in.
fn openapi_entries(entries: Option<&Value>) -> Vec<Value> {
    let usable = |e: &&Value| {
        get(e, "sourceUrl")
            .and_then(Value::as_str)
            .is_some_and(|u| !u.is_empty())
    };
    let entries = entries.and_then(Value::as_array).into_iter().flatten();
    entries
        .filter(usable)
        .map(|entry| {
            let field = |key| get(entry, key);
            let mut out = Map::new();
            put(&mut out, "sourceUrl", field("sourceUrl"));
            let group_by = field("groupBy").filter(|g| *g == "tags" || *g == "path");
            put(&mut out, "groupBy", group_by);
            for key in ["lastSyncDate", "specHash"] {
                put(&mut out, key, field(key).filter(|v| truthy(Some(v))));
            }
            out.insert(
                "autoCheck".into(),
                json!(field("autoCheck") != Some(&Value::Bool(false))),
            );
            out.insert(
                "autoCheckInterval".into(),
                js_or(field("autoCheckInterval"), json!(5)),
            );
            Value::Object(out)
        })
        .collect()
}

/// The `extensions` block: ignore list, presets, script settings and OpenAPI sync entries.
fn extensions(config: &Value) -> Value {
    let mut bruno = Map::new();
    if let Some(ignore) = get(config, "ignore").filter(|i| has_length(Some(i)) && i.is_array()) {
        bruno.insert("ignore".into(), ignore.clone());
    }
    let preset = |key| get(config, &format!("presets.{key}")).filter(|v| has_length(Some(v)));
    let (kind, url, environment) = (
        preset("requestType"),
        preset("requestUrl"),
        preset("defaultEnvironment"),
    );
    if kind.is_some() || url.is_some() || environment.is_some() {
        let mut request = Map::new();
        put(&mut request, "type", kind);
        put(&mut request, "url", url);
        let mut presets = Map::new();
        if !request.is_empty() {
            presets.insert("request".into(), Value::Object(request));
        }
        put(&mut presets, "defaultEnvironment", environment);
        bruno.insert("presets".into(), Value::Object(presets));
    }
    let mut scripts = Map::new();
    let roots = get(config, "scripts.additionalContextRoots").filter(|r| has_length(Some(r)));
    put(&mut scripts, "additionalContextRoots", roots);
    let flow = get(config, "scripts.flow").filter(|f| *f == "sandwich" || *f == "sequential");
    put(&mut scripts, "flow", flow);
    if !scripts.is_empty() {
        bruno.insert("scripts".into(), Value::Object(scripts));
    }
    let openapi = openapi_entries(get(config, "openapi"));
    if !openapi.is_empty() {
        bruno.insert("openapi".into(), Value::Array(openapi));
    }
    let mut out = Map::new();
    if !bruno.is_empty() {
        out.insert("bruno".into(), Value::Object(bruno));
    }
    Value::Object(out)
}

/// The reference calls `.map` or `.forEach` on these config fields once they are non-empty, which
/// throws for a string; returns the first such field's dotted name.
fn string_where_array_expected(config: &Value) -> Option<&'static str> {
    [
        "protobuf.protoFiles",
        "protobuf.importPaths",
        "clientCertificates.certs",
        "ignore",
    ]
    .into_iter()
    .find(|path| matches!(get(config, path), Some(Value::String(s)) if !s.is_empty()))
}

/// `opencollection.yml` from `bruno.json` as parsed and the root `collection.bru` parse, if any.
/// Errors with a message naming the config field the reference would throw on.
pub(crate) fn map_collection(
    collection_bru: Option<&Value>,
    bruno_json: &Value,
) -> Result<Mapping, String> {
    let root = collection_bru.map_or_else(|| json!({}), parse_root);
    let config = migrate_config(bruno_json);
    if let Some(field) = string_where_array_expected(&config) {
        return Err(format!("`{field}` must be an array, not a string"));
    }
    let mut info = Map::new();
    info.insert(
        "name".into(),
        js_or(get(&config, "name"), json!("Untitled Collection")),
    );
    match get(&config, "version") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s.is_empty() => {}
        version => {
            info.insert("version".into(), json!(ensure_string(version)));
        }
    }
    let mut doc = Map::new();
    doc.insert("opencollection".into(), json!("1.0.0"));
    doc.insert("info".into(), Value::Object(info));
    if let Some(block) = collection_config(&config) {
        doc.insert("config".into(), block);
    }
    // `mode && mode !== 'none'`: unlike a folder, a collection without auth adds no defaults.
    let collection_auth = |mode: Option<&Value>| truthy(mode) && mode != Some(&json!("none"));
    if let Some(request) = request_defaults(&root, collection_auth) {
        doc.insert("request".into(), request);
    }
    if let Some(docs) = docs(&root) {
        doc.insert("docs".into(), docs);
    }
    doc.insert("bundled".into(), json!(false));
    doc.insert("extensions".into(), extensions(&config));
    Ok(to_mapping(Value::Object(doc)))
}
