//! The "Script trust" settings of a collection: whether its scripts run, from the per-user script
//! trust file, which this view edits in place.

use std::path::PathBuf;

use apinata_core::{ScriptDecision, ScriptTrust};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::app::AppState;
use crate::approval_prompt::untrusted_text;
use crate::ui;

// The segmented options in order, each with the decision it records.
const CHOICES: [(&str, Option<ScriptDecision>); 3] = [
    ("Ask", None),
    ("Run scripts", Some(ScriptDecision::Allowed)),
    ("Never", Some(ScriptDecision::Blocked)),
];

/// The option labels in order, for tests that pin the wording.
pub fn choice_labels() -> [&'static str; 3] {
    CHOICES.map(|(label, _)| label)
}

/// Shows one collection's script decision; picking an option writes it to the trust file.
pub struct ScriptSettingsView {
    trust_file: PathBuf,
    root: PathBuf,
    decision: Option<ScriptDecision>,
    error: Option<String>,
    _subs: Vec<Subscription>,
}

impl ScriptSettingsView {
    /// A view of the decision for the collection at `root` in `trust_file`, following the
    /// app's approvals when `AppState` is set so a dialog answer shows at once.
    pub fn new(trust_file: PathBuf, root: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut subs = Vec::new();
        if let Some(state) = cx.try_global::<AppState>() {
            let approvals = state.approvals.clone();
            subs.push(cx.observe(&approvals, |this, _, cx| this.reload(cx)));
        }
        let mut view = ScriptSettingsView {
            trust_file,
            root,
            decision: None,
            error: None,
            _subs: subs,
        };
        view.reload(cx);
        view
    }

    /// The collection's decision; `None` asks before scripts run.
    pub fn decision(&self) -> Option<ScriptDecision> {
        self.decision
    }

    /// The last load or write error, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Re-reads the trust file; a file that fails to load shows its error.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        match ScriptTrust::load(&self.trust_file) {
            Ok(store) => {
                self.decision = store.decision(&self.root);
                self.error = None;
            }
            Err(e) => {
                self.decision = None;
                self.error = Some(e.to_string());
            }
        }
        cx.notify();
    }

    // Updates the store under its lock so entries written elsewhere survive; a store that
    // fails to load is never overwritten.
    fn pick(&mut self, decision: Option<ScriptDecision>, cx: &mut Context<Self>) {
        let written = ScriptTrust::update(&self.trust_file, |store| {
            store.set_decision(&self.root, decision);
        });
        match written {
            Ok(()) => {
                self.reload(cx);
                // Other views of the decision follow the approvals model.
                if let Some(state) = cx.try_global::<AppState>() {
                    let (approvals, root) = (state.approvals.clone(), self.root.clone());
                    cx.defer(move |cx| {
                        approvals.update(cx, |a, cx| a.note_script_decision(&root, decision, cx))
                    });
                }
            }
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
            }
        }
    }
}

impl Render for ScriptSettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A decision this build does not know shows no option selected.
        let selected = CHOICES
            .iter()
            .position(|(_, d)| *d == self.decision)
            .unwrap_or(CHOICES.len());
        let this = cx.entity().downgrade();
        let group = ui::segmented(
            "script-decision",
            CHOICES.map(|(label, _)| label),
            selected,
            move |ix, _, cx| {
                if let Some(&(_, decision)) = CHOICES.get(*ix) {
                    let _ = this.update(cx, |v, cx| v.pick(decision, cx));
                }
            },
        );
        v_flex()
            .id("script-settings")
            .gap_2()
            .child("Run this collection's scripts")
            .child(group)
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .id("config-scripts-error")
                        .test_support()
                        .child(untrusted_text(&error)),
                )
            })
    }
}
