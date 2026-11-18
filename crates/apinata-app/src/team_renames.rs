//! The rename rows a team pull leaves in the sync dialog: which are still open, what Move all
//! takes, and when the pull's range may be cleared.

use std::collections::{BTreeMap, BTreeSet};

use apinata_core::{PullRange, SecretName, SecretRename, SecretRenames, TargetState};

/// The rows of one pending pull range and what the person has done with each, keyed by the
/// name the value is stored under.
#[derive(Debug, Clone)]
pub struct RenameRows {
    range: PullRange,
    found: SecretRenames,
    answered: BTreeSet<SecretName>,
    reentering: BTreeMap<SecretName, SecretName>,
    errors: BTreeMap<SecretName, String>,
    failed: Option<String>,
}

impl RenameRows {
    /// The rows `found` for `range`, none answered yet.
    pub fn new(range: PullRange, found: SecretRenames) -> RenameRows {
        RenameRows {
            range,
            found,
            answered: BTreeSet::new(),
            reentering: BTreeMap::new(),
            errors: BTreeMap::new(),
            failed: None,
        }
    }

    /// The pull range these rows answer.
    pub fn range(&self) -> &PullRange {
        &self.range
    }

    /// Renames not listed because they touch an unsettled clash.
    pub fn held_back(&self) -> usize {
        self.found.held_back
    }

    /// Rows neither answered nor waiting for a value typed again, in order.
    pub fn open_rows(&self) -> Vec<&SecretRename> {
        (self.found.renames.iter())
            .filter(|r| !self.answered.contains(&r.from) && !self.reentering.contains_key(&r.from))
            .collect()
    }

    /// A fresh ask: new range and rows; answers and re-entry marks kept by old name, while a
    /// failed ask and per-row errors are cleared.
    pub fn refresh(&mut self, range: PullRange, found: SecretRenames) {
        self.range = range;
        self.found = found;
        self.errors.clear();
        self.failed = None;
    }

    /// The person moved or dismissed the row of `from`.
    pub fn answer(&mut self, from: &SecretName) {
        self.answered.insert(from.clone());
        self.errors.remove(from);
    }

    /// The value of `from` is being typed again under `to`.
    pub fn reenter(&mut self, from: SecretName, to: SecretName) {
        self.errors.remove(&from);
        self.reentering.insert(from, to);
    }

    /// The values being typed again, old name first.
    pub fn reentering(&self) -> Vec<(SecretName, SecretName)> {
        (self.reentering.iter())
            .map(|(from, to)| (from.clone(), to.clone()))
            .collect()
    }

    /// The value of `from` was typed again and its old key is gone: the row is answered.
    pub fn finished(&mut self, from: &SecretName) {
        self.reentering.remove(from);
        self.answer(from);
    }

    /// A failed ask: rows kept as they were, `settle_now` false until the next good ask.
    pub fn fail(&mut self, message: String) {
        self.failed = Some(message);
    }

    /// Why the last ask failed.
    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// Why the move of `from` failed; the row stays open.
    pub fn set_error(&mut self, from: &SecretName, message: String) {
        self.errors.insert(from.clone(), message);
    }

    /// The last error of the row of `from`.
    pub fn error(&self, from: &SecretName) -> Option<&str> {
        self.errors.get(from).map(String::as_str)
    }

    /// Open, sure rows with one candidate that is Free or Same.
    pub fn movable_sure(&self) -> Vec<(SecretName, SecretName)> {
        (self.open_rows().into_iter())
            .filter(|r| r.sure)
            .filter_map(|r| match r.to.as_slice() {
                [(to, TargetState::Free | TargetState::Same)] => Some((r.from.clone(), to.clone())),
                _ => None,
            })
            .collect()
    }

    /// Clear the range only with no failure, nothing held back, no open row, nothing being typed.
    pub fn settle_now(&self) -> bool {
        self.failed.is_none()
            && self.found.held_back == 0
            && self.reentering.is_empty()
            && self.open_rows().is_empty()
    }
}

/// "dev / token → staging / api_token", plus " (renamed? check before moving)" when not sure.
pub fn row_text(row: &SecretRename, candidate: usize) -> String {
    let from = &row.from;
    let mut text = match row.to.get(candidate) {
        Some((to, _)) => format!(
            "{} / {} → {} / {}",
            from.environment, from.variable, to.environment, to.variable
        ),
        None => format!("{} / {}", from.environment, from.variable),
    };
    if !row.sure {
        text.push_str(" (renamed? check before moving)");
    }
    text
}
