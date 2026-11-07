//! A titled group of labelled rows in Preferences that the search box can filter.

use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, SharedString, TestSupportExt};

use crate::style::SPACE_2;
use crate::ui;

/// True when `query`, trimmed, is empty or a case-insensitive substring of `text`.
pub fn matches(query: &str, text: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty() || text.to_lowercase().contains(&query)
}

struct Row {
    id: SharedString,
    label: SharedString,
    terms: Vec<SharedString>,
    control: AnyElement,
    note: Option<AnyElement>,
}

/// A titled group of labelled rows in Preferences, filtered by the search box.
pub struct PrefsSection {
    id: SharedString,
    title: SharedString,
    rows: Vec<Row>,
    notes: Vec<AnyElement>,
}

impl PrefsSection {
    /// An empty section; its heading's id is `"{id}-heading"`.
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            rows: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Adds a row rendered with `ui::form_row(id, label, control)`.
    pub fn row(
        self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        control: impl IntoElement,
    ) -> Self {
        self.row_with_terms(id, label, std::iter::empty::<SharedString>(), control)
    }

    /// `row` with extra texts the search also matches, such as a key binding's text.
    pub fn row_with_terms(
        mut self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        terms: impl IntoIterator<Item = impl Into<SharedString>>,
        control: impl IntoElement,
    ) -> Self {
        self.rows.push(Row {
            id: id.into(),
            label: label.into(),
            terms: terms.into_iter().map(Into::into).collect(),
            control: control.into_any_element(),
            note: None,
        });
        self
    }

    /// Adds a note shown under the rows whenever the section shows.
    pub fn note(mut self, note: impl IntoElement) -> Self {
        self.notes.push(note.into_any_element());
        self
    }

    /// Attaches a note to the last row, shown directly after it and only while it shows.
    pub fn row_note(mut self, note: impl IntoElement) -> Self {
        if let Some(row) = self.rows.last_mut() {
            row.note = Some(note.into_any_element());
        }
        self
    }

    /// The section's id, such as `"prefs-network"`.
    pub fn id(&self) -> &SharedString {
        &self.id
    }

    /// The section's title, shown as its heading.
    pub fn title(&self) -> &SharedString {
        &self.title
    }

    /// The ids of the rows `query` leaves, in order: all of them when the title matches.
    pub fn matching_rows(&self, query: &str) -> Vec<SharedString> {
        let title_hit = matches(query, &self.title);
        self.rows
            .iter()
            .filter(|row| {
                title_hit
                    || matches(query, &row.label)
                    || row.terms.iter().any(|term| matches(query, term))
            })
            .map(|row| row.id.clone())
            .collect()
    }

    /// The section as `v_flex().id(id).test_support()` with heading, matching rows and notes,
    /// or `None` when no row matches.
    pub fn filtered(self, query: &str) -> Option<AnyElement> {
        let keep = self.matching_rows(query);
        if keep.is_empty() {
            return None;
        }
        let heading = ui::form_heading(format!("{}-heading", self.id), self.title);
        let rows = self
            .rows
            .into_iter()
            .filter(|row| keep.contains(&row.id))
            .flat_map(|row| {
                let form_row = ui::form_row(row.id, row.label, row.control).into_any_element();
                std::iter::once(form_row).chain(row.note)
            });
        Some(
            v_flex()
                .id(self.id)
                .test_support()
                .gap(SPACE_2)
                .child(heading)
                .children(rows)
                .children(self.notes)
                .into_any_element(),
        )
    }
}
