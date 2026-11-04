//! The helpers the MCP server's redaction leans on: a `Debug` that hides a secret in the URL
//! path, the sensitive values assertions interpolate, and whether an undefined or cyclic
//! variable's name was written inside a sensitive value.

use crate::common;

use apinata_core::{
    Collection, ContextOptions, ItemPath, MemoryStore, OAuth2Target, SecretStore, VarContext,
    assertion_sensitive_values, oauth2_name_hidden,
};
use common::{at, write_tree};
use std::sync::Arc;
use tempfile::TempDir;

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

/// `dev` holds the secrets `token`, `broken` (whose value writes `{{missing}}`) and `tail`
/// (whose value closes a placeholder), and plain `plain`, whose value writes `{{onlyplain}}`.
const DEV: &str = "name: dev\nvariables:\n  - name: token\n    secret: true\n  - name: broken\n    secret: true\n  - name: tail\n    secret: true\n  - name: plain\n    value: \"{{onlyplain}}\"\n";

fn request(url: &str, assertions: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  assertions:\n{assertions}"
    )
}

fn open(req: &str, dotenv: Option<&str>) -> (TempDir, Collection, VarContext) {
    open_with_env(req, dotenv, &[("HOME_DIR", "/home/{{other}}")])
}

/// Opens the collection with `env` standing in for the host's process environment.
fn open_with_env(
    req: &str,
    dotenv: Option<&str>,
    env: &[(&str, &str)],
) -> (TempDir, Collection, VarContext) {
    open_tree(ROOT, DEV, req, dotenv, env)
}

/// Opens the collection from the given root and `dev` environment files.
fn open_tree(
    root: &str,
    dev: &str,
    req: &str,
    dotenv: Option<&str>,
    env: &[(&str, &str)],
) -> (TempDir, Collection, VarContext) {
    let mut files = vec![
        ("opencollection.yml", root),
        ("environments/dev.yml", dev),
        ("r.yml", req),
    ];
    if let Some(text) = dotenv {
        files.push((".env", text));
    }
    open_files(&files, env)
}

/// Opens `files` with the `dev` secrets stored and `env` as the host's environment.
fn open_files(files: &[(&str, &str)], env: &[(&str, &str)]) -> (TempDir, Collection, VarContext) {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    let store = MemoryStore::new();
    store
        .set(&c.secret_key("dev", "token"), "s3cr3t-Token")
        .unwrap();
    store
        .set(&c.secret_key("dev", "broken"), "ab{{missing}}cd")
        .unwrap();
    store.set(&c.secret_key("dev", "tail"), "xy}}rest").unwrap();
    let options = ContextOptions::new()
        .env("dev")
        .secrets(Arc::new(store))
        .process_env(env.iter().copied());
    let ctx = VarContext::new(&c, options).expect("context");
    (dir, c, ctx)
}

fn file(c: &Collection) -> &apinata_core::RequestFile {
    c.request(&ItemPath::new("r.yml").expect("item path"))
        .expect("request")
}

#[test]
fn debug_hides_a_secret_in_the_url_path() {
    let (_d, c, ctx) = open(
        &request("https://x.test/users/{{token}}/items", "    []\n"),
        None,
    );
    let resolved = at(&c, &ctx, "r.yml").expect("resolves");
    let shown = format!("{resolved:?}");
    assert!(!shown.contains("s3cr3t-Token"), "{shown}");
}

#[test]
fn debug_still_shows_a_url_that_holds_no_sensitive_value() {
    let (_d, c, ctx) = open(&request("https://x.test/users/42", "    []\n"), None);
    let resolved = at(&c, &ctx, "r.yml").expect("resolves");
    assert!(format!("{resolved:?}").contains("https://x.test/users/42"));
}

#[test]
fn assertion_values_report_secrets_used_only_on_a_right_hand_side() {
    let assertions = "    - expression: res.body.token\n      operator: eq\n      value: \"{{token}}\"\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let (_d, c, ctx) = open(&request("https://x.test/", assertions), None);
    assert_eq!(assertion_sensitive_values(file(&c), &ctx), ["s3cr3t-Token"]);
}

#[test]
fn assertion_values_skip_disabled_assertions_and_plain_variables() {
    let assertions = "    - expression: res.body.token\n      operator: eq\n      value: \"{{token}}\"\n      disabled: true\n    - expression: res.body.a\n      operator: eq\n      value: \"{{plain}}\"\n";
    let (_d, c, ctx) = open(&request("https://x.test/", assertions), None);
    assert!(assertion_sensitive_values(file(&c), &ctx).is_empty());
}

#[test]
fn assertion_values_include_dotenv_reads() {
    let assertions = "    - expression: res.body.k\n      operator: eq\n      value: \"{{process.env.API_KEY}}\"\n";
    let (_d, c, ctx) = open(
        &request("https://x.test/", assertions),
        Some("API_KEY=dotenv-Value-9\n"),
    );
    assert_eq!(
        assertion_sensitive_values(file(&c), &ctx),
        ["dotenv-Value-9"]
    );
}

#[test]
fn a_name_written_inside_a_secret_or_process_value_is_reported() {
    let (_d, c, ctx) = open(&request("https://x.test/", "    []\n"), None);
    let req = file(&c);
    assert!(ctx.name_inside_sensitive_value(req, "missing"));
    assert!(ctx.name_inside_sensitive_value(req, "other"));
}

#[test]
fn a_name_written_in_authored_plain_text_is_not() {
    let (_d, c, ctx) = open(
        &request("https://x.test/{{absent}}/{{token}}", "    []\n"),
        None,
    );
    let req = file(&c);
    assert!(!ctx.name_inside_sensitive_value(req, "onlyplain"));
    assert!(!ctx.name_inside_sensitive_value(req, "absent"));
    assert!(!ctx.name_inside_sensitive_value(req, "token"));
}

#[test]
fn a_name_written_nowhere_is_reported() {
    let (_d, c, ctx) = open(&request("https://x.test/", "    []\n"), None);
    assert!(ctx.name_inside_sensitive_value(file(&c), "nowhere"));
}

/// A request whose runtime variable `open` is `{{`, so a secret can join into a placeholder.
fn joining(url: &str) -> String {
    format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\nruntime:\n  variables:\n    - name: open\n      value: \"{{{{\"\n"
    )
}

#[test]
fn a_secret_joined_into_a_placeholder_name_is_reported() {
    let (_d, c, ctx) = open(&joining("https://x.test/{{open}}{{token}}}}"), None);
    let err = at(&c, &ctx, "r.yml").expect_err("undefined");
    assert!(format!("{err:?}").contains("s3cr3t-Token"), "{err:?}");
    assert!(ctx.name_inside_sensitive_value(file(&c), "s3cr3t-Token"));
}

#[test]
fn a_secret_fragment_joined_into_a_placeholder_name_is_reported() {
    let (_d, c, ctx) = open(&joining("https://x.test/{{open}}pre{{tail}}"), None);
    let err = at(&c, &ctx, "r.yml").expect_err("undefined");
    assert!(format!("{err:?}").contains("prexy"), "{err:?}");
    assert!(ctx.name_inside_sensitive_value(file(&c), "prexy"));
}

#[test]
fn a_host_env_value_inside_an_authored_name_does_not_withhold_it() {
    let (_d, c, ctx) = open_with_env(
        &request("https://x.test/{{nope}}", "    []\n"),
        None,
        &[("FLAG", "no")],
    );
    assert!(!ctx.name_inside_sensitive_value(file(&c), "nope"));
}

#[test]
fn an_authored_name_holding_an_unread_secret_is_named() {
    let (_d, c, ctx) = open(
        &request("https://x.test/{{a-s3cr3t-Token}}", "    []\n"),
        None,
    );
    assert!(!ctx.name_inside_sensitive_value(file(&c), "a-s3cr3t-Token"));
}

#[test]
fn an_authored_name_holding_a_read_secret_is_withheld() {
    let (_d, c, ctx) = open(
        &request("https://x.test/{{a-s3cr3t-Token}}/{{token}}", "    []\n"),
        None,
    );
    assert!(ctx.name_inside_sensitive_value(file(&c), "a-s3cr3t-Token"));
}

#[test]
fn a_read_process_value_inside_an_authored_name_withholds_it() {
    let (_d, c, ctx) = open_with_env(
        &request(
            "https://x.test/{{process.env.FLAG}}/{{xnope-ishx}}",
            "    []\n",
        ),
        None,
        &[("FLAG", "nope-ish")],
    );
    assert!(ctx.name_inside_sensitive_value(file(&c), "xnope-ishx"));
}

#[test]
fn a_secret_read_by_an_assertion_withholds_an_authored_name_holding_it() {
    let assertions =
        "    - expression: res.body.t\n      operator: eq\n      value: \"{{token}}\"\n";
    let (_d, c, ctx) = open(
        &request("https://x.test/{{a-s3cr3t-Token}}", assertions),
        None,
    );
    assert!(ctx.name_inside_sensitive_value(file(&c), "a-s3cr3t-Token"));
}

/// `dev` plus plain `open` (`{{`) and `a`, whose value joins `token` into a placeholder.
fn joining_dev() -> String {
    format!(
        "{DEV}  - name: open\n    value: \"{{{{\"\n  - name: a\n    value: \"{{{{open}}}}{{{{token}}}}}}}}\"\n"
    )
}

/// The writer's authored guess at the secret, kept in the request's docs.
const GUESS: &str = "docs: \"{{s3cr3t-Token}}\"\n";

#[test]
fn a_secret_joined_through_the_proxy_config_withholds_an_authored_guess() {
    let root = format!(
        "{ROOT}config:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: \"{{{{a}}}}\"\n      port: \"8080\"\n"
    );
    let req = format!("{}{GUESS}", request("https://x.test/", "    []\n"));
    let (_d, c, ctx) = open_tree(&root, &joining_dev(), &req, None, &[]);
    let err = at(&c, &ctx, "r.yml").expect_err("undefined");
    assert!(format!("{err:?}").contains("s3cr3t-Token"), "{err:?}");
    assert!(ctx.name_inside_sensitive_value(file(&c), "s3cr3t-Token"));
}

#[test]
fn a_secret_joined_in_a_split_assertion_item_withholds_an_authored_guess() {
    let assertions =
        "    - expression: res.status\n      operator: in\n      value: \"{{z,{{a}}\"\n";
    let req = format!("{}{GUESS}", request("https://x.test/", assertions));
    let (_d, c, ctx) = open_tree(ROOT, &joining_dev(), &req, None, &[]);
    assert!(ctx.name_inside_sensitive_value(file(&c), "s3cr3t-Token"));
}

/// An oauth2 auth whose token URL reads the secret `broken` and whose client id is authored
/// as `{{plainname}}`.
const SECRET_URL_AUTH: &str = "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"http://127.0.0.1:9/{{broken}}\"\n    credentials:\n      clientId: \"{{plainname}}\"\n    tokenConfig:\n      id: main\n";

#[test]
fn a_folder_target_hides_a_name_written_inside_a_secret() {
    let folder = format!("info:\n  name: f\n  type: folder\nrequest:\n{SECRET_URL_AUTH}");
    let files = [
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("f/folder.yml", folder.as_str()),
    ];
    let (_d, _c, ctx) = open_files(&files, &[]);
    let f = ItemPath::new("f").expect("item path");
    assert!(oauth2_name_hidden(
        OAuth2Target::Folder(&f),
        &ctx,
        "missing"
    ));
    assert!(!oauth2_name_hidden(
        OAuth2Target::Folder(&f),
        &ctx,
        "plainname"
    ));
}

#[test]
fn the_collection_target_hides_a_name_written_inside_a_secret() {
    let root = format!("{ROOT}request:\n{SECRET_URL_AUTH}");
    let files = [
        ("opencollection.yml", root.as_str()),
        ("environments/dev.yml", DEV),
    ];
    let (_d, _c, ctx) = open_files(&files, &[]);
    assert!(oauth2_name_hidden(
        OAuth2Target::Collection,
        &ctx,
        "missing"
    ));
    assert!(!oauth2_name_hidden(
        OAuth2Target::Collection,
        &ctx,
        "plainname"
    ));
}
