//! Where a comparison reads its other side from, and the snapshots it compares.

use apinata_core::{
    BodyState, Comparison, DigestKey, Example, ExampleBodyKind, ExampleResponse, Response,
    ResponseSnapshot,
};

use crate::body_text::{self, BodyKind};
use crate::last_response::{KeptDigests, KeptSend, Stored};
use crate::scrub;

/// What the response on screen is compared against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareSource {
    /// The send before the one on screen.
    Previous,
    /// The saved example at this index.
    Example(usize),
}

/// One entry of the source picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceChoice {
    /// What the entry selects.
    pub source: CompareSource,
    /// The label shown.
    pub label: String,
    /// Why the entry cannot be picked, when it cannot.
    pub disabled: Option<&'static str>,
}

/// The facts of a saved example that the picker needs.
pub struct ExampleRef {
    /// The example's name.
    pub name: String,
    /// Whether the example holds a response to compare against.
    pub has_response: bool,
}

/// The picker facts of `examples`, in order.
pub fn example_refs(examples: &[Example]) -> Vec<ExampleRef> {
    examples
        .iter()
        .map(|e| ExampleRef {
            name: e.name.clone(),
            has_response: e.response.is_some(),
        })
        .collect()
}

/// The seq of the send before the one shown. `history` is newest first and its head is the send
/// on screen when `viewing` is `None`; otherwise the entry after `viewing`.
pub fn previous_seq(history: &[KeptSend], viewing: Option<u64>) -> Option<u64> {
    let after = match viewing {
        None => 0,
        Some(seq) => history.iter().position(|k| k.seq == seq)? + 1,
    };
    // With nothing viewed, the head is the send on screen, so "previous" starts one further.
    let at = if viewing.is_none() { after + 1 } else { after };
    history.get(at).map(|k| k.seq)
}

/// The picker entries: the previous send, then one per example.
pub fn choices(
    history: &[KeptSend],
    viewing: Option<u64>,
    examples: &[ExampleRef],
) -> Vec<SourceChoice> {
    let previous = SourceChoice {
        source: CompareSource::Previous,
        label: "Previous send".to_owned(),
        disabled: previous_seq(history, viewing)
            .is_none()
            .then_some("No earlier send"),
    };
    let rest = examples.iter().enumerate().map(|(i, e)| SourceChoice {
        source: CompareSource::Example(i),
        label: e.name.clone(),
        disabled: (!e.has_response).then_some("This example has no response"),
    });
    std::iter::once(previous).chain(rest).collect()
}

/// One header value of a compared side: the text shown, the real value when this session holds
/// it, and a keyed digest of the real value when one is known. `raw` equal to `shown` means the
/// value was never masked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderValue {
    /// The header name as received.
    pub name: String,
    /// The value as the Headers tab shows it hidden.
    pub shown: String,
    /// The real value, `None` when only the masked text was kept.
    pub raw: Option<String>,
    /// The current key's digest of the real value, `None` without a key or a known digest.
    pub digest: Option<String>,
}

impl HeaderValue {
    /// Whether the shown text hides something, so a reveal could differ.
    pub fn masked(&self) -> bool {
        self.raw.as_deref() != Some(self.shown.as_str())
    }
}

/// One side of a comparison: its snapshot without headers, and its header values in order.
#[derive(Clone, Debug)]
pub struct CompareSide {
    /// Status and body; the headers are added per comparison.
    pub base: ResponseSnapshot,
    /// The header values in received order.
    pub headers: Vec<HeaderValue>,
}

impl CompareSide {
    /// The side as shown: every header masked.
    pub fn shown_snapshot(&self) -> ResponseSnapshot {
        let mut snap = self.base.clone();
        for h in &self.headers {
            snap = snap.with_header(h.name.clone(), h.shown.clone());
        }
        snap
    }
}

/// The live side: shown scrubbed of `secrets` so it matches what the store keeps, each header
/// masked as the CLI shows it with its true value beside it. The values at `kept_masked` places
/// were masked at rest (a restored send), so they have no true value, only the digest `kept`
/// holds for them when it was made under `key`. Other true values are digested under `key`.
pub fn side_of_response(
    response: &Response,
    secrets: &[String],
    kept_masked: &[usize],
    kept: Option<&KeptDigests>,
    key: Option<&DigestKey>,
) -> CompareSide {
    let forms = scrub::Forms::new(secrets);
    let scrub = |s: &str| forms.scrub(s);
    let base =
        ResponseSnapshot::new(response.status()).with_status_text(scrub(response.status_text()));
    let kept = kept.filter(|k| key.is_some_and(|key| k.key == key.id()));
    let headers = (response.headers().iter().enumerate())
        .map(|(ix, (name, value))| {
            let raw = (!kept_masked.contains(&ix)).then(|| value.clone());
            let held = kept.and_then(|k| k.values.iter().find(|(at, _)| *at == ix));
            let digest = match (key, held) {
                (None, _) => None,
                (Some(_), Some((_, digest))) => Some(digest.clone()),
                (Some(key), None) => raw.as_deref().map(|raw| key.digest(name, raw)),
            };
            HeaderValue {
                name: name.clone(),
                shown: scrub(&apinata_core::mask_response_header(name, value)),
                raw,
                digest,
            }
        })
        .collect();
    let base = if response.truncated() {
        base.with_body_state(BodyState::Truncated)
    } else {
        match body_text::body_kind(response.headers(), response.body()) {
            BodyKind::Binary | BodyKind::Image(_) => base.with_body_state(BodyState::Binary),
            _ => base.with_body(scrub(response.text())),
        }
    };
    CompareSide { base, headers }
}

/// The live side as shown, every header masked.
pub fn snapshot_of_response(response: &Response, secrets: &[String]) -> ResponseSnapshot {
    side_of_response(response, secrets, &[], None, None).shown_snapshot()
}

/// The kept side. It was scrubbed when saved, so it is not scrubbed again; a value the store
/// masked, or one the mask still changes, has no real value, only its kept digest under `key`.
/// A value holding the placeholder is masked too (see [`drop_placeholder_values`]).
pub fn side_of_stored(stored: &Stored, key: Option<&DigestKey>) -> CompareSide {
    let Some(response) = &stored.result.response else {
        return CompareSide {
            base: ResponseSnapshot::new(0).with_body_state(BodyState::Dropped),
            headers: Vec::new(),
        };
    };
    let kept = stored.header_digests.as_ref();
    let side = at_rest(side_of_response(
        response,
        &[],
        &stored.masked_headers,
        kept,
        key,
    ));
    let mut side = drop_placeholder_values(side, kept, key);
    if stored.body_dropped {
        side.base = side.base.with_body_state(BodyState::Dropped);
    }
    side
}

/// Masks a kept side's values that hold the placeholder, as a send kept before masked places
/// were recorded holds it as plain text: such a value has no true value, and its digest goes
/// too unless `kept` holds one for its place made under `key`.
pub fn drop_placeholder_values(
    mut side: CompareSide,
    kept: Option<&KeptDigests>,
    key: Option<&DigestKey>,
) -> CompareSide {
    let kept = kept.filter(|k| key.is_some_and(|key| k.key == key.id()));
    let held = |ix: usize| kept.is_some_and(|k| k.values.iter().any(|(at, _)| *at == ix));
    for (ix, h) in side.headers.iter_mut().enumerate() {
        if h.raw.as_deref().is_some_and(scrub::holds_placeholder) {
            h.raw = None;
            if !held(ix) {
                h.digest = None;
            }
        }
    }
    side
}

/// A kept side, from the store or restored on screen: a value the mask still changes was
/// kept as it was given, so only its masked text may be shown or compared. Its digest was made
/// from that masked text, so it goes too.
pub fn at_rest(mut side: CompareSide) -> CompareSide {
    for h in &mut side.headers {
        if h.raw.as_deref().is_some_and(|raw| raw != h.shown) {
            h.raw = None;
            h.digest = None;
        }
    }
    side
}

/// The kept side as shown.
pub fn snapshot_of_stored(stored: &Stored) -> ResponseSnapshot {
    side_of_stored(stored, None).shown_snapshot()
}

/// A saved example's side: kept as written, so every value is its own real value. Under `key`
/// a value is also digested, so it meets a masked side; only one holding the placeholder cannot
/// be a true value and has no digest.
pub fn side_of_example(response: &ExampleResponse, key: Option<&DigestKey>) -> CompareSide {
    let mut bare = response.clone();
    bare.headers.clear();
    let base = snapshot_of_example(&bare);
    let headers = (response.headers.iter())
        .map(|(name, value)| HeaderValue {
            name: name.clone(),
            shown: value.clone(),
            raw: Some(value.clone()),
            digest: key
                .filter(|_| !scrub::holds_placeholder(value))
                .map(|key| key.digest(name, value)),
        })
        .collect();
    CompareSide { base, headers }
}

/// Compares two sides. Each header name is compared by its real values when both sides hold
/// every one of them, else by their digests when both hold every one, else by the shown text,
/// so a real value never meets a masked one.
pub fn compare_sides(old: &CompareSide, new: &CompareSide) -> Comparison {
    let every = |name: &str, has: fn(&HeaderValue) -> bool| {
        [old, new].iter().all(|side| {
            (side.headers.iter())
                .filter(|h| h.name.eq_ignore_ascii_case(name))
                .all(has)
        })
    };
    let snapshot = |side: &CompareSide| {
        let mut snap = side.base.clone();
        for h in &side.headers {
            let text = match (&h.raw, &h.digest) {
                (Some(raw), _) if every(&h.name, |h| h.raw.is_some()) => raw.clone(),
                (_, Some(digest)) if every(&h.name, |h| h.digest.is_some()) => digest.clone(),
                _ => h.shown.clone(),
            };
            snap = snap.with_header(h.name.clone(), text);
        }
        snap
    };
    apinata_core::compare(&snapshot(old), &snapshot(new))
}

/// A saved example's response as a snapshot.
pub fn snapshot_of_example(response: &ExampleResponse) -> ResponseSnapshot {
    let mut snap = ResponseSnapshot::new(response.status.unwrap_or(0))
        .with_status_text(response.status_text.clone().unwrap_or_default());
    for (name, value) in &response.headers {
        snap = snap.with_header(name.clone(), value.clone());
    }
    match &response.body {
        Some(b) if b.kind == ExampleBodyKind::Binary => snap.with_body_state(BodyState::Binary),
        Some(b) => snap.with_body(b.data.clone()),
        None => snap.with_body(""),
    }
}
