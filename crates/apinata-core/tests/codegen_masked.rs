//! `generate_masked`: no target shows a secret in any form, whatever characters it holds.

use crate::common;

use apinata_core::{ResolvedRequest, Target, generate, generate_masked};
use common::resolved_one;

// Characters string literals escape, between distinct alphanumeric runs: no escaper changes
// letters or digits, so any escaped or unescaped form of the secret keeps a run whole.
const SECRET: &str = "Qx7Zr$Wv9Kp'Jm4Hs\"Bt2Ny\\Fd6Lc`Gu3Tk#{Ye8Pw}\nRo5Xa-\u{fc}\u{20ac}-Vi1Mq";
const RUNS: [&str; 9] = [
    "Qx7Zr", "Wv9Kp", "Jm4Hs", "Bt2Ny", "Fd6Lc", "Gu3Tk", "Ye8Pw", "Ro5Xa", "Vi1Mq",
];

/// `text` inside a double-quoted YAML scalar.
fn yaml(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// The secret up to its newline, for slots that cannot hold one.
fn head() -> &'static str {
    SECRET.split('\n').next().unwrap_or_default()
}

/// One request per slot a secret can sit in, with the secret value it holds there.
fn slots() -> Vec<(&'static str, ResolvedRequest, &'static str)> {
    let s = yaml(SECRET);
    let h = yaml(head());
    let url = "http://x.test/p";
    vec![
        (
            "header",
            resolved_one(
                "GET",
                url,
                &format!("  headers:\n    - name: X-K\n      value: \"k {h} k\"\n"),
                "",
            ),
            head(),
        ),
        (
            "text body",
            resolved_one(
                "POST",
                url,
                &format!("  body:\n    type: text\n    data: \"pw={s};\"\n"),
                "",
            ),
            SECRET,
        ),
        (
            "json body",
            resolved_one(
                "POST",
                url,
                &format!(
                    "  body:\n    type: json\n    data: \"{{\\\"pw\\\": {}}}\"\n",
                    yaml(&serde_json::to_string(SECRET).unwrap_or_default())
                ),
                "",
            ),
            SECRET,
        ),
        (
            "form body",
            resolved_one(
                "POST",
                url,
                &format!(
                    "  body:\n    type: form-urlencoded\n    data:\n      - name: pw\n        value: \"{s}\"\n"
                ),
                "",
            ),
            SECRET,
        ),
        (
            "multipart text",
            resolved_one(
                "POST",
                url,
                &format!(
                    "  body:\n    type: multipart-form\n    data:\n      - name: pw\n        type: text\n        value: \"{s}\"\n"
                ),
                "",
            ),
            SECRET,
        ),
        (
            "basic auth",
            resolved_one(
                "GET",
                url,
                &format!("  auth:\n    type: basic\n    username: bob\n    password: \"{h}\"\n"),
                "",
            ),
            head(),
        ),
        (
            "digest auth",
            resolved_one(
                "GET",
                url,
                &format!("  auth:\n    type: digest\n    username: bob\n    password: \"{s}\"\n"),
                "",
            ),
            SECRET,
        ),
    ]
}

#[test]
fn no_target_shows_a_secret_that_it_escapes() {
    for (slot, req, secret) in slots() {
        for target in Target::ALL {
            let raw = generate(&req, *target);
            let masked = generate_masked(&req, *target, &[(secret, "{{token}}")], "[redacted]");
            let runs = RUNS.iter().filter(|run| secret.contains(*run));
            for run in runs {
                if raw.contains(run) {
                    assert!(
                        masked.contains("{{token}}") || masked.contains("[redacted]"),
                        "{} {slot}: no mask in {masked}",
                        target.id()
                    );
                }
                assert!(
                    !masked.contains(run),
                    "{} leaks {run} from the {slot}: {masked}",
                    target.id()
                );
            }
        }
    }
}

#[test]
fn a_secret_in_the_query_is_masked_in_every_target() {
    let secret = "Qx7Zr$Wv9Kp";
    let req = resolved_one("GET", &format!("http://x.test/p?k=1{secret}"), "", "");
    for target in Target::ALL {
        let masked = generate_masked(&req, *target, &[(secret, "<secret>")], "<secret>");
        assert!(
            !masked.contains("Qx7Zr") && !masked.contains("Wv9Kp"),
            "{}: {masked}",
            target.id()
        );
        assert!(masked.contains("<secret>"), "{}: {masked}", target.id());
    }
}

#[test]
fn a_secret_inside_another_never_splits_it() {
    let req = resolved_one(
        "POST",
        "http://x.test/p",
        "  body:\n    type: text\n    data: \"Ab$9-long-Cd'7 and Ab$9\"\n",
        "",
    );
    for target in Target::ALL {
        let masked = generate_masked(
            &req,
            *target,
            &[("Ab$9", "{{short}}"), ("Ab$9-long-Cd'7", "{{long}}")],
            "[redacted]",
        );
        assert!(masked.contains("{{long}}"), "{}: {masked}", target.id());
        assert!(masked.contains("{{short}}"), "{}: {masked}", target.id());
        assert!(!masked.contains("Cd"), "{}: {masked}", target.id());
    }
}

#[test]
fn without_a_secret_to_mask_the_snippet_is_unchanged() {
    let req = resolved_one("GET", "http://x.test/p", "", "");
    for target in Target::ALL {
        // A value under 4 characters is never masked, as every redactor leaves it.
        let masked = generate_masked(&req, *target, &[("x.t", "{{h}}")], "[redacted]");
        assert_eq!(masked, generate(&req, *target), "{}", target.id());
    }
}
