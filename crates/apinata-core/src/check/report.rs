//! The check report: findings, their ordering and the stable JSON shape.

use serde::{Serialize, Serializer};
use serde_json::{Map, Value, json};

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Severity {
    /// The item would fail or misbehave when sent.
    Error,
    /// The item works but is probably not what the author meant.
    Warning,
}

impl Severity {
    /// The lowercase name the text output and the JSON both use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// What a finding is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FindingKind {
    /// A `{{name}}` that no scope defines.
    UndefinedVariable,
    /// A referenced file or certificate that does not exist.
    MissingFile,
    /// A script that does not parse.
    ScriptSyntax,
    /// A file that failed to load.
    UnparsableItem,
    /// A request that cannot be built, for a reason other than a variable.
    InvalidRequest,
    /// Two siblings share a non-zero sequence number.
    DuplicateSequence,
    /// A `{{name}` with no closing `}}`, which is sent as literal text.
    UnclosedPlaceholder,
}

/// One problem found in one item, optionally in one environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Finding {
    item: String,
    environment: Option<String>,
    severity: Severity,
    kind: FindingKind,
    message: String,
}

impl Finding {
    pub(crate) fn new(
        item: impl Into<String>,
        environment: Option<&str>,
        severity: Severity,
        kind: FindingKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            item: item.into(),
            environment: environment.map(str::to_owned),
            severity,
            kind,
            message: message.into(),
        }
    }

    /// The item's path relative to the collection root, with `/` separators.
    pub fn item(&self) -> &str {
        &self.item
    }

    /// The environment the finding applies to; `None` when it holds in all of them.
    pub fn environment(&self) -> Option<&str> {
        self.environment.as_deref()
    }

    /// How serious the finding is.
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// What the finding is about.
    pub fn kind(&self) -> FindingKind {
        self.kind
    }

    /// A human-readable description.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// The result of checking a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CheckReport {
    findings: Vec<Finding>,
    items_checked: usize,
    unresolved: Vec<String>,
    environments: Vec<String>,
}

impl CheckReport {
    /// Sort and de-duplicate `findings`, folding a finding that every checked
    /// environment shares into one without an environment.
    pub(crate) fn build(
        findings: Vec<Finding>,
        items_checked: usize,
        unresolved: Vec<String>,
        environments: Vec<String>,
    ) -> Self {
        let mut findings = collapse(findings, environments.len());
        findings.sort_by(|a, b| {
            (&a.item, a.kind, &a.environment, &a.message).cmp(&(
                &b.item,
                b.kind,
                &b.environment,
                &b.message,
            ))
        });
        findings.dedup();
        Self {
            findings,
            items_checked,
            unresolved,
            environments,
        }
    }

    /// Every finding, sorted by item, kind, environment and message.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// The number of error findings.
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    /// The number of warning findings.
    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    /// The number of request files checked.
    pub fn items_checked(&self) -> usize {
        self.items_checked
    }

    /// Items that could not be fully checked because a placeholder broke resolution.
    pub fn unresolved(&self) -> &[String] {
        &self.unresolved
    }

    /// The environments that were checked, in order.
    pub fn environments(&self) -> &[String] {
        &self.environments
    }

    /// The report as the `--json` document, without a version.
    pub fn to_json(&self) -> Value {
        json!({
            "summary": {
                "errors": self.errors(),
                "warnings": self.warnings(),
                "items_checked": self.items_checked,
                "unresolved": self.unresolved,
                "environments": self.environments,
            },
            "findings": self.findings,
        })
    }

    /// `to_json` with `"version": 1` as the first key.
    pub fn to_json_versioned(&self) -> Value {
        let mut map = Map::new();
        map.insert("version".to_owned(), json!(1));
        if let Value::Object(rest) = self.to_json() {
            map.extend(rest);
        }
        Value::Object(map)
    }

    fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

impl Serialize for CheckReport {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

/// A finding present with the same identity in every environment becomes one
/// finding with no environment; the rest keep theirs.
fn collapse(findings: Vec<Finding>, env_count: usize) -> Vec<Finding> {
    if env_count == 0 {
        return findings;
    }
    type Identity = (String, FindingKind, Severity, String);
    let key = |f: &Finding| (f.item.clone(), f.kind, f.severity, f.message.clone());
    let mut out = Vec::new();
    let mut groups: Vec<(Identity, Vec<Finding>)> = Vec::new();
    for f in findings {
        if f.environment.is_none() {
            out.push(f);
            continue;
        }
        let k = key(&f);
        match groups.iter_mut().find(|(gk, _)| *gk == k) {
            Some((_, members)) => members.push(f),
            None => groups.push((k, vec![f])),
        }
    }
    for (_, members) in groups {
        let mut envs: Vec<_> = members.iter().map(|f| f.environment.clone()).collect();
        envs.sort();
        envs.dedup();
        if envs.len() == env_count {
            let mut f = members.into_iter().next().expect("group is non-empty");
            f.environment = None;
            out.push(f);
        } else {
            out.extend(members);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_is_the_serde_name_of_every_severity() {
        for severity in [Severity::Error, Severity::Warning] {
            let serde_name = serde_json::to_value(severity).unwrap();
            assert_eq!(serde_name, severity.as_str(), "{severity:?}");
        }
    }

    fn finding(env: &str, message: &str) -> Finding {
        Finding::new(
            "a.yml",
            Some(env),
            Severity::Error,
            FindingKind::UndefinedVariable,
            message,
        )
    }

    fn envs(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn collapse_merges_a_finding_present_in_every_environment() {
        let report = CheckReport::build(
            vec![finding("dev", "shared"), finding("prod", "shared")],
            1,
            Vec::new(),
            envs(&["dev", "prod"]),
        );
        assert_eq!(report.findings().len(), 1);
        assert_eq!(report.findings()[0].environment(), None);
    }

    #[test]
    fn collapse_keeps_the_environment_of_a_finding_seen_in_one() {
        let report = CheckReport::build(
            vec![
                finding("dev", "shared"),
                finding("prod", "shared"),
                finding("prod", "only prod"),
            ],
            1,
            Vec::new(),
            envs(&["dev", "prod"]),
        );
        let only: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.message() == "only prod")
            .collect();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].environment(), Some("prod"));
        let shared: Vec<_> = report
            .findings()
            .iter()
            .filter(|f| f.message() == "shared")
            .collect();
        assert_eq!((shared.len(), shared[0].environment()), (1, None));
    }
}
