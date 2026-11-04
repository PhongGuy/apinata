//! Copy value and Copy path for the JSON node under a right-click in the response body.

use apinata_core::Assertion;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Copy, SelectAll};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::{SharedString, actions};

actions!(
    apinata,
    [
        /// Copies the value of the JSON node under the caret.
        CopyJsonValue,
        /// Copies the assertion path of the JSON node under the caret.
        CopyJsonPath,
        /// Opens the Set as variable dialog for the JSON node under the caret.
        SetJsonVariable,
        /// Adds an assertion on the JSON node under the caret to the request.
        AssertJsonValue,
        /// Copies the value of the JSON node at the response body's caret.
        CopyCaretJsonValue,
        /// Copies the assertion path of the JSON node at the response body's caret.
        CopyCaretJsonPath,
        /// Opens the Set as variable dialog for the JSON node at the response body's caret.
        SetCaretJsonVariable,
        /// Adds an assertion on the JSON node at the response body's caret to the request.
        AssertCaretJsonValue
    ]
);

/// One entry of the node menu: its label, the text it copies and the toast that confirms it.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeCopy {
    /// The menu item's label.
    pub label: &'static str,
    /// What the item puts on the clipboard.
    pub text: String,
    /// The confirmation toast's message.
    pub toast: &'static str,
    /// For the Set as variable entry, the name to prefill; `None` for a copy entry.
    pub variable: Option<String>,
    /// For the Assert this value entry, the assertion to append; `None` for the other entries.
    pub assertion: Option<Assertion>,
    /// Why the entry is disabled, when it is; the entry then does nothing.
    pub locked: Option<&'static str>,
    /// For Set as variable, whether the value was secret in the shown body, so the variable
    /// must be marked sensitive.
    pub sensitive: bool,
}

/// The entries for the node at byte `offset` of the shown `text`: Copy value always, Copy path
/// when the node has an expressible path; none when `text` is not valid JSON.
pub fn node_copies(text: &str, offset: usize) -> Vec<NodeCopy> {
    node_entries(text, offset, false)
}

/// [`node_copies`], plus Assert this value after Set as variable when `assert` is set and the
/// node is a scalar the assertion engine can state and pass.
pub fn node_entries(text: &str, offset: usize, assert: bool) -> Vec<NodeCopy> {
    let Some(node) = apinata_core::json_node_at(text, offset) else {
        return Vec::new();
    };
    let value = NodeCopy {
        label: "Copy value",
        text: node.value.clone(),
        toast: "Copied the value",
        variable: None,
        assertion: None,
        locked: None,
        sensitive: false,
    };
    let set = NodeCopy {
        label: SET_VARIABLE,
        text: node.value,
        toast: "",
        variable: Some(prefill_name(node.path.as_deref())),
        assertion: None,
        locked: None,
        sensitive: false,
    };
    let assertion = node.assertion.filter(|_| assert).map(|a| NodeCopy {
        label: ASSERT_VALUE,
        text: String::new(),
        toast: "",
        variable: None,
        assertion: Some(a),
        locked: None,
        sensitive: false,
    });
    let path = node.path.map(|text| NodeCopy {
        label: "Copy path",
        text,
        toast: "Copied the path",
        variable: None,
        assertion: None,
        locked: None,
        sensitive: false,
    });
    std::iter::once(value)
        .chain(path)
        .chain(std::iter::once(set))
        .chain(assertion)
        .collect()
}

/// The byte offset of each token of the JSON `text`: a string, a number or literal, or one of
/// `{ } [ ] , :`. Two texts of the same shape, such as a body before and after redaction,
/// have the same count, so a token of one is found in the other by its place.
pub fn json_token_starts(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut starts = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let b = bytes[at];
        if b.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        starts.push(at);
        at += 1;
        match b {
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {}
            b'"' => {
                while at < bytes.len() && bytes[at] != b'"' {
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                at += 1;
            }
            _ => {
                while at < bytes.len()
                    && !bytes[at].is_ascii_whitespace()
                    && !matches!(bytes[at], b'{' | b'}' | b'[' | b']' | b',' | b':' | b'"')
                {
                    at += 1;
                }
            }
        }
    }
    starts
}

/// The label of the entry that appends an assertion on the node to the request.
pub const ASSERT_VALUE: &str = "Assert this value";

/// The label of the entry that opens the Set as variable dialog.
pub const SET_VARIABLE: &str = "Set as variable...";

/// A variable name for the node at assertion `path`: its nearest object key, with characters
/// a variable name cannot hold turned into underscores. Empty for the root and for a node with
/// no path or no key above it.
pub fn prefill_name(path: Option<&str>) -> String {
    let mut rest = path.and_then(|p| p.strip_prefix("res.body")).unwrap_or("");
    let mut key = None;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            key = Some(&after[..end]);
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix("[\"") {
            let end = after.find("\"]").unwrap_or(after.len());
            key = Some(&after[..end]);
            rest = after.get(end + 2..).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix('[') {
            // An index names no variable, so the key before it stays the answer.
            rest = after.split_once(']').map_or("", |(_, tail)| tail);
        } else {
            break;
        }
    }
    (key.unwrap_or("").chars())
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The byte offset in `text` of the first non-blank character of `rows[row]`, where `rows`
/// are `text` split on newlines (a trailing CR dropped) with long lines cut into chunks.
pub fn row_offset(text: &str, rows: &[SharedString], row: usize) -> Option<usize> {
    let mut at = 0;
    for (index, shown) in rows.iter().enumerate() {
        if !text.get(at..)?.starts_with(shown.as_ref()) {
            return None;
        }
        if index == row {
            let blank = shown.len() - shown.trim_start().len();
            return Some(at + blank);
        }
        at += shown.len();
        // A line break follows a row that ends its line; a row cut by chunking is followed
        // directly by the rest of its line.
        let rest = &text[at..];
        at += if rest.starts_with("\r\n") {
            2
        } else {
            usize::from(rest.starts_with('\n'))
        };
    }
    None
}

fn menu_locked(menu: NativeMenu, entry: &NodeCopy) -> NativeMenu {
    let label = format!("{} ({})", entry.label, entry.locked.unwrap_or_default());
    let action: Box<dyn gpui_kit::Action> = match entry.label {
        SET_VARIABLE => Box::new(SetJsonVariable),
        ASSERT_VALUE => Box::new(AssertJsonValue),
        "Copy path" => Box::new(CopyJsonPath),
        _ => Box::new(CopyJsonValue),
    };
    menu.menu_with_disabled(label, true, action)
}

impl NodeCopy {
    /// Writes the text to the clipboard and confirms with a toast.
    pub fn write(&self, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(self.text.clone()));
        window.push_notification(crate::ui::info_toast(self.toast), cx);
    }
}

/// The editor's right-click menu for a JSON body: Copy and Select all as the kit offers them,
/// then the node `entries` resolved when the press landed.
pub fn node_menu(
    menu: NativeMenu,
    entries: &[NodeCopy],
    has_selection: bool,
    cx: &gpui_kit::App,
) -> NativeMenu {
    let menu = menu
        .menu_with_disabled(
            crate::ui::native_menu_label("Copy", !has_selection, cx),
            !has_selection,
            Box::new(Copy),
        )
        .menu("Select All", Box::new(SelectAll))
        .separator();
    entries.iter().fold(menu, |menu, entry| match entry.label {
        // A locked entry stays listed, disabled, with its reason in the label.
        _ if entry.locked.is_some() => menu_locked(menu, entry),
        "Copy path" => menu.menu(entry.label, Box::new(CopyJsonPath)),
        SET_VARIABLE => menu.menu(entry.label, Box::new(SetJsonVariable)),
        ASSERT_VALUE => menu.menu(entry.label, Box::new(AssertJsonValue)),
        _ => menu.menu(entry.label, Box::new(CopyJsonValue)),
    })
}

#[cfg(test)]
mod tests {
    use super::json_token_starts;

    #[test]
    fn tokens_start_at_strings_numbers_and_punctuation() {
        let text = r#"{"a": -1.5e3, "b": [true, null]}"#;
        let tokens: Vec<&str> = json_token_starts(text)
            .iter()
            .map(|&s| &text[s..s + 1])
            .collect();
        assert_eq!(
            tokens,
            [
                "{", "\"", ":", "-", ",", "\"", ":", "[", "t", ",", "n", "]", "}"
            ]
        );
    }

    #[test]
    fn an_escaped_quote_does_not_end_a_string() {
        let text = r#"["a\"b", "c\\"]"#;
        let starts = json_token_starts(text);
        // [ "a\"b" , "c\\" ]
        assert_eq!(starts, [0, 1, 7, 9, 14]);
    }

    #[test]
    fn a_shorter_secret_keeps_the_same_token_count() {
        let raw = r#"{"k": "s3cr3t-value", "n": 12}"#;
        let masked = r#"{"k": "[redacted]", "n": 12}"#;
        assert_eq!(
            json_token_starts(raw).len(),
            json_token_starts(masked).len()
        );
    }
}
