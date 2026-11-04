//! `{{$name}}` dynamic variables through `resolve`.

use crate::common;

use apinata_core::{
    Body, Collection, ContextOptions, MemoryStore, ResolveError, SecretStore, VarContext,
    dynamic_variables, resolve,
};
use common::{request, write_tree};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";

fn at() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(1_791_705_600_123)
}
fn req(url: &str, rest: &str) -> String {
    format!("info:\n  name: r\n  type: http\nhttp:\n  method: POST\n  url: \"{url}\"\n{rest}")
}
fn open(files: &[(&str, &str)]) -> (tempfile::TempDir, Collection) {
    let dir = write_tree(files);
    let c = Collection::open(dir.path()).expect("opens");
    assert!(c.load_errors().is_empty(), "{:?}", c.load_errors());
    (dir, c)
}
fn seeded(c: &Collection) -> VarContext {
    VarContext::new(c, ContextOptions::new().dynamic_values(9, at())).expect("context")
}
fn url_of(c: &Collection, ctx: &VarContext) -> String {
    resolve(request(c, "r.yml"), ctx)
        .expect("resolves")
        .url()
        .to_owned()
}

#[test]
fn each_occurrence_is_fresh_and_a_seed_repeats() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &req("http://x.test/{{$guid}}/{{$guid}}?t={{$timestamp}}", ""),
        ),
    ]);
    let url = url_of(&c, &seeded(&c));
    assert_eq!(url, url_of(&c, &seeded(&c)));
    let path: Vec<&str> = url["http://x.test/".len()..].split(['/', '?']).collect();
    assert_eq!(path[0].len(), 36);
    assert_ne!(path[0], path[1]);
    assert_eq!(path[2], "t=1791705600");
}

#[test]
fn every_send_gets_new_values_without_a_seed() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req("http://x.test/{{$guid}}", "")),
    ]);
    let ctx = VarContext::new(&c, ContextOptions::new()).expect("context");
    assert_ne!(url_of(&c, &ctx), url_of(&c, &ctx));
}

#[test]
fn values_inside_variables_headers_and_bodies_resolve() {
    let rest = "  headers:\n    - name: X-Id\n      value: \"{{id}}-{{id}}\"\n  body:\n    type: json\n    data: '{\"n\": {{$randomInt}}, \"at\": \"{{$isoTimestamp}}\"}'\nruntime:\n  variables:\n    - name: id\n      value: \"{{$randomUUID}}\"\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req("http://x.test/", rest)),
    ]);
    let r = resolve(request(&c, "r.yml"), &seeded(&c)).expect("resolves");
    let header = &r
        .headers()
        .iter()
        .find(|(n, _)| n == "X-Id")
        .expect("header")
        .1;
    let (a, b) = header.split_at(36);
    assert_eq!(&b[1..], a, "one variable keeps one value within a field");
    let Body::Text(body) = r.body() else {
        panic!("text body")
    };
    assert!(
        body.contains("\"at\": \"2026-10-11T08:00:00.123Z\""),
        "{body}"
    );
    assert!(!body.contains("{{"), "{body}");
}

#[test]
fn unknown_names_stay_undefined_unless_a_variable_has_them() {
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req("http://x.test/{{$randomNoSuchThing}}", "")),
    ]);
    let err = resolve(request(&c, "r.yml"), &seeded(&c)).unwrap_err();
    assert_eq!(
        err,
        ResolveError::UndefinedVariable {
            name: "$randomNoSuchThing".into(),
            hint: None
        }
    );
    let vars = "runtime:\n  variables:\n    - name: $mine\n      value: m\n    - name: $guid\n      value: shadow\n";
    let (_d2, c2) = open(&[
        ("opencollection.yml", ROOT),
        ("r.yml", &req("http://x.test/{{$mine}}/{{$guid}}", vars)),
    ]);
    let url = url_of(&c2, &seeded(&c2));
    assert!(url.starts_with("http://x.test/m/"), "{url}");
    assert!(
        !url.ends_with("shadow"),
        "a known name is never shadowed: {url}"
    );
}

#[test]
fn a_url_escape_survives_next_to_a_dynamic_value() {
    // Delimiter escapes only matter with encodeUrl on; a dropped mark would turn %26 into %2526.
    // The params table holds the `&` the escape stands for, as `set_params` writes it.
    let settings = "  params:\n    - name: a\n      value: \"1&2\"\n      type: query\n    - name: id\n      value: \"{{$guid}}\"\n      type: query\nsettings:\n  encodeUrl: true\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &req("http://x.test/?a=1%262&id={{$guid}}", settings),
        ),
    ]);
    let url = url_of(&c, &seeded(&c));
    assert!(url.starts_with("http://x.test/?a=1%262&id="), "{url}");
    assert!(!url.contains("%2526"), "{url}");
}

#[test]
fn resolving_never_rewrites_the_request_file() {
    let text = req("http://x.test/{{$guid}}", "");
    let (dir, c) = open(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let _ = url_of(&c, &seeded(&c));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("r.yml")).unwrap(),
        text
    );
    assert!(dynamic_variables().contains(&"randomInt"));
}

#[test]
fn a_secret_holding_a_dynamic_value_redacts_the_generated_value() {
    let rest = "  headers:\n    - name: Authorization\n      value: \"{{token}}\"\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "environments/dev.yml",
            "name: dev\nvariables:\n  - name: token\n    secret: true\n",
        ),
        ("r.yml", &req("http://x.test/", rest)),
    ]);
    let store = MemoryStore::new();
    store
        .set(&c.secret_key("dev", "token"), "{{$guid}}")
        .expect("set");
    let options = ContextOptions::new()
        .env("dev")
        .secrets(Arc::new(store))
        .dynamic_values(9, at());
    let ctx = VarContext::new(&c, options).expect("context");
    let r = resolve(request(&c, "r.yml"), &ctx).expect("resolves");
    let header = &r
        .headers()
        .iter()
        .find(|(n, _)| n == "Authorization")
        .expect("header")
        .1;
    assert_eq!(header.len(), 36, "{header}");
    assert!(
        r.sensitive_values().iter().any(|v| v == header),
        "{:?}",
        r.sensitive_values()
    );
}

#[test]
fn the_name_and_text_generators_resolve_through_requests() {
    let rest =
        "  headers:\n    - name: X-Who\n      value: \"{{$randomFullName}} <{{$randomEmail}}>\"\n";
    let (_d, c) = open(&[
        ("opencollection.yml", ROOT),
        (
            "r.yml",
            &req(
                "http://x.test/{{$randomLoremWord}}?c={{$randomColor}}",
                rest,
            ),
        ),
    ]);
    let r = resolve(request(&c, "r.yml"), &seeded(&c)).expect("resolves");
    assert!(!r.url().contains("{{"), "{}", r.url());
    let who = &r
        .headers()
        .iter()
        .find(|(n, _)| n == "X-Who")
        .expect("header")
        .1;
    assert!(who.contains(" <") && who.contains("@example."), "{who}");
    let again = resolve(request(&c, "r.yml"), &seeded(&c)).expect("resolves");
    assert_eq!(r.url(), again.url());
    for name in ["randomFirstName", "randomPhoneNumber", "randomCountry"] {
        assert!(dynamic_variables().contains(&name), "{name}");
    }
}
