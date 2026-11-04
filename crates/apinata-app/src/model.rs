//! The GPUI entity that fronts one collection actor: it numbers commands, tracks which are
//! not yet acknowledged, and holds the newest snapshot.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use apinata_core::{EnvOverlay, RuntimeVars, SecretStore};
use async_channel::Sender;
use gpui_kit::{App, AppContext, Context, Entity, Task};

use crate::actor::{Faults, Update, run_actor};
use crate::command::{Command, Envelope, Field, Notice, NoticeKind, Target};
use crate::prompt_vars::PromptMemory;
use crate::snapshot::CollectionSnapshot;

/// The notice for any command after a panic stopped the actor.
pub const STOPPED_MESSAGE: &str =
    "The collection stopped after an internal error; close and reopen it.";
/// The notice for any command after the collection failed to open.
pub const NOT_OPEN_MESSAGE: &str = "the collection did not open";
/// The open error when opening panicked.
pub const OPEN_PANIC_MESSAGE: &str = "an internal error stopped the open";

/// The request fields an `ApplyImport` rewrites, each held in flight until it lands.
const IMPORTED_FIELDS: [Field; 7] = [
    Field::Method,
    Field::UrlParams,
    Field::Headers,
    Field::Body,
    Field::Auth,
    Field::Docs,
    Field::Settings,
];

/// One open collection as the UI sees it.
pub struct CollectionModel {
    sender: Sender<Envelope>,
    last_id: u64,
    snapshot: Option<Arc<CollectionSnapshot>>,
    open_error: Option<String>,
    /// The open failed because the root is in the legacy .bru format.
    legacy: bool,
    /// A panic stopped the actor.
    stopped: bool,
    received: u64,
    notices: Vec<Notice>,
    /// Ids of sent commands other than `Plan`, `SecretValues`, `Search`, `RevealSecret` and the
    /// orphan commands that no snapshot has acknowledged yet.
    unacked: VecDeque<u64>,
    /// The last command id that wrote each field, while above `acked`.
    fields: HashMap<(Target, Field), u64>,
    /// Post-response captures for this open, shared with the actor's contexts.
    runtime_vars: RuntimeVars,
    /// Prompt answers given in this open this app session, offered again on the next ask.
    prompt_memory: PromptMemory,
    /// Script-set environment and collection variables, shared the same way.
    env_overlay: EnvOverlay,
    _actor: Task<()>,
    _pump: Task<()>,
}

impl CollectionModel {
    /// Start an actor for the collection at `root`. Dropping the entity ends the actor.
    pub fn open(root: PathBuf, store: Arc<dyn SecretStore>, cx: &mut App) -> Entity<Self> {
        Self::open_with_faults(root, store, Faults::default(), cx)
    }

    /// `open` with test-only fault injection on the actor.
    #[doc(hidden)]
    pub fn open_with_faults(
        root: PathBuf,
        store: Arc<dyn SecretStore>,
        faults: Faults,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx: &mut Context<Self>| {
            let (sender, commands) = async_channel::unbounded();
            let (updates, inbox) = async_channel::unbounded();
            let runtime_vars = RuntimeVars::new();
            let env_overlay = EnvOverlay::new();
            let tokens = crate::oauth2_tokens::shared(cx);
            let network_ack = crate::network_gate::path(cx);
            let actor = cx.background_spawn(run_actor(
                root,
                store,
                commands,
                updates,
                faults,
                runtime_vars.clone(),
                env_overlay.clone(),
                tokens,
                network_ack,
            ));
            // Take every update that has queued in one entity update, so a backlog costs one
            // notify; applying them in order keeps every notice and ends on the newest snapshot.
            let pump = cx.spawn(async move |this, cx| {
                while let Ok(first) = inbox.recv().await {
                    let mut batch = vec![first];
                    while let Ok(next) = inbox.try_recv() {
                        batch.push(next);
                    }
                    let alive = this.update(cx, |m, cx| {
                        batch.into_iter().for_each(|u| m.receive(u));
                        cx.notify();
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            });
            CollectionModel {
                sender,
                last_id: 0,
                snapshot: None,
                open_error: None,
                legacy: false,
                stopped: false,
                received: 0,
                notices: Vec::new(),
                unacked: VecDeque::new(),
                fields: HashMap::new(),
                runtime_vars,
                prompt_memory: PromptMemory::default(),
                env_overlay,
                _actor: actor,
                _pump: pump,
            }
        })
    }

    /// The runtime variables this open collection's sends capture into and read from.
    pub fn runtime_vars(&self) -> RuntimeVars {
        self.runtime_vars.clone()
    }

    /// The prompt answers given in this open collection this app session.
    pub fn prompt_memory(&self) -> PromptMemory {
        self.prompt_memory.clone()
    }

    /// The environment and collection variables this open collection's scripts set.
    pub fn env_overlay(&self) -> EnvOverlay {
        self.env_overlay.clone()
    }

    /// Queue `command` for the actor and return its id, stamped with the newest snapshot's
    /// sequence (0 before the first). A stopped actor turns it into a `Failed` notice.
    pub fn send(&mut self, command: Command) -> u64 {
        self.send_from(command, None)
    }

    /// As [`send`](Self::send), for content built from the snapshot `basis` when that lags
    /// the newest; the actor holds such an edit back if a reload came after `basis`.
    pub fn send_from(&mut self, command: Command, basis: Option<u64>) -> u64 {
        self.last_id += 1;
        let id = self.last_id;
        let seen = self.snapshot.as_ref().map_or(0, |s| s.seq);
        let basis = basis.map_or(seen, |b| b.min(seen));
        let counted = !matches!(
            command,
            Command::Plan { .. }
                | Command::PlanSaved { .. }
                | Command::PlanSavedItems { .. }
                | Command::SecretValues { .. }
                | Command::Search { .. }
                | Command::CopyOut { .. }
                | Command::RevealSecret { .. }
                | Command::StoredSecret { .. }
                | Command::OrphanedSecrets { .. }
                | Command::RemoveOrphanedSecrets { .. }
        );
        let fields = match &command {
            Command::Edit { target, edit } => vec![(target.clone(), edit.field())],
            Command::ApplyImport { item, .. } => IMPORTED_FIELDS
                .iter()
                .map(|f| (Target::Request(item.clone()), *f))
                .collect(),
            _ => Vec::new(),
        };
        // An actor that stopped or never opened will apply nothing, so refuse locally and
        // count nothing as pending.
        let refusal = if self.stopped {
            Some(STOPPED_MESSAGE)
        } else if self.open_error.is_some() {
            Some(NOT_OPEN_MESSAGE)
        } else {
            None
        };
        if let Some(message) = refusal {
            self.fail(id, message);
            return id;
        }
        if self
            .sender
            .try_send(Envelope {
                id,
                seen,
                basis,
                command,
            })
            .is_err()
        {
            self.fail(id, "the collection is closed");
            return id;
        }
        if counted {
            self.unacked.push_back(id);
        }
        for key in fields {
            self.fields.insert(key, id);
        }
        id
    }

    /// The newest snapshot; `None` until the collection opens.
    pub fn snapshot(&self) -> Option<&Arc<CollectionSnapshot>> {
        self.snapshot.as_ref()
    }

    /// Why the collection failed to open.
    pub fn open_error(&self) -> Option<&str> {
        self.open_error.as_deref()
    }

    /// Whether the open failed because the root is in the legacy .bru format.
    pub fn is_legacy(&self) -> bool {
        self.legacy
    }

    /// Whether a panic stopped the actor.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Whether a sent edit to `field` of `target` is not yet in a snapshot.
    pub fn in_flight(&self, target: &Target, field: Field) -> bool {
        self.fields.contains_key(&(target.clone(), field))
    }

    /// How many commands other than `Plan`, `SecretValues`, `Search`, `RevealSecret` and the
    /// orphan commands await acknowledgement; 0 after an open failure.
    pub fn pending(&self) -> u64 {
        self.unacked.len() as u64
    }

    /// How many snapshots have arrived, the opening one included.
    pub fn snapshots_received(&self) -> u64 {
        self.received
    }

    /// The notices received since the last call, in command order.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    fn fail(&mut self, command: u64, message: &str) {
        self.notices.push(Notice {
            command,
            kind: NoticeKind::Failed {
                target: None,
                message: message.into(),
                row: None,
            },
        });
    }

    fn refuse(&mut self, error: String) {
        self.open_error = Some(error);
        self.unacked.clear();
        self.fields.clear();
    }

    fn receive(&mut self, update: Update) {
        match update {
            Update::Snapshot(s) => {
                self.received += 1;
                self.notices.extend(s.notices.iter().cloned());
                let acked = s.acked;
                while self.unacked.front().is_some_and(|&id| id <= acked) {
                    self.unacked.pop_front();
                }
                self.fields.retain(|_, &mut id| id > acked);
                self.snapshot = Some(s);
            }
            // Nothing queued will ever be applied after any of these, so nothing is pending.
            // A legacy root also sets the open error, so every refusal path still holds.
            Update::OpenFailed(e) => self.refuse(e),
            Update::LegacyFormat(e) => {
                self.legacy = true;
                self.refuse(e);
            }
            Update::Stopped => {
                self.stopped = true;
                self.unacked.clear();
                self.fields.clear();
                self.fail(0, STOPPED_MESSAGE);
            }
        }
    }
}
