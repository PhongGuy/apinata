//! Bulk text for key-value tables: rows as `name: value` lines and back, keeping each
//! parsed row tied to the row it most likely came from.

use crate::kv_table::KvRow;

/// The prefix that marks a disabled row's line.
const DISABLED: &str = "//";

/// Whether a row appears in the text: fixed rows, file rows and rows with a line break in
/// the name or value cannot round-trip through lines, so they stay out.
pub(crate) fn editable(row: &KvRow) -> bool {
    let multi_line = |s: &str| s.contains(['\n', '\r']);
    !row.fixed && row.files.is_none() && !multi_line(&row.name) && !multi_line(&row.value)
}

/// The editable rows as `name: value` lines, `//` before a disabled row.
pub fn bulk_text(rows: &[KvRow]) -> String {
    rows.iter()
        .filter(|row| editable(row))
        .map(|row| {
            let prefix = if row.enabled { "" } else { DISABLED };
            format!("{prefix}{}: {}", row.name, row.value)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rows parsed from `text`, each matched to a row of `current`; the rows of
/// `current` left out of the text follow unchanged, in order, each keeping its slot among the text rows.
pub fn bulk_rows(text: &str, current: &[KvRow]) -> Vec<KvRow> {
    let editable_rows: Vec<&KvRow> = current.iter().filter(|row| editable(row)).collect();
    let mut used = vec![false; editable_rows.len()];
    let mut out: Vec<KvRow> = parse(text)
        .enumerate()
        .map(|(line, (name, value, enabled))| {
            // Match the unused row of the same name nearest this line's position; on a tie
            // the lower index wins, since `min_by_key` keeps the first minimum.
            let found = editable_rows
                .iter()
                .enumerate()
                .filter(|(ix, row)| !used[*ix] && row.name == name)
                .min_by_key(|(ix, _)| ix.abs_diff(line))
                .map(|(ix, _)| ix);
            match found {
                Some(ix) => {
                    used[ix] = true;
                    KvRow {
                        value,
                        enabled,
                        ..editable_rows[ix].clone()
                    }
                }
                None => KvRow {
                    name,
                    value,
                    enabled,
                    ..KvRow::default()
                },
            }
        })
        .collect();
    // A left-out row between text rows keeps its slot, so a new line never lands above it;
    // left-out rows after the last text row stay at the end.
    let last_text = current.iter().rposition(editable);
    for (slot, row) in current.iter().enumerate().filter(|(_, row)| !editable(row)) {
        if last_text.is_some_and(|last| slot < last) {
            out.insert(slot.min(out.len()), row.clone());
        } else {
            out.push(row.clone());
        }
    }
    out
}

/// The `(name, value, enabled)` of each line that has a `:` and a non-empty name.
fn parse(text: &str) -> impl Iterator<Item = (String, String, bool)> + '_ {
    text.lines().filter_map(|line| {
        let line = line.trim();
        let (enabled, line) = match line.strip_prefix(DISABLED) {
            Some(rest) => (false, rest.trim_start()),
            None => (true, line),
        };
        let (name, value) = line.split_once(':')?;
        let name = name.trim();
        (!name.is_empty()).then(|| (name.to_owned(), value.trim().to_owned(), enabled))
    })
}
