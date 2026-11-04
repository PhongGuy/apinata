//! Request variables, assertions and settings.

use crate::parity;

use apinata_core::{Assertion, RequestFile, RequestSettings, Timeout, VarValue, Variable};
use parity::{check_requests, file_with, keys, saved};

const ONLY_VARS: &str = "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: u\nruntime:\n  variables:\n    - name: a\n      value: \"1\"\n";

fn typed(kind: &str, data: &str) -> VarValue {
    VarValue::Typed {
        kind: kind.to_owned(),
        data: data.to_owned(),
    }
}

#[test]
fn variable_edits_match_bruno() {
    check_requests("vars-", |id, r| {
        let mut vars = r.vars();
        match id {
            "vars-add" => vars = vec![Variable::new("a", VarValue::Text("1".to_owned()))],
            "vars-edit-typed" => vars[1].value = typed("number", "7"),
            "vars-disable" => vars[0].enabled = false,
            "vars-clear-keeps-scripts" => vars.clear(),
            other => panic!("no Apinata edit for {other}"),
        }
        r.set_vars(vars);
    });
}

#[test]
fn assertion_edits_match_bruno() {
    check_requests("assert-", |id, r| {
        let mut assertions = r.assertions();
        match id {
            "assert-add" => {
                let mut off = Assertion::new("res.body.ok", "isTruthy", None);
                off.set_enabled(false);
                assertions = vec![
                    Assertion::new("res.status", "eq", Some("200".to_owned())),
                    off,
                ];
            }
            "assert-edit" => {
                assertions[0] = Assertion::new("res.status", "neq", Some("500".to_owned()))
            }
            "assert-clear" => assertions.clear(),
            other => panic!("no Apinata edit for {other}"),
        }
        r.set_assertions(assertions);
    });
}

#[test]
fn settings_edits_match_bruno() {
    check_requests("settings-", |id, r| {
        let mut settings = r.settings();
        match id {
            "settings-timeout" => settings.timeout = Timeout::Millis(5000),
            "settings-all" => {
                settings.encode_url = false;
                settings.timeout = Timeout::Inherit;
                settings.follow_redirects = false;
                settings.max_redirects = 2;
                settings.forward_authorization_header = false;
            }
            other => panic!("no Apinata edit for {other}"),
        }
        r.set_settings(settings);
    });
}

#[test]
fn variables_read_plain_and_typed_values() {
    let text = "info:\n  name: r\nhttp:\n  url: u\nruntime:\n  variables:\n    - name: s\n      value: 5\n    - name: n\n      value:\n        type: number\n        data: 42\n    - name: off\n      disabled: true\n";
    let (_dir, path) = file_with(text);
    let r = RequestFile::load(&path).expect("loads");
    let mut off = Variable::new("off", VarValue::Text(String::new()));
    off.enabled = false;
    assert_eq!(
        r.vars(),
        vec![
            Variable::new("s", VarValue::Text("5".to_owned())),
            Variable::new("n", typed("number", "42")),
            off
        ]
    );
}

#[test]
fn a_string_typed_variable_is_written_as_plain_text() {
    let (_dir, path) = file_with(ONLY_VARS);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_vars(vec![Variable::new("a", typed("string", "x"))]);
    r.save().expect("saves");
    assert_eq!(
        saved(&path)["runtime"]["variables"][0]["value"],
        yaml_serde::Value::from("x")
    );
}

#[test]
fn clearing_the_last_runtime_list_removes_runtime() {
    let (_dir, path) = file_with(ONLY_VARS);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_vars(Vec::new());
    r.save().expect("saves");
    assert!(saved(&path).get("runtime").is_none());
}

#[test]
fn a_hand_written_runtime_stays_clean_when_written_back() {
    let text = "info:\n  name: r\nhttp:\n  url: u\nruntime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: 200\n      x-note: keep\n  variables:\n    - name: a\n      value: 1\n";
    let (_dir, path) = file_with(text);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_vars(r.vars());
    r.set_assertions(r.assertions());
    r.set_settings(r.settings());
    assert!(!r.is_dirty());
}

#[test]
fn an_edited_assertion_is_reparsed_the_way_bruno_writes_it() {
    let (_dir, path) = file_with(ONLY_VARS);
    let mut r = RequestFile::load(&path).expect("loads");
    r.set_assertions(vec![
        Assertion::new("res.body", "isEmpty", Some("ignored".to_owned())),
        Assertion::new("res.body.name", "custom", Some("x".to_owned())),
    ]);
    r.save().expect("saves");
    let after = saved(&path);
    let list = &after["runtime"]["assertions"];
    assert_eq!(list[0]["operator"], yaml_serde::Value::from("isEmpty"));
    assert!(list[0].get("value").is_none());
    // An operator Bruno does not know becomes an `eq` on the whole text.
    assert_eq!(list[1]["operator"], yaml_serde::Value::from("eq"));
    assert_eq!(list[1]["value"], yaml_serde::Value::from("custom x"));
    assert_eq!(keys(&after["runtime"]), ["variables", "assertions"]);
}

#[test]
fn settings_read_with_brunos_defaults_and_coercions() {
    let (_dir, path) = file_with("info:\n  name: r\nhttp:\n  url: u\n");
    // A request with no timeout of its own takes the default when sent, so it reads as Inherit.
    let mut defaults = RequestSettings::new();
    defaults.timeout = Timeout::Inherit;
    assert_eq!(
        RequestFile::load(&path).expect("loads").settings(),
        defaults
    );
    let (_dir, path) = file_with("info:\n  name: r\nhttp:\n  url: u\nsettings:\n  timeout: null\n");
    let s = RequestFile::load(&path).expect("loads").settings();
    assert_eq!(s.timeout, Timeout::Inherit);
    let text = "info:\n  name: r\nhttp:\n  url: u\nsettings:\n  encodeUrl: \"false\"\n  timeout: -3\n  followRedirects: nope\n  maxRedirects: 2.9\n";
    let (_dir, path) = file_with(text);
    let s = RequestFile::load(&path).expect("loads").settings();
    assert!(!s.encode_url);
    assert_eq!(s.timeout, Timeout::Unlimited);
    assert!(s.follow_redirects);
    assert_eq!(s.max_redirects, 2);
    assert!(s.forward_authorization_header);
}

#[test]
fn a_settings_edit_on_a_file_without_settings_adds_only_the_changed_key() {
    let (_dir, path) = file_with("info:\n  name: r\nhttp:\n  url: u\n");
    let mut r = RequestFile::load(&path).expect("loads");
    let mut s = r.settings();
    s.timeout = Timeout::Millis(250);
    r.set_settings(s);
    r.save().expect("saves");
    let after = saved(&path);
    assert_eq!(keys(&after["settings"]), ["timeout"]);
    assert_eq!(after["settings"]["timeout"], yaml_serde::Value::from(250));
}

#[test]
fn debug_shows_variable_names_and_sizes_only() {
    let shown = format!(
        "{:?}",
        Variable::new("api_key", VarValue::Text("hunter2".to_owned()))
    );
    assert!(
        shown.contains("api_key") && shown.contains("7 bytes"),
        "{shown}"
    );
    assert!(!shown.contains("hunter2"), "{shown}");
}

#[test]
fn a_timeout_set_back_to_its_saved_value_leaves_the_request_clean() {
    // No settings block: Custom then Default removes what the first edit added.
    let (_dir, path) = file_with("info:\n  name: r\nhttp:\n  url: u\n");
    let mut r = RequestFile::load(&path).expect("loads");
    let mut s = r.settings();
    s.timeout = Timeout::Millis(250);
    r.set_settings(s);
    assert!(r.is_dirty());
    s.timeout = Timeout::Inherit;
    r.set_settings(s);
    assert!(
        !r.is_dirty(),
        "Default after Custom is the saved file again"
    );

    // A saved null reads as inherit and comes back as null; the other key is untouched.
    let text =
        "info:\n  name: r\nhttp:\n  url: u\nsettings:\n  encodeUrl: false\n  timeout: null\n";
    let (_dir, path) = file_with(text);
    let mut r = RequestFile::load(&path).expect("loads");
    let mut s = r.settings();
    s.timeout = Timeout::Millis(250);
    r.set_settings(s);
    s.timeout = Timeout::Inherit;
    r.set_settings(s);
    assert!(!r.is_dirty());
    assert!(!r.settings().encode_url);
}

#[test]
fn an_ill_typed_saved_timeout_is_not_written_back() {
    // `abc` reads as no limit; choosing no limit again writes Bruno's 0, not the bad node.
    let text = "info:\n  name: r\nhttp:\n  url: u\nsettings:\n  timeout: abc\n";
    let (_dir, path) = file_with(text);
    let mut r = RequestFile::load(&path).expect("loads");
    let mut s = r.settings();
    assert_eq!(s.timeout, Timeout::Unlimited);
    s.timeout = Timeout::Millis(250);
    r.set_settings(s);
    s.timeout = Timeout::Unlimited;
    r.set_settings(s);
    assert!(r.is_dirty());
    r.save().expect("saves");
    assert_eq!(
        saved(&path)["settings"]["timeout"],
        yaml_serde::Value::from(0)
    );
}

#[test]
fn restoring_one_setting_keeps_a_pending_edit_on_another() {
    // The request stays dirty while any key differs from the file, and is clean once all are back.
    let text =
        "info:\n  name: r\nhttp:\n  url: u\nsettings:\n  encodeUrl: false\n  timeout: null\n";
    let (_dir, path) = file_with(text);
    let mut r = RequestFile::load(&path).expect("loads");
    let mut s = r.settings();
    s.encode_url = true;
    s.timeout = Timeout::Millis(250);
    r.set_settings(s);
    s.timeout = Timeout::Inherit;
    r.set_settings(s);
    assert!(r.is_dirty(), "the encodeUrl edit is still pending");
    assert!(r.settings().encode_url);
    s.encode_url = false;
    r.set_settings(s);
    assert!(!r.is_dirty(), "every key is back to the saved file");
}
