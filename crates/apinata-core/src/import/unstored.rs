//! Secrets an import could not store because the secret store failed, reported as one note in
//! plain words; the store's own error text never reaches the user.

use super::{ImportNote, NoteKind};
use crate::secrets::SecretError;

/// The secrets a failing store left without a value, and whether it failed by not answering.
#[derive(Debug, Clone, Default)]
pub(crate) struct Unstored {
    names: Vec<String>,
    no_answer: bool,
}

impl Unstored {
    fn absorb(&mut self, other: &Unstored) {
        for name in &other.names {
            if !self.names.contains(name) {
                self.names.push(name.clone());
            }
        }
        self.no_answer |= other.no_answer;
    }

    fn detail(&self) -> String {
        let one = self.names.len() == 1;
        let (noun, them) = if one {
            ("secret", "it")
        } else {
            ("secrets", "them")
        };
        let names = listed(&self.names);
        let what = if self.no_answer {
            let verb = if one { "was" } else { "were" };
            format!("the keychain did not answer, so the {noun} {names} {verb} not stored")
        } else {
            format!("the keychain could not store the {noun} {names}")
        };
        format!("{what}; set {them} with `--env-var`, in `.env` or in the app's environment editor")
    }
}

/// The first three names in backticks, then how many more there are.
fn listed(names: &[String]) -> String {
    let mut shown: Vec<String> = names.iter().take(3).map(|n| format!("`{n}`")).collect();
    if names.len() > 3 {
        shown.push(format!("{} more", names.len() - 3));
    }
    match shown.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Mark `note` as saying that the store failed to keep `name`; [`fold`] later replaces it.
pub(crate) fn mark(note: &mut ImportNote, name: &str, error: &SecretError) {
    note.unstored = Some(Unstored {
        names: vec![name.to_owned()],
        no_answer: matches!(error, SecretError::Unavailable { .. }),
    });
}

/// A `NeedsInput` note from `source` saying the store failed to keep `name`.
pub(crate) fn note(source: impl Into<String>, name: &str, error: &SecretError) -> ImportNote {
    let mut note = ImportNote::new(source, NoteKind::NeedsInput, String::new());
    mark(&mut note, name, error);
    note
}

/// Replace every marked note with one note naming all their secrets, at the first one's place,
/// and take those secrets out of notes claiming them stored, dropping a claim left empty.
/// Folding a report that already holds a summary merges it with the rest.
pub(crate) fn fold(notes: &mut Vec<ImportNote>) {
    let mut failed = Unstored::default();
    let mut marked = Vec::new();
    for note in notes.iter() {
        if let Some(unstored) = &note.unstored {
            failed.absorb(unstored);
            marked.push(note);
        }
    }
    let Some(first) = marked.first() else {
        return;
    };
    // The summary keeps a source or file only when every folded note agrees on it.
    let source = match marked.iter().all(|n| n.source == first.source) {
        true => first.source.clone(),
        false => "secrets".to_owned(),
    };
    let mut note = ImportNote::new(source, NoteKind::NeedsInput, failed.detail());
    note.file = first
        .file
        .clone()
        .filter(|_| marked.iter().all(|n| n.file == first.file));
    note.unstored = Some(failed.clone());
    let mut summary = Some(note);
    let mut folded = Vec::with_capacity(notes.len());
    for mut note in notes.drain(..) {
        if note.unstored.is_some() {
            folded.extend(summary.take());
            continue;
        }
        if !note.claims_stored.is_empty() {
            note.claims_stored.retain(|n| !failed.names.contains(n));
            if note.claims_stored.is_empty() {
                continue;
            }
            note.detail = super::protect::stored_claim(&note.claims_stored);
        }
        folded.push(note);
    }
    *notes = folded;
}
