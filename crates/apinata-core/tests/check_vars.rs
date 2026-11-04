use crate::common;
use apinata_core::{
    CheckOptions, Collection, FindingKind, HintShell, MemoryStore, SecretStore, Severity,
    check_collection,
};
use common::{ROOT, check_tree};
use std::net::TcpListener;
use std::sync::Arc;

fn req(name: &str, url: &str, extra: &str) -> String {
    format!(
        "info:\n  name: {name}\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"{url}\"\n{extra}"
    )
}
const DEV: &str = "name: dev\nvariables:\n  - name: host\n    value: example.test\n  - name: token\n    secret: true\n";
const PROD: &str = "name: prod\nvariables:\n  - name: token\n    secret: true\n";

fn one(files: &[(&str, &str)]) -> Vec<(String, Severity, FindingKind, Option<String>)> {
    let (_d, r) = check_tree(files, &CheckOptions::default());
    r.findings()
        .iter()
        .map(|f| {
            (
                f.message().to_owned(),
                f.severity(),
                f.kind(),
                f.environment().map(str::to_owned),
            )
        })
        .collect()
}

#[test]
fn an_undefined_variable_is_an_error_naming_the_item() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "http://{{nope}}/x", "")),
        ],
        &CheckOptions::default(),
    );
    assert_eq!(r.findings().len(), 1);
    let f = &r.findings()[0];
    assert_eq!(
        (f.item(), f.kind(), f.severity()),
        ("a.yml", FindingKind::UndefinedVariable, Severity::Error)
    );
    assert!(f.message().contains("nope"));
}

#[test]
fn every_undefined_name_of_a_request_is_reported() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "http://{{one}}/{{two}}?q={{three}}", "")),
        ],
        &CheckOptions::default(),
    );
    let names: Vec<&str> = r.findings().iter().map(|f| f.message()).collect();
    assert_eq!(names.len(), 3, "{names:?}");
    for n in ["one", "two", "three"] {
        assert!(names.iter().any(|m| m.contains(n)), "{n} in {names:?}");
    }
}

#[test]
fn a_declared_secret_without_a_value_is_not_undefined() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", DEV),
            ("a.yml", &req("a", "http://{{host}}/{{token}}", "")),
        ],
        &CheckOptions::default(),
    );
    // The secret is declared, so the request is not undefined; the only finding is the
    // warning that the environment has no value for it.
    assert_eq!(r.errors(), 0, "{:?}", r.findings());
    assert_eq!(r.findings().len(), 1, "{:?}", r.findings());
    assert_eq!(r.findings()[0].severity(), Severity::Warning);
}

#[test]
fn a_finding_seen_in_one_environment_names_it_and_one_seen_in_all_does_not() {
    let found = one(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", PROD),
        ("a.yml", &req("a", "http://{{host}}/{{gone}}", "")),
    ]);
    let host: Vec<_> = found.iter().filter(|f| f.0.contains("host")).collect();
    let gone: Vec<_> = found.iter().filter(|f| f.0.contains("gone")).collect();
    assert_eq!(host.len(), 1);
    assert_eq!(host[0].3.as_deref(), Some("prod"));
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].3, None);
}

#[test]
fn env_var_and_global_env_var_count_as_defined() {
    let files = [
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", "http://{{h}}/{{g}}", "")),
    ];
    let (d, bare) = check_tree(&files, &CheckOptions::default());
    assert_eq!(bare.errors(), 2);
    let r = check_collection(
        d.path(),
        &CheckOptions::default()
            .env_var("h", "x")
            .global_env_var("g", "y"),
    )
    .unwrap();
    assert!(r.findings().is_empty(), "{:?}", r.findings());
}

#[test]
fn a_resolve_error_with_no_placeholder_is_an_invalid_request_with_fixed_text() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "http://exa mple.test", "")),
        ],
        &CheckOptions::default(),
    );
    let f = &r.findings()[0];
    assert_eq!(
        (f.kind(), f.severity()),
        (FindingKind::InvalidRequest, Severity::Error)
    );
    assert!(
        !f.message().contains("exa mple"),
        "the URL text must not be copied: {}",
        f.message()
    );
}

#[test]
fn no_secret_value_appears_anywhere_in_the_report() {
    const CANARY_DOTENV: &str = "canary-dotenv-7f3a";
    const CANARY_FLAG: &str = "canary-flag-91c2";
    let dir = common::write_tree(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        (
            ".env",
            &format!("token={CANARY_DOTENV}\nother={CANARY_DOTENV}\n"),
        ),
        (
            "a.yml",
            &req(
                "a",
                "{{base}} bad {{token}} {{process.env.other}} {{missing}}",
                "",
            ),
        ),
    ]);
    let opts = CheckOptions::default().env_var("base", CANARY_FLAG);
    let report = check_collection(dir.path(), &opts).unwrap();
    let json = serde_json::to_string(&report).unwrap() + &format!("{:?}", report);
    for canary in [CANARY_DOTENV, CANARY_FLAG] {
        assert!(!json.contains(canary), "{canary} leaked: {json}");
    }
}

#[test]
fn an_unreadable_dotenv_is_an_unparsable_item_with_fixed_text() {
    let dir = common::write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", "http://x/", "")),
    ]);
    std::fs::write(dir.path().join(".env"), [0xff, 0xfe, 0x00, b'=']).unwrap();
    let r = check_collection(dir.path(), &CheckOptions::default()).unwrap();
    let f = r
        .findings()
        .iter()
        .find(|f| f.item() == ".env")
        .expect("a finding on .env");
    assert_eq!(
        (f.kind(), f.severity()),
        (FindingKind::UnparsableItem, Severity::Error)
    );
}

#[test]
fn debug_output_of_check_options_redacts_variable_values() {
    let opts = CheckOptions::default()
        .env("dev")
        .env_var("api_key", "canary-opt-4d1e")
        .global_env_var("g_key", "canary-glob-8b20");
    let text = format!("{opts:?}") + &format!("{opts:#?}");
    assert!(!text.contains("canary-opt-4d1e"), "{text}");
    assert!(!text.contains("canary-glob-8b20"), "{text}");
    // Names stay visible so the output is still useful.
    assert!(text.contains("api_key") && text.contains("g_key"), "{text}");
}

#[test]
fn a_bad_url_built_from_an_env_var_never_copies_the_value() {
    const CANARY: &str = "canary-url-52ab";
    let dir = common::write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", "{{base}} bad", "")),
    ]);
    let opts = CheckOptions::default().env_var("base", CANARY);
    let report = check_collection(dir.path(), &opts).unwrap();
    let f = report
        .findings()
        .iter()
        .find(|f| f.kind() == FindingKind::InvalidRequest)
        .expect("an invalid request finding");
    assert_eq!(f.item(), "a.yml");
    let text = serde_json::to_string(&report).unwrap() + &format!("{report:?}");
    assert!(!text.contains(CANARY), "{text}");
}

#[test]
fn a_dotenv_failure_never_copies_file_content() {
    const CANARY: &str = "canary-dotenv-bytes-c3d9";
    let dir = common::write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", "http://x/", "")),
    ]);
    let mut bytes = format!("{CANARY}=").into_bytes();
    bytes.extend([0xff, 0xfe]);
    std::fs::write(dir.path().join(".env"), bytes).unwrap();
    let report = check_collection(dir.path(), &CheckOptions::default()).unwrap();
    let f = report
        .findings()
        .iter()
        .find(|f| f.item() == ".env")
        .expect("a finding on .env");
    assert_eq!(f.message(), "the .env file cannot be read");
    let text = serde_json::to_string(&report).unwrap() + &format!("{report:?}");
    assert!(!text.contains(CANARY), "{text}");
}

#[test]
fn a_name_that_only_a_variable_value_introduces_is_not_a_finding() {
    let dir = common::write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", "http://{{v}}/", "")),
    ]);
    let opts = CheckOptions::default().env_var("v", "{{leak}}");
    let report = check_collection(dir.path(), &opts).unwrap();
    assert!(report.findings().is_empty(), "{:?}", report.findings());
    assert_eq!(report.unresolved(), ["a.yml"]);
}

#[test]
fn hitting_the_round_cap_marks_the_request_unresolved() {
    let url: String = (0..60).map(|i| format!("{{{{n{i}}}}}")).collect();
    let (_d, report) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", &format!("http://{url}/"), "")),
        ],
        &CheckOptions::default(),
    );
    assert_eq!(report.unresolved(), ["a.yml"]);
    assert!(report.errors() <= 50, "{}", report.errors());
}

fn post_var(name: &str) -> String {
    format!(
        "runtime:\n  actions:\n    - type: set-variable\n      phase: after-response\n      selector:\n        expression: res.body.id\n      variable:\n        name: {name}\n"
    )
}
fn script(code: &str) -> String {
    script_of("after-response", code)
}
fn script_of(kind: &str, code: &str) -> String {
    let body: String = code.lines().map(|l| format!("        {l}\n")).collect();
    format!("runtime:\n  scripts:\n    - type: {kind}\n      code: |-\n{body}")
}

#[test]
fn a_name_set_at_run_time_is_a_warning() {
    for setter in [
        post_var("later"),
        script("bru.setVar('later', 1)"),
        script("bru.setEnvVar('later', 1)"),
    ] {
        let found = one(&[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "http://x/{{later}}", &setter)),
        ]);
        assert_eq!(found.len(), 1, "{setter}: {found:?}");
        assert_eq!(found[0].1, Severity::Warning, "{setter}");
        assert_eq!(
            found[0].0,
            "variable `later` is not defined here; it may be set at run time"
        );
    }
}

#[test]
fn a_dynamic_setter_makes_every_undefined_name_a_warning_and_a_plain_name_stays_an_error() {
    let dynamic = script("bru.setVar(name, 1)");
    let found = one(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &req("a", "http://x/{{any}}", &dynamic)),
    ]);
    assert_eq!(found[0].1, Severity::Warning);
    let found = one(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req("a", "http://x/{{other}}", &script("bru.setVar('later', 1)")),
        ),
    ]);
    assert_eq!(found[0].1, Severity::Error);
}

#[test]
fn process_env_names_are_warnings_never_errors() {
    let found = one(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req("a", "http://x/{{process.env.HOME_NOT_SET_I344}}", ""),
        ),
    ]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1, Severity::Warning);
    assert_eq!(
        found[0].0,
        "`process.env.HOME_NOT_SET_I344` depends on the process environment"
    );
}

#[test]
fn an_oauth2_request_without_a_token_is_listed_as_unresolved_not_as_a_finding() {
    let auth = "  auth:\n    type: oauth2\n    flow: client_credentials\n    accessTokenUrl: \"http://x/token\"\n    credentials:\n      clientId: app\n    scope: read\n    tokenConfig:\n      id: main\n";
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("o.yml", &req("o", "http://x/o", auth)),
        ],
        &CheckOptions::default(),
    );
    assert!(r.findings().is_empty(), "{:?}", r.findings());
    assert_eq!(r.unresolved(), ["o.yml"]);
}

#[test]
fn a_prompt_variable_is_answered_with_a_placeholder_and_the_rest_still_checked() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "http://x/{{?pin}}/{{gone}}", "")),
        ],
        &CheckOptions::default(),
    );
    assert_eq!(r.findings().len(), 1, "{:?}", r.findings());
    assert_eq!(r.findings()[0].kind(), FindingKind::UndefinedVariable);
    assert!(r.findings()[0].message().contains("gone"));
}

#[test]
fn a_failure_after_a_placeholder_is_unresolved_not_a_finding() {
    // The placeholder is what makes the URL unparsable; the author did not break it.
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "{{scheme}}://x y", "")),
        ],
        &CheckOptions::default(),
    );
    assert!(
        r.findings()
            .iter()
            .all(|f| f.kind() == FindingKind::UndefinedVariable),
        "{:?}",
        r.findings()
    );
    assert_eq!(r.unresolved(), ["a.yml"]);
}

#[test]
fn checking_opens_no_connection_to_a_request_url() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/ping/{{{{v}}}}", listener.local_addr().unwrap());
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &req("a", &url, ""))],
        &CheckOptions::default().env_var("v", "1"),
    );
    assert!(r.findings().is_empty(), "{:?}", r.findings());
    let accepted = listener.accept();
    assert!(
        matches!(accepted, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "a connection arrived"
    );
}

#[test]
fn setter_names_are_read_from_literal_first_arguments_only() {
    // A string literal in any of the three quote styles names the variable.
    for call in [
        "bru.setVar('later', 1)",
        "bru.setVar(\"later\", 1)",
        "bru.setGlobalEnvVar(`later`, 1)",
    ] {
        let found = one(&[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "http://x/{{later}}", &script(call))),
        ]);
        assert_eq!((found.len(), found[0].1), (1, Severity::Warning), "{call}");
    }
}

#[test]
fn a_concatenated_name_is_a_dynamic_setter_and_makes_all_names_warnings() {
    let found = one(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req(
                "a",
                "http://x/{{other}}",
                &script("bru.setVar('la' + suffix, 1)"),
            ),
        ),
    ]);
    assert_eq!(found[0].1, Severity::Warning);
}

#[test]
fn a_setter_in_a_comment_still_counts() {
    // Comments are not stripped: the scan stays conservative and never turns a possible
    // run-time name into an error.
    let found = one(&[
        ("opencollection.yml", ROOT),
        (
            "a.yml",
            &req(
                "a",
                "http://x/{{later}}",
                &script("// bru.setVar('later', 1)\nlog()"),
            ),
        ),
    ]);
    assert_eq!(found[0].1, Severity::Warning);
}

#[test]
fn a_script_that_would_call_out_never_reaches_the_network_during_a_check() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    // A breakout segment that would send a request if anything evaluated it.
    let code =
        format!("log()\n}})(); bru.sendRequest({{ method: 'GET', url: '{url}' }}); (function(){{");
    // Control: evaluated in the wrapper shape, this text does call the send API.
    let rt = rquickjs::Runtime::new().unwrap();
    let js = rquickjs::Context::full(&rt).unwrap();
    let called = js.with(|c| {
        c.eval::<(), _>("globalThis.__sent = false; globalThis.bru = { sendRequest() { globalThis.__sent = true; } }; function log() {}").unwrap();
        c.eval::<(), _>(format!("(async function(){{\n{code}\n}})();")).unwrap();
        c.eval::<bool, _>("globalThis.__sent").unwrap()
    });
    assert!(
        called,
        "the probe text must call the send API when evaluated"
    );
    let (_d, _r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", &url, &script(&code))),
        ],
        &CheckOptions::default(),
    );
    let accepted = listener.accept();
    assert!(
        matches!(accepted, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "a script ran"
    );
}

#[test]
fn a_secret_placeholder_in_a_bad_url_is_unresolved_not_a_finding() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", DEV),
            ("a.yml", &req("a", "http://exa mple.test/{{token}}", "")),
        ],
        &CheckOptions::default(),
    );
    // Only the no-value warning; the request itself is neither a finding nor judged.
    assert_eq!(r.errors(), 0, "{:?}", r.findings());
    assert!(
        r.findings().iter().all(|f| f.item() == "dev"),
        "{:?}",
        r.findings()
    );
    assert_eq!(r.unresolved(), ["a.yml"]);
}

#[test]
fn a_prompt_placeholder_with_a_bad_url_is_unresolved_not_a_finding() {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "http://exa mple.test/{{?pin}}", "")),
        ],
        &CheckOptions::default(),
    );
    assert!(r.findings().is_empty(), "{:?}", r.findings());
    assert_eq!(r.unresolved(), ["a.yml"]);
}

#[test]
fn a_setter_in_a_pre_request_or_test_script_of_another_request_counts() {
    for kind in ["before-request", "tests"] {
        let mut found = one(&[
            ("opencollection.yml", ROOT),
            ("a.yml", &req("a", "http://x/{{later}}", "")),
            (
                "b.yml",
                &req("b", "http://x/", &script_of(kind, "bru.setVar('later', 1)")),
            ),
        ]);
        found.retain(|f| f.2 == FindingKind::UndefinedVariable);
        assert_eq!((found.len(), found[0].1), (1, Severity::Warning), "{kind}");
    }
}

const NO_VALUE_TOKEN: &str = "token is a secret in environment dev with no value; pass --env-var token=VALUE or set it in the app's environment editor";
const KEYCHAIN_UNCHECKED: &str = " (keychain not checked)";

fn secret_files<'a>(env_yml: &'a str, request: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", env_yml),
        ("a.yml", request),
    ]
}

#[test]
fn a_secret_with_no_value_is_one_warning_and_no_error() {
    let a = req("a", "http://{{host}}/{{token}}", "");
    let (_d, r) = check_tree(&secret_files(DEV, &a), &CheckOptions::default().env("dev"));
    assert_eq!(
        (r.errors(), r.findings().len()),
        (0, 1),
        "{:?}",
        r.findings()
    );
    let f = &r.findings()[0];
    assert_eq!(
        (f.message(), f.severity(), f.kind(), f.item()),
        (
            format!("{NO_VALUE_TOKEN}{KEYCHAIN_UNCHECKED}").as_str(),
            Severity::Warning,
            FindingKind::UndefinedVariable,
            "dev"
        )
    );
}

#[test]
fn a_secret_supplied_by_env_var_is_not_warned_about() {
    let a = req("a", "http://{{host}}/{{token}}", "");
    let opts = CheckOptions::default()
        .env("dev")
        .env_var("token", "canary-sup-1");
    let (_d, r) = check_tree(&secret_files(DEV, &a), &opts);
    assert!(r.findings().is_empty(), "{:?}", r.findings());
}

#[test]
fn a_collection_without_secrets_has_no_findings() {
    let env = "name: dev\nvariables:\n  - name: host\n    value: example.test\n";
    let a = req("a", "http://{{host}}/", "");
    let (_d, r) = check_tree(&secret_files(env, &a), &CheckOptions::default().env("dev"));
    assert!(r.findings().is_empty(), "{:?}", r.findings());
}

#[test]
fn an_unresolved_name_lists_the_environments_that_define_it() {
    let dev = "name: dev\nvariables:\n  - name: host\n    value: h\n";
    let stage = "name: stage\nvariables:\n  - name: extra\n    value: x\n";
    let prod = "name: prod\nvariables:\n  - name: extra\n    value: y\n";
    let a = req("a", "http://{{host}}/{{extra}}/{{gone}}", "");
    let files = [
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("environments/stage.yml", stage),
        ("environments/prod.yml", prod),
        ("a.yml", a.as_str()),
    ];
    let (_d, r) = check_tree(&files, &CheckOptions::default().env("dev"));
    let msgs: Vec<&str> = r.findings().iter().map(|f| f.message()).collect();
    assert!(
        msgs.contains(&"variable `extra` is not defined; defined in environments prod, stage"),
        "{msgs:?}"
    );
    assert!(msgs.contains(&"variable `gone` is not defined"), "{msgs:?}");
}

#[test]
fn an_unresolved_name_defined_in_one_other_environment_says_so() {
    let dev = "name: dev\nvariables:\n  - name: host\n    value: h\n";
    let prod = "name: prod\nvariables:\n  - name: extra\n    value: y\n";
    let a = req("a", "http://{{host}}/{{extra}}", "");
    let files = [
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", dev),
        ("environments/prod.yml", prod),
        ("a.yml", a.as_str()),
    ];
    let (_d, r) = check_tree(&files, &CheckOptions::default().env("dev"));
    assert_eq!(
        r.findings()[0].message(),
        "variable `extra` is not defined; defined in environment prod"
    );
}

#[test]
fn an_unresolved_name_still_collapses_across_environments() {
    let found = one(&[
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", "name: dev\nvariables: []\n"),
        ("environments/prod.yml", "name: prod\nvariables: []\n"),
        ("a.yml", &req("a", "http://x/{{gone}}", "")),
    ]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].3, None);
    assert_eq!(found[0].0, "variable `gone` is not defined");
}

#[test]
fn the_no_value_warning_is_per_environment_and_never_carries_a_value() {
    const CANARY: &str = "canary-warn-5e";
    let a = req("a", "http://{{host}}/{{token}}", "");
    let files = [
        ("opencollection.yml", ROOT),
        ("environments/dev.yml", DEV),
        ("environments/prod.yml", PROD),
        ("a.yml", a.as_str()),
    ];
    let dir = common::write_tree(&files);
    let opts = CheckOptions::default().env_var("other", CANARY);
    let r = check_collection(dir.path(), &opts).unwrap();
    let warns: Vec<_> = r
        .findings()
        .iter()
        .filter(|f| f.message().contains("no value"))
        .collect();
    assert_eq!(warns.len(), 2, "{:?}", r.findings());
    let json = serde_json::to_string(&r).unwrap() + &format!("{r:?}");
    assert!(!json.contains(CANARY), "{json}");
}

/// A store holding `token` of `dev` for the collection written from `files`.
fn store_with_token(dir: &std::path::Path, value: Option<&str>) -> Arc<MemoryStore> {
    let store = Arc::new(MemoryStore::new());
    if let Some(value) = value {
        let key = Collection::open(dir).unwrap().secret_key("dev", "token");
        store.set(&key, value).unwrap();
    }
    store
}

#[test]
fn a_secret_the_store_holds_is_not_warned_about() {
    const CANARY: &str = "canary-store-7c";
    let a = req("a", "http://{{host}}/{{token}}", "");
    let dir = common::write_tree(&secret_files(DEV, &a));
    let store = store_with_token(dir.path(), Some(CANARY));
    let opts = CheckOptions::default().env("dev").secrets(store);
    let r = check_collection(dir.path(), &opts).unwrap();
    assert!(r.findings().is_empty(), "{:?}", r.findings());
    assert!(!format!("{r:?}").contains(CANARY));
}

#[test]
fn a_secret_the_store_lacks_warns_without_the_keychain_note() {
    let a = req("a", "http://{{host}}/{{token}}", "");
    let dir = common::write_tree(&secret_files(DEV, &a));
    let store = store_with_token(dir.path(), None);
    let opts = CheckOptions::default().env("dev").secrets(store);
    let r = check_collection(dir.path(), &opts).unwrap();
    let messages: Vec<&str> = r.findings().iter().map(|f| f.message()).collect();
    assert_eq!(messages, [NO_VALUE_TOKEN]);
}

#[test]
fn the_keychain_not_used_marker_drops_the_suffix_without_a_store() {
    let a = req("a", "http://{{host}}/{{token}}", "");
    let dir = common::write_tree(&secret_files(DEV, &a));
    let opts = CheckOptions::default().env("dev").keychain_not_used();
    let r = check_collection(dir.path(), &opts).unwrap();
    let messages: Vec<&str> = r.findings().iter().map(|f| f.message()).collect();
    assert_eq!(messages, [NO_VALUE_TOKEN]);
}

#[test]
fn the_mcp_shell_words_the_warning_without_command_line_flags() {
    let a = req("a", "http://{{host}}/{{token}}", "");
    let opts = CheckOptions::default()
        .env("dev")
        .hint_shell(HintShell::Mcp);
    let (_d, r) = check_tree(&secret_files(DEV, &a), &opts);
    assert_eq!(
        r.findings()[0].message(),
        "token is a secret in environment dev with no value; set it in the app's environment editor (keychain not checked)"
    );
}

/// A store that cannot be reached, so every read fails.
struct Refusing;

impl SecretStore for Refusing {
    fn get(
        &self,
        _: &apinata_core::SecretKey,
    ) -> Result<Option<String>, apinata_core::SecretError> {
        Err(apinata_core::SecretError::Unavailable {
            message: "no session bus".into(),
        })
    }
    fn set(&self, _: &apinata_core::SecretKey, _: &str) -> Result<(), apinata_core::SecretError> {
        unreachable!("check never writes")
    }
    fn delete(&self, _: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
        unreachable!("check never deletes")
    }
}

#[test]
fn an_unreadable_store_is_named_in_the_warning_and_the_note_is_not_added() {
    let a = req("a", "http://{{host}}/{{token}}", "");
    let opts = CheckOptions::default()
        .env("dev")
        .secrets(Arc::new(Refusing));
    let (_d, r) = check_tree(&secret_files(DEV, &a), &opts);
    let messages: Vec<&str> = r.findings().iter().map(|f| f.message()).collect();
    assert_eq!(
        messages,
        [
            "token is a secret in environment dev; its stored value could not be read (secret store unavailable: no session bus); pass --env-var token=VALUE or set it in the app's environment editor"
        ]
    );
}

/// Each literal URL that can never be sent, with the text its finding must carry.
const BAD_URLS: [(&str, &str); 3] = [
    ("", "the URL is empty"),
    ("   ", "the URL is empty"),
    ("http://exa mple.com/", "the URL is not valid"),
];

#[test]
fn a_literal_bad_url_is_an_error_whatever_placeholders_the_environment_has() {
    let envs: [(&str, Option<&str>); 3] = [
        ("no environment", None),
        ("a plain environment", Some("name: dev\n")),
        ("a secret environment", Some(DEV)),
    ];
    for (url, text) in BAD_URLS {
        for (label, env) in envs {
            let a = req("a", url, "");
            let mut files = vec![("opencollection.yml", ROOT), ("a.yml", a.as_str())];
            if let Some(env) = env {
                files.push(("environments/dev.yml", env));
            }
            let (_d, r) = check_tree(&files, &CheckOptions::default());
            let bad: Vec<_> = r
                .findings()
                .iter()
                .filter(|f| f.kind() == FindingKind::InvalidRequest)
                .collect();
            assert_eq!(bad.len(), 1, "{url:?} with {label}: {:?}", r.findings());
            assert_eq!(
                (bad[0].item(), bad[0].message(), bad[0].severity()),
                ("a.yml", text, Severity::Error),
                "{url:?} with {label}"
            );
            assert!(r.unresolved().is_empty(), "{url:?} with {label}");
        }
    }
}

#[test]
fn a_literal_bad_url_is_an_error_beside_an_undefined_name_elsewhere_in_the_request() {
    let a = req(
        "a",
        "http://exa mple.com/",
        "  headers:\n    - name: X\n      value: \"{{nope}}\"\n",
    );
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &a)],
        &CheckOptions::default(),
    );
    let bad = r
        .findings()
        .iter()
        .filter(|f| f.kind() == FindingKind::InvalidRequest)
        .count();
    assert_eq!(bad, 1, "{:?}", r.findings());
}

fn unclosed(url: &str, extra: &str) -> Vec<(String, Severity, Option<String>)> {
    let (_d, r) = check_tree(
        &[
            ("opencollection.yml", ROOT),
            ("environments/dev.yml", DEV),
            ("environments/prod.yml", PROD),
            ("a.yml", &req("a", url, extra)),
        ],
        &CheckOptions::default(),
    );
    r.findings()
        .iter()
        .filter(|f| f.kind() == FindingKind::UnclosedPlaceholder)
        .map(|f| {
            (
                f.message().to_owned(),
                f.severity(),
                f.environment().map(str::to_owned),
            )
        })
        .collect()
}

fn body(kind: &str, data: &str) -> String {
    format!("  body:\n    type: {kind}\n    data: |-\n      {data}\n")
}

#[test]
fn an_unclosed_placeholder_in_the_url_is_one_warning_without_an_environment() {
    let got = unclosed("http://host/{{host}", "");
    assert_eq!(got.len(), 1, "{got:?}");
    let (message, severity, env) = &got[0];
    assert_eq!((*severity, env.as_deref()), (Severity::Warning, None));
    assert!(
        message.contains("{{host}") && message.contains("url"),
        "{message}"
    );
}

#[test]
fn a_placeholder_cut_off_at_the_end_of_a_field_warns() {
    assert_eq!(unclosed("http://host/{{host", "").len(), 1);
}

#[test]
fn each_scanned_field_warns_and_names_itself() {
    let header = "  headers:\n    - name: X-Y\n      value: \"a{{tok}\"\n";
    let query = "  params:\n    - name: q\n      value: \"{{v}\"\n      type: query\n";
    let bearer = "  auth:\n    type: bearer\n    token: \"{{tok}\"\n";
    for (extra, field) in [
        (header.to_owned(), "X-Y"),
        (query.to_owned(), "`q`"),
        (body("text", "hi {{name}"), "body"),
        (bearer.to_owned(), "auth `token`"),
    ] {
        let got = unclosed("http://host/", &extra);
        assert_eq!(got.len(), 1, "{field}: {got:?}");
        assert!(got[0].0.contains(field), "{field}: {}", got[0].0);
    }
}

#[test]
fn text_that_only_looks_like_a_placeholder_does_not_warn() {
    for (kind, data) in [
        ("json", r#"{"a":{"b":1}}"#),
        ("json", r#"{"a":{"b":{"c":1}}}"#),
        ("text", "{{{"),
        ("text", "{{#each x}}..{{/each}}"),
        ("text", "{{> partial}}"),
        ("text", "{{!c}}"),
        ("text", "{{ host}"),
        ("text", "{{host}}"),
    ] {
        let got = unclosed("http://host/", &body(kind, data));
        assert!(got.is_empty(), "{data}: {got:?}");
    }
}

#[test]
fn a_graphql_query_with_nested_braces_does_not_warn() {
    let gql = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"http://host/g\"\n  body:\n    query: \"query { a { b } }\"\n    variables: \"{\\\"a\\\":{\\\"b\\\":1}}\"\n";
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("g.yml", gql)],
        &CheckOptions::default(),
    );
    assert!(r.findings().is_empty(), "{:?}", r.findings());
}

#[test]
fn a_script_is_not_scanned() {
    let script = "runtime:\n  scripts:\n    - type: before-request\n      code: |-\n        const s = \"{{host}\";\n";
    let r = format!("{}{script}", req("a", "http://host/", ""));
    let (_d, rep) = check_tree(
        &[("opencollection.yml", ROOT), ("a.yml", &r)],
        &CheckOptions::default(),
    );
    assert!(
        rep.findings()
            .iter()
            .all(|f| f.kind() != FindingKind::UnclosedPlaceholder),
        "{:?}",
        rep.findings()
    );
}

#[test]
fn the_warning_fires_once_even_when_resolution_fails_in_every_environment() {
    let got = unclosed("http://{{nope}}/{{host}", "");
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].2, None);
}

#[test]
fn a_disabled_header_row_does_not_warn() {
    let extra = "  headers:\n    - name: X\n      value: \"{{tok}\"\n      disabled: true\n";
    assert!(unclosed("http://host/", extra).is_empty());
}

#[test]
fn every_text_body_shape_warns_once() {
    let form = "  body:\n    type: form-urlencoded\n    data:\n      - name: \"{{a}\"\n        value: v\n      - name: b\n        value: \"{{c}\"\n";
    let multi = "  body:\n    type: multipart-form\n    data:\n      - name: f\n        type: text\n        value: \"{{c}\"\n      - name: g\n        type: file\n        value: [\"{{d}\"]\n";
    for (extra, hits) in [
        (body("json", r#"{"a": "{{x}"}"#), 1),
        (body("xml", "<a>{{x}</a>"), 1),
        (body("text", "{{x"), 1),
        (form.to_owned(), 2),
        (multi.to_owned(), 1),
    ] {
        let got = unclosed("http://host/", &extra);
        assert_eq!(got.len(), hits, "{extra}: {got:?}");
    }
}

#[test]
fn an_unclosed_placeholder_in_a_graphql_body_warns() {
    let gql = "info:\n  name: g\n  type: graphql\n  seq: 1\ngraphql:\n  method: POST\n  url: \"http://host/g\"\n  body:\n    query: \"query { a(x: \\\"{{v}\\\") }\"\n";
    let (_d, r) = check_tree(
        &[("opencollection.yml", ROOT), ("g.yml", gql)],
        &CheckOptions::default(),
    );
    let n = r
        .findings()
        .iter()
        .filter(|f| f.kind() == FindingKind::UnclosedPlaceholder)
        .count();
    assert_eq!(n, 1, "{:?}", r.findings());
}

fn unclosed_findings(files: &[(&str, &str)]) -> Vec<(String, String)> {
    let (_d, r) = check_tree(files, &CheckOptions::default());
    r.findings()
        .iter()
        .filter(|f| f.kind() == FindingKind::UnclosedPlaceholder)
        .map(|f| (f.item().to_owned(), f.message().to_owned()))
        .collect()
}

const FOLDER_BAD: &str = "info:\n  name: sub\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: Authorization\n      value: \"Bearer {{token}\"\n";

#[test]
fn a_folder_header_warns_once_naming_the_folder_file_not_each_inheriting_request() {
    let got = unclosed_findings(&[
        ("opencollection.yml", ROOT),
        ("sub/folder.yml", FOLDER_BAD),
        ("sub/a.yml", &req("a", "http://host/", "")),
        ("sub/b.yml", &req("b", "http://host/", "")),
    ]);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "sub/folder.yml");
    assert!(
        got[0].1.contains("folder")
            && got[0].1.contains("Authorization")
            && got[0].1.contains("{{token}"),
        "{}",
        got[0].1
    );
}

#[test]
fn a_folder_auth_field_warns() {
    let folder = "info:\n  name: sub\n  type: folder\n  seq: 1\nrequest:\n  auth:\n    type: bearer\n    token: \"{{tok}\"\n";
    let got = unclosed_findings(&[
        ("opencollection.yml", ROOT),
        ("sub/folder.yml", folder),
        ("sub/a.yml", &req("a", "http://host/", "")),
    ]);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "sub/folder.yml");
    assert!(got[0].1.contains("auth `token`"), "{}", got[0].1);
}

#[test]
fn collection_level_headers_and_auth_warn_naming_the_collection_file() {
    let root = format!(
        "{ROOT}request:\n  headers:\n    - name: X-C\n      value: \"{{{{c}}\"\n  auth:\n    type: bearer\n    token: \"{{{{t}}\"\n"
    );
    let got = unclosed_findings(&[
        ("opencollection.yml", &root),
        ("a.yml", &req("a", "http://host/", "")),
    ]);
    assert_eq!(got.len(), 2, "{got:?}");
    for (item, message) in &got {
        assert_eq!(item, "opencollection.yml");
        assert!(message.contains("collection"), "{message}");
    }
    assert!(got.iter().any(|(_, m)| m.contains("X-C")), "{got:?}");
    assert!(
        got.iter().any(|(_, m)| m.contains("auth `token`")),
        "{got:?}"
    );
}

#[test]
fn a_disabled_folder_header_does_not_warn() {
    let folder = format!("{FOLDER_BAD}      disabled: true\n");
    let got = unclosed_findings(&[
        ("opencollection.yml", ROOT),
        ("sub/folder.yml", &folder),
        ("sub/a.yml", &req("a", "http://host/", "")),
    ]);
    assert!(got.is_empty(), "{got:?}");
}
