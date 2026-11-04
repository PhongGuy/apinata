//! A streaming pass over YAML events that bounds nesting and event count before the loader
//! builds anything, so a hostile document is refused in linear time and bounded memory.
//! An alias counts as every event of the node it names, so nested aliases cannot expand
//! quadratically past the budget.

use std::collections::HashMap;

use saphyr_parser::{Event, Parser};

/// The deepest container nesting the pre-pass accepts, matching the YAML loader's own limit.
pub(crate) const MAX_NESTING: usize = 128;

/// Why the pre-pass refused a document.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// More events than the budget allows.
    TooManyEvents,
    /// Nesting too deep or a syntax error, at a byte offset into the text given.
    At { offset: usize, message: String },
}

/// Stream `text`'s YAML events without building anything; refuse nesting past MAX_NESTING,
/// more than `max_events` events once aliases are expanded, or a syntax error.
pub(crate) fn prepass(text: &str, max_events: u64) -> Result<(), Refusal> {
    // saphyr-parser rejects a tab after `:` that libyaml accepts as a separator; a same-length
    // copy with spaces keeps every offset valid for the caller's text.
    let text = &text.replace(":\t", ": ");
    // saphyr-parser marks positions in chars; errors are rare, so converting on demand is enough.
    let at = |chars: usize, message: String| Refusal::At {
        offset: text
            .char_indices()
            .nth(chars)
            .map_or(text.len(), |(byte, _)| byte),
        message,
    };
    let mut events = 0u64;
    // Expanded size of each finished anchored node, and for each open container the anchor it
    // defines (0 for none) with the event count when it started.
    let mut sizes: HashMap<usize, u64> = HashMap::new();
    let mut open: Vec<(usize, u64)> = Vec::new();
    for item in Parser::new_from_str(text) {
        let (event, span) = item.map_err(|e| at(e.marker().index(), e.info().to_owned()))?;
        let before = events;
        events = events.saturating_add(match event {
            Event::Alias(id) => sizes.get(&id).copied().unwrap_or(1),
            _ => 1,
        });
        if events > max_events {
            return Err(Refusal::TooManyEvents);
        }
        match event {
            Event::Scalar(_, _, id, _) if id != 0 => {
                sizes.insert(id, 1);
            }
            Event::SequenceStart(id, _) | Event::MappingStart(id, _) => {
                open.push((id, before));
                if open.len() > MAX_NESTING {
                    return Err(at(
                        span.start.index(),
                        format!("nesting deeper than {MAX_NESTING}"),
                    ));
                }
            }
            Event::SequenceEnd | Event::MappingEnd => {
                if let Some((id, start)) = open.pop()
                    && id != 0
                {
                    sizes.insert(id, events - start);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// About 10 KB of text that expands to 64 million values with only about 1,100 alias jumps, so
/// yaml_serde's jump-per-event cap never trips.
#[cfg(test)]
pub(crate) fn hostile_yaml() -> String {
    let leaves = vec!["x"; 1000].join(", ");
    let a = vec!["*a"; 1000].join(", ");
    let b = vec!["*b"; 64].join(", ");
    format!("a: &a [{leaves}]\nb: &b [{a}]\nc: [{b}]\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_expansion_counts_against_the_budget() {
        let text = hostile_yaml();
        assert!(text.len() < 50_000);
        assert_eq!(prepass(&text, 2_000_000), Err(Refusal::TooManyEvents));
    }

    #[test]
    fn redefined_and_scalar_anchors_are_sized_by_their_latest_definition() {
        // *s is one event; the second &a is 4 events (start, 2 scalars, end), reused twice.
        let text = "s: &s 1\na: &a [1]\nb: *s\na2: &a [1, 2]\nc: [*a, *a]\n";
        // Mapping start 1 + 5 + 1 (*s) ... stays far below the budget.
        assert_eq!(prepass(text, 40), Ok(()));
        assert_eq!(prepass(text, 8), Err(Refusal::TooManyEvents));
    }

    #[test]
    fn modest_aliases_pass() {
        let text = "base: &b {a: 1, b: 2}\nx: *b\ny: *b\n";
        assert_eq!(prepass(text, 100), Ok(()));
    }
}
