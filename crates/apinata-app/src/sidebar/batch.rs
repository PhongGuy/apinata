//! Tracks the commands of one multi-item delete, move, paste or undo, so the sidebar can show
//! a single summary toast instead of one per item.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use apinata_core::ItemPath;

use crate::command::{Notice, NoticeKind};
use crate::confirm::{Ask, delete_ask};
use crate::ui::ToastKind;

/// What a batch does to each of its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Every item is deleted.
    Delete,
    /// Every item is moved.
    Move,
    /// Every item is pasted as a copy.
    Paste,
    /// Every item comes back from the trash.
    Restore,
}

/// The commands one gesture sent to one collection and how each ended.
pub struct Batch {
    verb: Verb,
    root: PathBuf,
    // The open generation of `root` the commands went to; only its notices answer them.
    generation: u64,
    items: BTreeMap<u64, ItemPath>,
    outcomes: BTreeMap<u64, Result<(), String>>,
    // Side-effect warnings of tracked commands, each text once however many commands raised it.
    warnings: Vec<String>,
    // The trash tickets of tracked commands that moved their item to the trash.
    tickets: Vec<u64>,
    // Pasted commands whose source had unsaved edits the copy left out.
    unsaved: Vec<u64>,
    // Whether the paste was a cut, which leaves an unsaved source in place.
    cut: bool,
    // Whether the commands move their items to the trash.
    to_trash: bool,
    // Whether a Ctrl+Z pressed before the batch finished waits to undo it.
    undo_owed: bool,
}

impl Batch {
    /// A batch of `verb` over the commands `sent` (id and item) to the instance `generation`
    /// of the collection at `root`.
    pub fn new(verb: Verb, root: PathBuf, generation: u64, sent: Vec<(u64, ItemPath)>) -> Batch {
        Batch {
            verb,
            root,
            generation,
            items: sent.into_iter().collect(),
            outcomes: BTreeMap::new(),
            warnings: Vec::new(),
            tickets: Vec::new(),
            unsaved: Vec::new(),
            cut: false,
            to_trash: false,
            undo_owed: false,
        }
    }

    /// Notes the commands of `ids` as pastes of a source with unsaved edits; `cut` says the
    /// sources stay in place.
    pub fn note_unsaved(&mut self, ids: Vec<u64>, cut: bool) {
        self.unsaved = ids;
        self.cut = cut;
    }

    /// The notice for the pastes of unsaved sources that succeeded, or None when none did.
    pub fn unsaved_notice(&self) -> Option<String> {
        let pasted = (self.unsaved.iter())
            .filter(|id| matches!(self.outcomes.get(id), Some(Ok(()))))
            .count();
        if pasted == 0 {
            return None;
        }
        let tail = if self.cut {
            "unsaved edits were not copied and the original was left in place."
        } else {
            "unsaved edits were not copied."
        };
        let noun = if pasted == 1 { "item" } else { "items" };
        Some(format!(
            "Pasted the saved version of {pasted} {noun}; {tail}"
        ))
    }

    /// Marks the batch as deletes into the trash, which an undo can take back.
    pub fn into_trash(mut self) -> Batch {
        self.to_trash = true;
        self
    }

    /// Whether a Ctrl+Z can still claim this batch: a delete into the trash that has not
    /// reported, and that no earlier press claimed.
    pub fn awaits_undo(&self) -> bool {
        self.to_trash && !self.undo_owed
    }

    /// Asks for the batch to be undone as soon as it finishes.
    pub fn owe_undo(&mut self) {
        self.undo_owed = true;
    }

    /// Whether a Ctrl+Z claimed the batch while it was still landing.
    pub fn undo_owed(&self) -> bool {
        self.undo_owed
    }

    /// The tickets that undo this batch's deletes, in the order the notices came.
    pub fn tickets(&self) -> &[u64] {
        &self.tickets
    }

    /// The Undo toast's text: the item's name, or a count when there are several.
    pub fn trashed_message(&self) -> String {
        match self.tickets.as_slice() {
            [ticket] => {
                let name = self.items.get(ticket).map_or("item", |i| i.file_name());
                format!("Deleted \"{}\"", name.strip_suffix(".yml").unwrap_or(name))
            }
            all => format!("Deleted {}", items_label(all.len())),
        }
    }

    /// The trashed items with the tickets that restore them, in the order the notices came.
    pub fn trashed(&self) -> Vec<(u64, ItemPath)> {
        (self.tickets.iter())
            .filter_map(|t| Some((*t, self.items.get(t)?.clone())))
            .collect()
    }

    /// The toast for a finished restore where every item came back, or None otherwise; a
    /// failure is told by `summary` instead.
    pub fn restored_message(&self) -> Option<String> {
        let clean = self.outcomes.values().all(Result::is_ok);
        if self.verb != Verb::Restore || self.items.is_empty() || !self.is_finished() || !clean {
            return None;
        }
        Some(match self.items.values().collect::<Vec<_>>().as_slice() {
            [item] => {
                let name = item.file_name();
                format!("Restored \"{}\"", name.strip_suffix(".yml").unwrap_or(name))
            }
            all => format!("Restored {}", items_label(all.len())),
        })
    }

    /// The collection the batch was sent to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The open generation of the collection the batch was sent to.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether command `id` of the instance `generation` of `root` is one of this batch's.
    pub fn tracks(&self, root: &Path, generation: u64, id: u64) -> bool {
        root == self.root && generation == self.generation && self.items.contains_key(&id)
    }

    /// Notes the notice when it is about a tracked command of the instance `generation` of
    /// `root`; true when it was.
    pub fn record(&mut self, root: &Path, generation: u64, notice: &Notice) -> bool {
        if root != self.root
            || generation != self.generation
            || !self.items.contains_key(&notice.command)
        {
            return false;
        }
        // A warning does not decide the command's outcome: its own notice does.
        if let NoticeKind::Warnings(texts) = &notice.kind {
            for text in texts {
                if !self.warnings.contains(text) {
                    self.warnings.push(text.clone());
                }
            }
            return true;
        }
        if let NoticeKind::Trashed { ticket, .. } = &notice.kind {
            self.tickets.push(*ticket);
        }
        let outcome = match &notice.kind {
            NoticeKind::Deleted(_) | NoticeKind::Trashed { .. } | NoticeKind::Moved { .. } => {
                Ok(())
            }
            NoticeKind::Created(_) if self.verb == Verb::Paste => Ok(()),
            NoticeKind::Restored(_) if self.verb == Verb::Restore => Ok(()),
            NoticeKind::Failed { message, .. } => Err(message.clone()),
            NoticeKind::NotFound(_) => Err("it no longer exists".to_owned()),
            NoticeKind::Stale(_) => Err("it changed on disk".to_owned()),
            _ => return false,
        };
        self.outcomes.insert(notice.command, outcome);
        true
    }

    /// Counts tracked commands up to `acked` that produced no notice as done, except a
    /// restore: its silence means the ticket was unknown, so the item did not come back.
    pub fn settle(&mut self, acked: u64) {
        let silent = match self.verb {
            Verb::Restore => Err("it is no longer in the trash".to_owned()),
            _ => Ok(()),
        };
        for id in self.items.keys().filter(|id| **id <= acked) {
            self.outcomes.entry(*id).or_insert_with(|| silent.clone());
        }
    }

    /// Whether every tracked command has an outcome.
    pub fn is_finished(&self) -> bool {
        self.outcomes.len() == self.items.len()
    }

    /// The toast kind and text, or None when nothing failed and nothing warned. A batch where
    /// nothing was done is an error; a partial one, or one with only side-effect warnings, is a
    /// warning.
    pub fn summary(&self) -> Option<(ToastKind, String)> {
        let failed: Vec<String> = self
            .outcomes
            .iter()
            .filter_map(|(id, o)| {
                let why = o.as_ref().err()?;
                Some(format!("{} ({why})", self.items[id].file_name()))
            })
            .collect();
        if failed.is_empty() {
            return (!self.warnings.is_empty())
                .then(|| (ToastKind::Warning, self.warnings.join(". ")));
        }
        let (past, bare) = match self.verb {
            Verb::Delete => ("Deleted", "delete"),
            Verb::Move => ("Moved", "move"),
            Verb::Paste => ("Pasted", "paste"),
            Verb::Restore => ("Restored", "restore"),
        };
        let done = self.outcomes.len() - failed.len();
        let mut text = format!(
            "{past} {done} of {}. Could not {bare}: {}",
            self.items.len(),
            failed.join(", ")
        );
        for warning in &self.warnings {
            text.push_str(". ");
            text.push_str(warning);
        }
        let kind = if done == 0 {
            ToastKind::Error
        } else {
            ToastKind::Warning
        };
        Some((kind, text))
    }
}

/// The Undo toast's text for items removed by something other than a sidebar delete, such as
/// a spec update: the item's name, or a count when there are several.
pub fn removed_message(trashed: &[(u64, ItemPath)]) -> String {
    match trashed {
        [(_, item)] => {
            let name = item.file_name();
            format!("Removed \"{}\"", name.strip_suffix(".yml").unwrap_or(name))
        }
        all => format!("Removed {}", items_label(all.len())),
    }
}

/// "1 item" or "5 items".
pub fn items_label(n: usize) -> String {
    if n == 1 {
        "1 item".to_owned()
    } else {
        format!("{n} items")
    }
}

/// The detail line shared by the bulk prompts, before any permanence note.
fn bulk_detail(folder: bool, dirty: bool) -> Option<&'static str> {
    match (folder, dirty) {
        (true, true) => {
            Some("Folders are deleted with everything inside them. Unsaved edits are lost.")
        }
        (true, false) => Some("Folders are deleted with everything inside them."),
        (false, true) => Some("Unsaved edits are lost."),
        (false, false) => None,
    }
}

/// The prompt for deleting one row. `permanent` says the delete cannot be undone, as it is
/// when there is no trash.
pub fn single_delete_ask(name: &str, folder: bool, permanent: bool) -> Ask {
    let inside = folder.then_some("Everything inside it is deleted too.");
    let mut ask = delete_ask(format!("Delete \"{name}\"?"), inside);
    if permanent {
        ask.detail = Some(match inside {
            Some(text) => format!("{text} This is permanent.").into(),
            None => "This is permanent.".into(),
        });
    }
    ask
}

/// `bulk_delete_ask` for when there is no trash: the detail ends by saying it is permanent.
pub fn permanent_delete_ask(count: usize, folder: bool, dirty: bool) -> Ask {
    let mut ask = bulk_delete_ask(count, folder, dirty);
    ask.detail = Some(match bulk_detail(folder, dirty) {
        Some(text) => format!("{text} This is permanent.").into(),
        None => "This is permanent.".into(),
    });
    ask
}

/// The delete prompt for a multi-selection of `count` items; the folder sentence appears only
/// when the set holds a `folder`.
pub fn bulk_delete_ask(count: usize, folder: bool, dirty: bool) -> Ask {
    delete_ask(
        format!("Delete {}?", items_label(count)),
        bulk_detail(folder, dirty),
    )
}
