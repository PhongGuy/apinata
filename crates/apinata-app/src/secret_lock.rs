//! What is asked before a stored secret value would be lost: the rule, the prompts, and the
//! wait on both. The secret lock and a Save of a new secret row share them, in both editors.

use gpui_kit::{AsyncWindowContext, Task, Window};

use crate::confirm::{self, Ask};

/// The loss a toggle would cause, which the person answers before it happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockQuestion {
    /// The plain text would overwrite a different stored value.
    Replace,
    /// The stored value would leave the store for the environment's file.
    Move,
}

/// What to ask before setting a variable's lock to `secret`, given the row's `plain` text and
/// the `stored` value; `None` when the toggle loses nothing.
pub fn question(secret: bool, plain: &str, stored: Option<&str>) -> Option<LockQuestion> {
    let stored = stored.filter(|s| !s.is_empty())?;
    match secret {
        true => (!plain.is_empty() && plain != stored).then_some(LockQuestion::Replace),
        false => Some(LockQuestion::Move),
    }
}

/// The names in `typing` a Save would overwrite, given each one's `stored` value in order.
pub fn replaced(typing: &[(String, String)], stored: &[Option<String>]) -> Vec<String> {
    let lost = |((name, text), stored): (&(String, String), &Option<String>)| {
        let asks = question(true, text, stored.as_deref()).is_some();
        asks.then(|| name.clone())
    };
    typing.iter().zip(stored).filter_map(lost).collect()
}

/// The prompt for `question` about variable `name`.
pub fn lock_ask(question: LockQuestion, name: &str) -> Ask {
    match question {
        LockQuestion::Replace => confirm::replace_secret_ask(std::slice::from_ref(&name.into())),
        LockQuestion::Move => confirm::move_secret_ask(name),
    }
}

/// Where a lock check stands once its read has answered.
pub enum LockStep {
    /// The read failed or the check no longer applies.
    Stop,
    /// Nothing to ask.
    Go,
    /// The prompt is open; its confirming button is index 1.
    Ask(async_channel::Receiver<usize>),
}

impl LockStep {
    /// Whether the toggle goes ahead; a dialog gone unanswered counts as no.
    pub async fn confirmed(self) -> bool {
        match self {
            LockStep::Stop => false,
            LockStep::Go => true,
            LockStep::Ask(answer) => answer.recv().await == Ok(1),
        }
    }
}

/// How a check of what a Save would replace came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceAnswer {
    /// No stored value would be replaced, so nothing was asked.
    Nothing,
    /// The person agreed to replace the values named.
    Replace,
    /// A read failed, or the prompt was cancelled or went unanswered.
    Stop,
}

/// Whether typing in the twin rows `names` may be dropped; true at once when there is none.
pub async fn twins_dropped(names: Vec<String>, cx: &mut AsyncWindowContext) -> bool {
    if names.is_empty() {
        return true;
    }
    let spec = confirm::discard_twin_ask(&names);
    let opened = cx.update(|window: &mut Window, cx| confirm::ask(spec, window, cx));
    matches!(opened, Ok(answer) if answer.recv().await == Ok(1))
}

/// Whether a Save may go ahead once `reads` have named what it would replace: `None` from a
/// read means it failed and has said so. All the names share one prompt.
pub async fn replace_agreed(
    reads: Vec<Task<Option<Vec<String>>>>,
    cx: &mut AsyncWindowContext,
) -> bool {
    replace_answer(reads, cx).await != ReplaceAnswer::Stop
}

/// [`replace_agreed`], telling apart a Save that asked from one with nothing to ask.
pub async fn replace_answer(
    reads: Vec<Task<Option<Vec<String>>>>,
    cx: &mut AsyncWindowContext,
) -> ReplaceAnswer {
    let mut names = Vec::new();
    for read in reads {
        match read.await {
            Some(found) => names.extend(found),
            None => return ReplaceAnswer::Stop,
        }
    }
    names.sort();
    names.dedup();
    if names.is_empty() {
        return ReplaceAnswer::Nothing;
    }
    let spec = confirm::replace_secret_ask(&names);
    let opened = cx.update(|window: &mut Window, cx| confirm::ask(spec, window, cx));
    match opened {
        Ok(answer) if answer.recv().await == Ok(1) => ReplaceAnswer::Replace,
        _ => ReplaceAnswer::Stop,
    }
}
