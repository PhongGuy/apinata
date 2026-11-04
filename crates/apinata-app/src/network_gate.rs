//! Noticing when the network settings a send uses changed since the person saw them: an
//! outside edit (a pull, an editor, an agent) or a folder, request or run variable that sends
//! a request elsewhere. The Network section shows the collection's change, and core asks
//! through [`check`] before each send, judged from the settings in memory that the send uses.
//! Edits made in the app are acknowledged as they are saved. No credential or secret value is
//! ever stored or shown.

use std::path::{Path, PathBuf};

use apinata_core::{
    AckState, NetworkAck, NetworkAnswer, NetworkChange, NetworkCheck, NetworkQuestion,
    NetworkSnapshot, QuestionKind, ShownNetwork, saved_network,
};
use gpui_kit::{App, AsyncApp, Global, WeakEntity};

use crate::command::{Command, NetworkOp};
use crate::confirm::{Ask, Choice, ask};
use crate::script_gate::dialog_window;
use crate::workspace::Workspace;

// Choice indices of `change_question`.
const CONTINUE: usize = 0;

/// Builds the snapshot to compare, given the salt of the record it is compared with.
pub type Make<'a> = &'a dyn Fn(&str) -> NetworkSnapshot;

struct AckFile(PathBuf);

impl Global for AckFile {}

// The workspace whose Network sections look again once a question was answered.
struct NoticeWorkspace(WeakEntity<Workspace>);

impl Global for NoticeWorkspace {}

/// Names the acknowledgement file; `app::start` calls it before the executor is built.
/// Without it no send is checked.
pub fn install(path: PathBuf, cx: &mut App) {
    cx.set_global(AckFile(path));
}

/// The workspace to tell when an answer recorded settings; `app::start` calls it.
pub fn attach(workspace: WeakEntity<Workspace>, cx: &mut App) {
    cx.set_global(NoticeWorkspace(workspace));
}

/// The acknowledgement file, once installed.
pub fn path(cx: &App) -> Option<PathBuf> {
    cx.try_global::<AckFile>().map(|f| f.0.clone())
}

/// What the Network section tells the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// True when the settings changed since they were acknowledged; false when the collection
    /// was never seen here and merely sets something.
    pub changed: bool,
    /// True when the record of what was seen cannot be read, so the person may reset it.
    pub unreadable: bool,
    /// One line per setting or change.
    pub lines: Vec<String>,
    /// What the notice shows, so Acknowledge records exactly that.
    pub shown: Option<ShownNetwork>,
}

fn lines(changes: &[NetworkChange]) -> Vec<String> {
    changes.iter().map(NetworkChange::describe).collect()
}

fn notice_of(state: AckState, shown: Option<ShownNetwork>) -> Option<Notice> {
    match state {
        AckState::Changed(changes) => Some(Notice {
            changed: true,
            unreadable: changes.contains(&NetworkChange::RecordUnreadable),
            lines: lines(&changes),
            shown,
        }),
        AckState::FirstSeen(changes) if !changes.is_empty() => Some(Notice {
            changed: false,
            unreadable: false,
            lines: lines(&changes),
            shown,
        }),
        _ => None,
    }
}

/// The notice for `root` under the settings `make` builds, or `None` when there is nothing to
/// tell. Records a baseline for a collection that sets nothing.
pub fn notice_for(file: &Path, root: &Path, make: Make) -> Option<Notice> {
    let (state, shown) = NetworkAck::check_shown(file, root, make);
    notice_of(state, shown)
}

/// Records the snapshot `make` builds as seen for `root`.
pub fn acknowledge(file: &Path, root: &Path, make: Make) {
    let _ = NetworkAck::update(file, |ack| {
        let snapshot = make(ack.salt());
        ack.acknowledge_snapshot(root, &snapshot);
    });
}

/// Records what a notice showed for `root`, never a later state.
pub fn acknowledge_shown(file: &Path, root: &Path, shown: &ShownNetwork) {
    let _ = NetworkAck::update(file, |ack| ack.acknowledge_shown(root, shown));
}

/// Moves an unreadable record aside as `<name>.bad`, so a new one starts.
pub fn reset(file: &Path) {
    let _ = NetworkAck::reset_unreadable(file);
}

/// Whether a save of `root`'s settings may be acknowledged as the person's own: the file
/// on disk, as `make` builds it from the state before the save, is what they last saw (or sets
/// nothing). An unacknowledged outside change stays flagged through an in-app save.
pub fn may_acknowledge_save(file: &Path, root: &Path, make: Make) -> bool {
    NetworkAck::check_shown(file, root, make).0 == AckState::Unchanged
}

/// The settings of `root` as saved on disk now, for a caller that resolves them itself.
pub fn saved(root: &Path) -> Option<apinata_core::CollectionNetwork> {
    saved_network(root).ok()
}

/// The question before a send when the settings it uses changed: Cancel is the default and
/// the escape key.
pub fn change_question(lines: &[String]) -> Ask {
    let list: Vec<String> = lines.iter().map(|l| format!("- {l}")).collect();
    Ask {
        title: "Network settings changed".into(),
        detail: Some(
            format!(
                "Changed since you last saw them:\n{}\n\nThey decide where this request goes \
                 and which proxy or certificate it uses.",
                list.join("\n")
            )
            .into(),
        ),
        choices: vec![
            Choice {
                id: "network-change-continue",
                label: "Send anyway".into(),
                danger: false,
            },
            Choice {
                id: "network-change-cancel",
                label: "Cancel".into(),
                danger: false,
            },
        ],
        default: 1,
        cancel: 1,
    }
}

type Asked = (NetworkQuestion, async_channel::Sender<NetworkAnswer>);

/// The check every send of the app's executors runs, or `None` before [`install`]. Its
/// questions are answered on the UI thread: a first sight only informs and goes ahead
/// unrecorded; a change opens the question in the shell window, where Send anyway records what
/// it showed (here, before the Network section looks again) and Cancel (or a dialog that goes
/// away) stops that send. With no window to ask in, a change is refused.
pub fn check(cx: &mut App) -> Option<NetworkCheck> {
    let file = path(cx)?;
    let (questions, inbox) = async_channel::unbounded::<Asked>();
    // The loop ends when the executor holding the sender is replaced and dropped.
    cx.spawn(async move |cx| {
        while let Ok((question, reply)) = inbox.recv().await {
            let answer = answer(&question, cx).await;
            let _ = reply.try_send(answer);
        }
    })
    .detach();
    Some(NetworkCheck::new(file, move |question| {
        let questions = questions.clone();
        async move {
            let (reply, answer) = async_channel::bounded(1);
            if questions.send((question, reply)).await.is_err() {
                return NetworkAnswer::Cancel;
            }
            answer.recv().await.unwrap_or(NetworkAnswer::Cancel)
        }
    }))
}

// Shows `question` and waits for the person.
async fn answer(question: &NetworkQuestion, cx: &mut AsyncApp) -> NetworkAnswer {
    if question.kind() == QuestionKind::FirstSeen {
        return NetworkAnswer::SendOnce;
    }
    // A change nobody can be asked about is refused, never sent unseen.
    let Some(window) = cx.update(|cx| dialog_window(cx)) else {
        return NetworkAnswer::Cancel;
    };
    let spec = change_question(&question.lines());
    let Ok(choice) = window.update(cx, |_, window, cx| ask(spec, window, cx)) else {
        return NetworkAnswer::Cancel;
    };
    if choice.recv().await.ok() != Some(CONTINUE) {
        return NetworkAnswer::Cancel;
    }
    // Recorded here rather than by core, so the Network section looks again after the write.
    let root = question.root().to_path_buf();
    cx.update(|cx| {
        if let Some(file) = path(cx) {
            acknowledge_shown(&file, &root, question.shown());
        }
        let workspace = cx.try_global::<NoticeWorkspace>().map(|w| w.0.clone());
        if let Some(workspace) = workspace.and_then(|w| w.upgrade()) {
            workspace.update(cx, |w, cx| {
                w.send(&root, Command::Network(NetworkOp::Recheck), cx)
            });
        }
    });
    NetworkAnswer::SendOnce
}
