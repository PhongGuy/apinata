//! Whether a send or run may execute its collection's scripts: decided once per collection,
//! by asking the user in the shell window before the first send that needs scripts.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use apinata_core::{ScriptDecision, ScriptOptions, ScriptTrust, plan_script_texts};
use gpui_kit::{AnyWindowHandle, App, AsyncApp, Entity, Global};

use crate::app::AppState;
use crate::approvals::Approvals;
use crate::command::Planned;
use crate::confirm::{Ask, Choice, ask};
use crate::settings::{RunScripts, SCRIPT_TIMEOUT_SECS};

/// What to do with a plan's scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Run them.
    Run,
    /// Run none of them.
    Skip,
    /// Ask the user first.
    Ask,
}

/// Never or Blocked: Skip. Allowed: Run. Undecided: Ask when `needs`, else Run.
pub fn gate(setting: RunScripts, decision: Option<ScriptDecision>, needs: bool) -> Gate {
    if setting == RunScripts::Never {
        return Gate::Skip;
    }
    match decision {
        Some(ScriptDecision::Allowed) => Gate::Run,
        None if needs => Gate::Ask,
        None => Gate::Run,
        // Blocked, or a decision this build does not know, fails closed.
        Some(_) => Gate::Skip,
    }
}

/// For an undecided collection whose every script text is already approved (typed in the app),
/// `on` limited to the approved set so a script that shows up later cannot run; else `None`.
pub fn approved_only(
    hashes: &[String],
    approved: &BTreeSet<String>,
    on: ScriptOptions,
) -> Option<ScriptOptions> {
    let all = !hashes.is_empty() && hashes.iter().all(|h| approved.contains(h));
    all.then(|| on.only_approved(approved.clone()))
}

// The window the trust question opens in; tests and a backgrounded app have no active window.
struct ShellWindow(AnyWindowHandle);

impl Global for ShellWindow {}

/// Ask the trust question in `window`; `app::start` calls it for the shell window.
pub fn install(window: AnyWindowHandle, cx: &mut App) {
    cx.set_global(ShellWindow(window));
}

/// The window a send's dialogs open in: the shell window, else the active one, else any.
pub(crate) fn dialog_window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<ShellWindow>()
        .map(|w| w.0)
        .or_else(|| cx.active_window())
        .or_else(|| cx.windows().first().copied())
}

// Choice indices of `trust_question`.
const RUN: usize = 0;
const NEVER: usize = 1;

/// The trust question: what scripts can do, with Don't run (this send only) as both default
/// and cancel.
pub fn trust_question() -> Ask {
    trust_question_for(false)
}

/// The scratch pad's directory, so a send can tell a scratch draft from a collection's request.
pub struct ScratchDir(pub PathBuf);

impl Global for ScratchDir {}

/// `trust_question`, worded for a scratch draft when `scratch`: it belongs to no collection.
pub fn trust_question_for(scratch: bool) -> Ask {
    let (title, detail, never) = if scratch {
        (
            "Run scripts from this draft?",
            "These scripts run on your computer and can read your variables, including \
             secrets. Run them only if you trust where they came from.",
            "Never for the scratch pad",
        )
    } else {
        (
            "Run scripts from this collection?",
            "These scripts run on your computer and can read this collection's variables, \
             including secrets. Run them only if you trust where this collection came from.",
            "Never for this collection",
        )
    };
    Ask {
        title: title.into(),
        detail: Some(detail.into()),
        // The safe default is the primary, so it sits last, at the footer's right edge.
        choices: vec![
            Choice {
                id: "script-trust-run",
                label: "Run scripts".into(),
                danger: false,
            },
            Choice {
                id: "script-trust-never",
                label: never.into(),
                danger: false,
            },
            Choice {
                id: "script-trust-skip",
                label: "Don't run".into(),
                danger: false,
            },
        ],
        default: 2,
        cancel: 2,
    }
}

/// What the gate decided before anything is asked.
pub enum Prepared {
    /// Run with these options, asking nothing.
    Ready(ScriptOptions),
    /// Ask the trust question first.
    Ask(TrustQuestion),
}

/// The trust question still to ask; `ask` opens it and records the answer.
pub struct TrustQuestion {
    root: PathBuf,
    path: PathBuf,
    window: AnyWindowHandle,
    hashes: Vec<String>,
    off: ScriptOptions,
    on: ScriptOptions,
    scratch: bool,
}

/// Decides the script options for `planned` without asking. Without `AppState`, a script
/// trust path or a window to ask in, scripts stay off.
pub fn prepare(root: PathBuf, planned: &Planned, cx: &mut AsyncApp) -> Prepared {
    let Some((setting, secs, path, window, scratch)) = cx.update(|cx| {
        let state = cx.try_global::<AppState>()?;
        let settings = state.settings.read(cx).get();
        let (setting, secs) = (settings.run_scripts, settings.script_timeout_secs);
        let path = state.approvals.read(cx).script_trust()?.to_path_buf();
        let scratch = cx
            .try_global::<ScratchDir>()
            .is_some_and(|d| d.0 == crate::workspace::absolute(&root));
        Some((setting, secs, path, dialog_window(cx), scratch))
    }) else {
        return Prepared::Ready(ScriptOptions::new());
    };
    let off = limited(secs);
    let on = off.clone().enabled(true);
    // A store that fails to load still lets the user answer, but only for this send.
    let store = ScriptTrust::load(&path).ok();
    let decision = store.as_ref().and_then(|s| s.decision(&root));
    let texts = plan_script_texts(&planned.plan, &planned.context);
    match gate(setting, decision, !texts.is_empty()) {
        Gate::Run => return Prepared::Ready(on),
        Gate::Skip => return Prepared::Ready(off),
        Gate::Ask => {}
    }
    // Scripts the user typed here are all approved already: run them without asking, but only
    // them, and leave the decision undecided.
    let hashes: Vec<String> = texts.iter().map(|(_, t)| t.hash.clone()).collect();
    if let Some(store) = &store
        && let Some(opts) = approved_only(&hashes, &store.approved(&root), on.clone())
    {
        return Prepared::Ready(opts);
    }
    let Some(window) = window else {
        return Prepared::Ready(off);
    };
    Prepared::Ask(TrustQuestion {
        root,
        path,
        window,
        hashes,
        off,
        on,
        scratch,
    })
}

impl TrustQuestion {
    /// Asks in the shell window and records the answer; a dialog that goes away unanswered
    /// skips this send and records nothing, as does Don't run.
    pub async fn ask(self, cx: &mut AsyncApp) -> ScriptOptions {
        let TrustQuestion {
            root,
            path,
            window,
            hashes,
            off,
            on,
            scratch,
        } = self;
        let Ok(answer) = window.update(cx, |_, window, cx| {
            ask(trust_question_for(scratch), window, cx)
        }) else {
            return off;
        };
        let Ok(answer) = answer.recv().await else {
            return off;
        };
        let run = answer == RUN;
        // Only Run and Never store anything; Don't run and Escape skip this send alone.
        if run || answer == NEVER {
            record(&path, &root, run, hashes.iter().map(String::as_str));
        }
        // Views of the trust file follow the approvals model, so the answer shows at once.
        let stored = if run {
            Some(ScriptDecision::Allowed)
        } else if answer == NEVER {
            Some(ScriptDecision::Blocked)
        } else {
            None
        };
        cx.update(|cx| {
            if let Some(state) = cx.try_global::<AppState>() {
                let approvals = state.approvals.clone();
                approvals.update(cx, |a, cx| match stored {
                    Some(d) => a.note_script_decision(&root, Some(d), cx),
                    None => cx.notify(),
                });
            }
        });
        if run { on } else { off }
    }
}

// Updates under the trust file's lock so entries written elsewhere survive; a store that
// fails to load is never overwritten.
fn record<'a>(path: &Path, root: &Path, run: bool, hashes: impl Iterator<Item = &'a str>) {
    let _ = ScriptTrust::update(path, |store| {
        if run {
            store.set_decision(root, Some(ScriptDecision::Allowed));
            store.approve(root, hashes);
        } else {
            store.set_decision(root, Some(ScriptDecision::Blocked));
        }
    });
}

// Scripts off under the time limit `secs`. A hand-edited settings file may hold a limit the
// Preferences row would refuse, so it is clamped to that row's range.
fn limited(secs: u32) -> ScriptOptions {
    let (low, high) = (*SCRIPT_TIMEOUT_SECS.start(), *SCRIPT_TIMEOUT_SECS.end());
    ScriptOptions::new().timeout(Duration::from_secs(secs.clamp(low, high).into()))
}

/// Clears `root`'s Blocked decision so the next send asks again, and tells the approvals
/// model so every trust view follows; false if the write failed.
pub(crate) fn ask_again(approvals: &Entity<Approvals>, root: &Path, cx: &mut App) -> bool {
    let Some(path) = approvals.read(cx).script_trust().map(Path::to_path_buf) else {
        return false;
    };
    if ScriptTrust::update(&path, |store| store.set_decision(root, None)).is_err() {
        return false;
    }
    approvals.update(cx, |a, cx| a.note_script_decision(root, None, cx));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_out_of_range_limit_is_clamped() {
        let secs = |n: u64| ScriptOptions::new().timeout(Duration::from_secs(n));
        assert_eq!(limited(0), secs(1));
        assert_eq!(limited(u32::MAX), secs(3600));
        assert_eq!(limited(90), secs(90));
    }
}
