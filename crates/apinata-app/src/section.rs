//! The event every request editor section emits.

use crate::command::{Edit, NoticeKind};

/// An edit a section made; the request editor sends it to the collection.
#[derive(Clone)]
pub struct SectionEdit {
    /// The new value of the field.
    pub edit: Edit,
    /// The `(row, column)` of the cell typed in, when a keystroke caused the edit.
    pub typed: Option<(usize, u8)>,
    /// Whether the edit came from table rows a reload has since replaced.
    pub lagging: bool,
    /// How many loads the section had taken when it built the edit, for a section whose
    /// `lags` asks whether a load came between the edit and its delivery; 0 otherwise.
    pub loads: u64,
}

impl SectionEdit {
    /// An edit not caused by typing in a cell.
    pub fn new(edit: Edit) -> Self {
        SectionEdit {
            edit,
            typed: None,
            lagging: false,
            loads: 0,
        }
    }

    /// This edit, built after the section's `loads`th load.
    pub fn after_loads(mut self, loads: u64) -> Self {
        self.loads = loads;
        self
    }
}

/// The text a section shows for a notice on its own edit: a refusal's message or the
/// warnings joined; other notices show nothing there.
pub(crate) fn notice_text(kind: &NoticeKind) -> Option<String> {
    match kind {
        NoticeKind::Failed { message, .. } => Some(message.clone()),
        NoticeKind::Warnings(warnings) => Some(warnings.join("; ")),
        _ => None,
    }
}
