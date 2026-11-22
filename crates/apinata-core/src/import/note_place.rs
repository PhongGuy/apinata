//! How a note's place and detail read to a person: the request or folder by name, with the
//! member path in words, and long names shortened in the middle.

use super::ImportNote;

/// The most characters a name shows in a note before its middle is cut.
const NAME_CAP: usize = 80;

impl ImportNote {
    /// Where the note applies, in words: the request or folder by its name in backticks, then
    /// the part of it in words, such as ``request `Users / Login`, auth password`` or
    /// `collection`. An index stays only where the input has no name, as in `entry 4`; a
    /// member path with no plain wording is shown as it is.
    pub fn place(&self) -> String {
        let mut names: Vec<&str> = self.source.split(" / ").collect();
        let last = names.last().copied().unwrap_or_default();
        let member = member_words(last);
        if member.is_some() {
            names.pop();
        }
        let kind = item_kind(self.file.as_deref(), last);
        let owner = match names.as_slice() {
            [] => None,
            ["collection"] => Some("collection".to_owned()),
            // One segment with no member may be a label such as `curl flag -k` rather than a
            // name, so it is shown as it is.
            [only] if member.is_none() => Some(shorten(only)),
            _ => {
                let joined: Vec<String> = names.iter().map(|n| shorten(n)).collect();
                let quoted = format!("`{}`", joined.join(" / "));
                Some(match kind {
                    Some(kind) => format!("{kind} {quoted}"),
                    None => quoted,
                })
            }
        };
        match (owner, member.flatten()) {
            (Some(owner), Some(words)) => format!("{owner}, {words}"),
            (Some(owner), None) => owner,
            (None, Some(words)) => words,
            (None, None) => shorten(&self.source),
        }
    }

    /// The detail with every name in backticks longer than the cap shortened in the middle.
    pub fn shown_detail(&self) -> String {
        let mut out = String::with_capacity(self.detail.len());
        for (i, part) in self.detail.split('`').enumerate() {
            if i > 0 {
                out.push('`');
            }
            // Odd parts sit between a pair of backticks.
            if i % 2 == 1 {
                out.push_str(&shorten(part));
            } else {
                out.push_str(part);
            }
        }
        out
    }
}

/// `name` cut in the middle with an ellipsis when it is longer than the cap.
fn shorten(name: &str) -> String {
    let count = name.chars().count();
    if count <= NAME_CAP {
        return name.to_owned();
    }
    let head = NAME_CAP / 2;
    let tail = NAME_CAP - head - 1;
    let start: String = name.chars().take(head).collect();
    let end: String = name.chars().skip(count - tail).collect();
    format!("{start}…{end}")
}

/// Whether the note's item is a request or a folder, from the file it was written to or a
/// `request.` member path; `None` for the collection, an environment or an unwritten item.
fn item_kind(file: Option<&str>, last: &str) -> Option<&'static str> {
    if last.starts_with("request.") {
        return Some("request");
    }
    let file = file?;
    if file == "folder.yml" || file.ends_with("/folder.yml") {
        Some("folder")
    } else if file == "opencollection.yml" || file.starts_with("environments/") {
        None
    } else {
        file.ends_with(".yml").then_some("request")
    }
}

/// The member path `segment` in words: `Some(None)` for a member the detail already names,
/// `Some(Some(words))` for one with words of its own, `None` when it is not a member path.
fn member_words(segment: &str) -> Option<Option<String>> {
    let (entry, rest) = match indexed(segment, "entries") {
        Some((n, rest)) => (Some(format!("entry {n}")), rest.strip_prefix('.')),
        None => (None, Some(segment)),
    };
    let words = match rest {
        None | Some("") => None,
        Some(rest) => match part_words(rest.strip_prefix("request.").unwrap_or(rest)) {
            Some(words) => words,
            // A dotted or indexed path with no plain wording is still a member path.
            None if !rest.contains(' ') && (rest.contains('.') || rest.contains('[')) => {
                Some(rest.to_owned())
            }
            None if entry.is_some() => Some(rest.to_owned()),
            None => return None,
        },
    };
    Some(match (entry, words) {
        (Some(entry), Some(words)) => Some(format!("{entry}, {words}")),
        (entry, words) => entry.or(words),
    })
}

/// Words for a known member after any `request.` prefix; see [`member_words`] for the shape.
fn part_words(part: &str) -> Option<Option<String>> {
    if indexed(part, "event").is_some() {
        return Some(None);
    }
    let words = match part {
        "testScript" => "test script".to_owned(),
        "preRequestScript" => "pre-request script".to_owned(),
        "afterResponseScript" => "post-response script".to_owned(),
        "auth" => "auth".to_owned(),
        "url" | "url.raw" => "URL".to_owned(),
        "body" => "body".to_owned(),
        "response" => "saved responses".to_owned(),
        _ if part.starts_with("body.") => "body".to_owned(),
        _ => {
            if let Some(field) = part.strip_prefix("auth.") {
                format!("auth {}", field.replace('_', " "))
            } else if let Some((n, _)) = indexed(part, "headers") {
                format!("header {n}")
            } else if let Some((n, _)) = indexed(part, "params") {
                format!("query parameter {n}")
            } else if let Some((n, _)) =
                indexed(part, "variable").or_else(|| indexed(part, "values"))
            {
                format!("variable {n}")
            } else {
                return None;
            }
        }
    };
    Some(Some(words))
}

/// For `<key>[i]<rest>`, the 1-based position `i + 1` and the rest.
fn indexed<'a>(text: &'a str, key: &str) -> Option<(usize, &'a str)> {
    let inner = text.strip_prefix(key)?.strip_prefix('[')?;
    let (digits, rest) = inner.split_once(']')?;
    let index: usize = digits.parse().ok()?;
    Some((index + 1, rest))
}
