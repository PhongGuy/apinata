//! The vendored reference test cases for export translation, and the harness that runs them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::tokens::{Tok, contains, same, tokens};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub(super) enum Kind {
    #[serde(rename = "toBe")]
    ToBe,
    #[serde(rename = "toContain")]
    ToContain,
}

/// One assertion of a reference case on the translated script.
#[derive(Debug, serde::Deserialize)]
pub(super) struct Check {
    pub kind: Kind,
    pub text: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct Case {
    pub id: String,
    #[expect(
        dead_code,
        reason = "read when debugging a case; reports name cases by id"
    )]
    pub name: String,
    pub input: String,
    pub checks: Vec<Check>,
}

/// The reference's translation tables, rows in file order.
pub(super) struct Table {
    pub simple: Vec<(String, String)>,
    pub call_to_property: Vec<(String, String)>,
}

/// The corpus outcome; `differing` and `stale` hold case ids, `stale` the listed deviations
/// that now match.
pub(super) struct Report {
    pub plan_a: usize,
    pub pending: usize,
    pub matched: usize,
    pub deviated: usize,
    pub differing: Vec<String>,
    pub stale: Vec<String>,
}

impl Report {
    pub(super) fn summary(&self) -> String {
        format!(
            "plan A: {} of {} match, {} deviations, {} differ; {} pending for plan B",
            self.matched,
            self.plan_a,
            self.deviated,
            self.differing.len(),
            self.pending
        )
    }
}

fn fixture(name: &str) -> String {
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference/2026-11-09-postman-export-translation");
    std::fs::read_to_string(dir.join(name)).expect("reads the vendored fixture")
}

pub(super) fn cases() -> Vec<Case> {
    serde_json::from_str(&fixture("cases.json")).expect("parses cases.json")
}

pub(super) fn table() -> Table {
    let json: Value = serde_json::from_str(&fixture("table.json")).expect("parses table.json");
    let rows = |key: &str| -> Vec<(String, String)> {
        json[key]
            .as_object()
            .expect("a table object")
            .iter()
            .map(|(from, to)| (from.clone(), to.as_str().expect("a string row").to_owned()))
            .collect()
    };
    Table {
        simple: rows("simple"),
        call_to_property: rows("callToProperty"),
    }
}

/// Case id to the reason its output knowingly differs from the reference.
pub(super) fn deviations() -> BTreeMap<String, String> {
    serde_json::from_str(&fixture("deviations.json")).expect("parses deviations.json")
}

/// The later-plan marker in `input`, found on tokens so strings and comments never count.
pub(super) fn pending(input: &str) -> Option<&'static str> {
    let toks = tokens(input);
    for (k, tok) in toks.iter().enumerate() {
        let Tok::Word(word) = tok else { continue };
        let member = k > 0 && toks[k - 1] == Tok::Punct(".".into());
        let called = toks.get(k + 1) == Some(&Tok::Punct("(".into()));
        if matches!(word.as_str(), "test" | "expect") && !member && called {
            return Some("test or expect call");
        }
    }
    None
}

/// Whether `output` satisfies `check`, on exact text first and then on tokens.
pub(super) fn passes(check: &Check, output: &str) -> bool {
    match check.kind {
        Kind::ToBe => output.trim() == check.text.trim() || same(output, &check.text),
        Kind::ToContain => output.contains(&check.text) || contains(output, &check.text),
    }
}

/// Runs every case not pending for a later plan through `translate` and tallies the outcome.
pub(super) fn run(translate: &dyn Fn(&str) -> String) -> Report {
    let deviations = deviations();
    let mut report = Report {
        plan_a: 0,
        pending: 0,
        matched: 0,
        deviated: 0,
        differing: Vec::new(),
        stale: Vec::new(),
    };
    for case in cases() {
        if pending(&case.input).is_some() {
            report.pending += 1;
            continue;
        }
        report.plan_a += 1;
        let output = translate(&case.input);
        let listed = deviations.contains_key(&case.id);
        if case.checks.iter().all(|check| passes(check, &output)) {
            report.matched += 1;
            if listed {
                report.stale.push(case.id);
            }
        } else if listed {
            report.deviated += 1;
        } else {
            report.differing.push(case.id);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::{Check, Kind, cases, deviations, passes, pending, run, table};

    #[test]
    fn the_vendored_corpus_has_every_case() {
        let cases = cases();
        assert_eq!(cases.len(), 195);
        let checks: Vec<&Check> = cases.iter().flat_map(|c| &c.checks).collect();
        assert_eq!(checks.len(), 274);
        assert_eq!(checks.iter().filter(|c| c.kind == Kind::ToBe).count(), 163);
        let ids: std::collections::HashSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), 195);
        let table = table();
        assert_eq!(table.simple.len(), 105);
        assert_eq!(table.call_to_property.len(), 13);
        assert_eq!(
            table.simple[0],
            (
                "bru.getGlobalEnvVar".to_owned(),
                "pm.globals.get".to_owned()
            )
        );
        assert!(
            deviations()
                .values()
                .all(|reason| !reason.trim().is_empty())
        );
    }

    #[test]
    fn plan_b_cases_are_recognised_by_their_markers() {
        assert_eq!(
            pending("test('ok', () => {});"),
            Some("test or expect call")
        );
        assert_eq!(
            pending("expect(1).to.equal(1);"),
            Some("test or expect call")
        );
        assert_eq!(pending("expect.fail('x');"), None);
        assert_eq!(pending("x.test(1);"), None);
        assert_eq!(
            pending(
                "jar.getCookie(u, 'a'); jar.setCookie(u, n, v); jar.getCookies(u); jar.deleteCookie(u, n); jar.deleteCookies(u);"
            ),
            None
        );
        assert_eq!(pending("bru.sendRequest({}); req.setHeader(a, b);"), None);
        assert_eq!(
            cases()
                .iter()
                .filter(|c| pending(&c.input).is_none())
                .count(),
            180
        );
    }

    #[test]
    fn checks_pass_on_text_or_on_tokens() {
        let to_be = Check {
            kind: Kind::ToBe,
            text: "a = 'x';".into(),
        };
        assert!(passes(&to_be, "a = \"x\";\n"));
        assert!(!passes(&to_be, "a = 'y';"));
        let to_contain = Check {
            kind: Kind::ToContain,
            text: "pm.request.headers.upsert({key: k, value: v})".into(),
        };
        assert!(passes(
            &to_contain,
            "pm.request.headers.upsert({\n  key: k,\n  value: v\n});"
        ));
        assert!(!passes(
            &to_contain,
            "pm.request.headers.add({key: k, value: v});"
        ));
    }

    #[test]
    fn an_identity_translation_matches_no_plan_a_case() {
        let report = run(&|s: &str| s.to_owned());
        eprintln!("{}", report.summary());
        assert_eq!((report.plan_a, report.pending), (180, 15));
        assert_eq!(report.matched, 0);
        assert_eq!(report.differing.len(), 180);
        assert!(report.stale.is_empty());
    }
}
