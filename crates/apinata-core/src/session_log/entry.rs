//! Field builders that redact and cap each value before it is stored, and the entry types'
//! `Debug` impls, which show sizes and header names only.

use super::cap::{Cut, cap_bytes, cap_bytes_of, recap};
use super::redact::Redactor;
use super::{ConsoleLine, LoggedBody, LoggedHop, NetworkEntry};
use crate::{ConsoleLevel, ScriptPhase};
use std::fmt;
use std::mem::size_of;
use std::time::SystemTime;

pub(crate) const URL_CAP: usize = 8 * 1024;
pub(crate) const METHOD_CAP: usize = 64;
pub(crate) const ERROR_CAP: usize = 4 * 1024;
pub(crate) const HEADERS_CAP: usize = 32 * 1024;
pub(crate) const PAIR_OVERHEAD: usize = 48;
pub(crate) const HOPS_CAP: usize = 32;
pub(crate) const CONSOLE_CAP: usize = 16 * 1024;
pub(crate) const NAME_CAP: usize = 1024;

/// A stored text field and its cut record, which the entry keeps beside it.
pub(crate) type Field = (String, Option<Cut>);

/// `url` without its userinfo password, redacted and capped at `URL_CAP`.
pub(crate) fn logged_url(r: &Redactor, url: &str) -> Field {
    cap_bytes(r, r.redact_url(url).as_bytes(), URL_CAP)
}

/// `method` redacted and capped at `METHOD_CAP`.
pub(crate) fn logged_method(r: &Redactor, method: &str) -> Field {
    cap_bytes(r, method.as_bytes(), METHOD_CAP)
}

/// An error's text redacted and capped at `ERROR_CAP`.
pub(crate) fn logged_error(r: &Redactor, error: &str) -> Field {
    cap_bytes(r, error.as_bytes(), ERROR_CAP)
}

/// `bytes` decoded lossily, redacted and capped at `cap`; `size` counts the raw bytes.
#[cfg(test)]
pub(crate) fn logged_body(r: &Redactor, bytes: &[u8], cap: usize) -> LoggedBody {
    logged_body_of(r, bytes, bytes.len() as u64, cap)
}

/// `bytes`, the kept start of a body `size` bytes long, decoded lossily, redacted and
/// capped at `cap`; a body kept only in part is stored as cut.
pub(crate) fn logged_body_of(r: &Redactor, bytes: &[u8], size: u64, cap: usize) -> LoggedBody {
    let whole = bytes.len() as u64 == size && bytes.len() <= cap;
    // UTF-16 copies are swapped before the text is made; a whole body's N follows the swap.
    let (text, cut) = match r.redact_wide(bytes) {
        Some(wide) if bytes.len() as u64 == size => cap_bytes_of(r, &wide, cap, wide.len() as u64),
        Some(wide) => cap_bytes_of(r, &wide, cap, size),
        None => cap_bytes_of(r, bytes, cap, size),
    };
    let raw =
        (whole && std::str::from_utf8(bytes).is_err() && !r.holds(bytes)).then(|| bytes.to_vec());
    LoggedBody {
        text,
        size,
        truncated: cut.is_some(),
        cut,
        raw,
    }
}

/// `headers` redacted pair by pair, keeping whole pairs while their cost stays within
/// `HEADERS_CAP`; the first pair that would pass it and every later one fold into a single
/// `("…", "{n} more headers")` pair. Also returns n, or 0 when nothing folded.
pub(crate) fn logged_headers(
    r: &Redactor,
    headers: &[(String, String)],
) -> (Vec<(String, String)>, usize) {
    // The credential rule looks at the raw name; the stored name goes through the forms.
    let pairs = headers
        .iter()
        .map(|(name, value)| (r.redact(name), r.redact_header(name, value)));
    fold_headers(pairs, headers.len())
}

/// Whole `pairs` kept while their cost stays within `HEADERS_CAP`, then one overflow pair
/// counting the rest of the `count` headers the list stands for, and that rest. Pairs
/// past the cut are never pulled from the iterator.
fn fold_headers(
    pairs: impl Iterator<Item = (String, String)>,
    count: usize,
) -> (Vec<(String, String)>, usize) {
    let mut kept = Vec::new();
    let mut cost = 0;
    for (mut n, mut v) in pairs {
        let pair = n.len() + v.len() + PAIR_OVERHEAD;
        if cost + pair > HEADERS_CAP {
            break;
        }
        cost += pair;
        n.shrink_to_fit();
        v.shrink_to_fit();
        kept.push((n, v));
    }
    let folded = count.saturating_sub(kept.len());
    if folded > 0 {
        let mut more = format!("{folded} more headers");
        more.shrink_to_fit();
        kept.push(("…".to_owned(), more));
    }
    kept.shrink_to_fit();
    (kept, folded)
}

/// At most `HOPS_CAP` hops, keeping the first `HOPS_CAP - 1` and the last, with the
/// number dropped from between them.
pub(crate) fn cap_hops<T>(mut hops: Vec<T>) -> (Vec<T>, usize) {
    if hops.len() <= HOPS_CAP {
        hops.shrink_to_fit();
        return (hops, 0);
    }
    let omitted = hops.len() - HOPS_CAP;
    hops.drain(HOPS_CAP - 1..hops.len() - 1);
    hops.shrink_to_fit();
    (hops, omitted)
}

/// The heap bytes a header list owns: its pair slots plus every name and value.
fn pairs_bytes(pairs: &Vec<(String, String)>) -> usize {
    pairs.capacity() * size_of::<(String, String)>()
        + pairs
            .iter()
            .map(|(n, v)| n.capacity() + v.capacity())
            .sum::<usize>()
}

/// `stored` re-redacted under the truncation invariant from its cut record; an uncut
/// field that grows past `cap` takes its own stored length as N.
fn recap_field(r: &Redactor, stored: &mut String, cut: &mut Option<Cut>, cap: usize) {
    (*stored, *cut) = recap(r, stored, *cut, cap, stored.len() as u64);
}

/// Every real pair re-redacted, then the list cap re-applied so growth folds whole pairs
/// into a regenerated overflow pair; `folded` is the overflow count, read and updated.
fn rededact_headers(r: &Redactor, headers: &mut Vec<(String, String)>, folded: &mut usize) {
    let mut pairs = std::mem::take(headers);
    if *folded > 0 {
        pairs.pop();
    }
    let count = pairs.len().saturating_add(*folded);
    // The stored name feeds the credential rule, as the raw name did at capture.
    let redacted = pairs
        .into_iter()
        .map(|(name, value)| (r.redact(&name), r.redact_header(&name, &value)));
    (*headers, *folded) = fold_headers(redacted, count);
}

/// Every header value replaced by `placeholder`, names kept, then the list cap re-applied
/// as in `rededact_headers`.
fn withhold_headers(headers: &mut Vec<(String, String)>, folded: &mut usize, placeholder: &str) {
    let mut pairs = std::mem::take(headers);
    if *folded > 0 {
        pairs.pop();
    }
    let count = pairs.len().saturating_add(*folded);
    let withheld = pairs
        .into_iter()
        .map(|(name, _)| (name, placeholder.to_owned()));
    (*headers, *folded) = fold_headers(withheld, count);
}

/// `url` with its query and fragment, where its kept content shows them, each replaced by
/// `placeholder`, then recapped at `URL_CAP`.
fn withhold_url(url: &mut String, cut: &mut Option<Cut>, placeholder: &str) {
    let content = cut.map_or(url.as_str(), |c| &url[..c.kept]);
    let hash = content.find('#');
    let query = content.find('?').filter(|q| hash.is_none_or(|h| *q < h));
    let Some(end) = query.or(hash) else {
        return;
    };
    let mut out = content[..end].to_owned();
    if query.is_some() {
        out.push('?');
        out.push_str(placeholder);
    }
    if hash.is_some() {
        out.push('#');
        out.push_str(placeholder);
    }
    (*url, *cut) = cap_bytes(&Redactor::none(), out.as_bytes(), URL_CAP);
}

impl LoggedBody {
    /// The exact bytes of a binary body, kept only when the whole body fit the cap and held
    /// no value the log hides; `None` for text, which `text` already holds exactly.
    pub fn raw(&self) -> Option<&[u8]> {
        self.raw.as_deref()
    }

    /// Re-redacts the text at `cap` with `size` as N; a cut sets `truncated`, `size` stays.
    /// Bytes that now hold a value cannot be redacted in place, so they are dropped.
    fn rededact(&mut self, r: &Redactor, cap: usize) {
        // UTF-16 copies in the kept content are swapped first; the cut moves with the content.
        let kept = self.cut.map_or(self.text.len(), |c| c.kept);
        if let Some(wide) = r.redact_wide(&self.text.as_bytes()[..kept]) {
            let content = String::from_utf8_lossy(&wide).into_owned();
            if let Some(cut) = &mut self.cut {
                cut.kept = content.len();
            }
            self.text = content + &self.text[kept..];
        }
        (self.text, self.cut) = recap(r, &self.text, self.cut, cap, self.size);
        self.truncated |= self.cut.is_some();
        if self.raw.as_deref().is_some_and(|raw| r.holds(raw)) {
            self.raw = None;
        }
    }

    /// The text replaced whole by `placeholder`; `size` stays.
    fn withhold(&mut self, placeholder: &str) {
        self.text = placeholder.to_owned();
        self.cut = None;
        self.truncated = false;
        self.raw = None;
    }

    fn owned_bytes(&self) -> usize {
        self.text.capacity() + self.raw.as_ref().map_or(0, Vec::capacity)
    }
}

impl LoggedHop {
    /// The heap bytes the hop owns; its own slot is counted by the vector holding it.
    fn owned_bytes(&self) -> usize {
        self.method.capacity()
            + self.url.capacity()
            + pairs_bytes(&self.request_headers)
            + pairs_bytes(&self.response_headers)
            + self.pac.as_ref().map_or(0, String::capacity)
            + self.override_from.as_ref().map_or(0, String::capacity)
    }

    fn rededact(&mut self, r: &Redactor) {
        recap_field(r, &mut self.method, &mut self.method_cut, METHOD_CAP);
        recap_field(r, &mut self.url, &mut self.url_cut, URL_CAP);
        if let Some(pac) = &mut self.pac {
            recap_field(r, pac, &mut self.pac_cut, ERROR_CAP);
        }
        if let Some(line) = &mut self.override_from {
            recap_field(r, line, &mut self.override_cut, ERROR_CAP);
        }
        rededact_headers(r, &mut self.request_headers, &mut self.request_folded);
        rededact_headers(r, &mut self.response_headers, &mut self.response_folded);
    }

    fn withhold(&mut self, placeholder: &str) {
        withhold_url(&mut self.url, &mut self.url_cut, placeholder);
        if let Some(pac) = &mut self.pac {
            *pac = placeholder.to_owned();
            self.pac_cut = None;
        }
        if let Some(line) = &mut self.override_from {
            *line = placeholder.to_owned();
            self.override_cut = None;
        }
        withhold_headers(
            &mut self.request_headers,
            &mut self.request_folded,
            placeholder,
        );
        withhold_headers(
            &mut self.response_headers,
            &mut self.response_folded,
            placeholder,
        );
    }
}

impl NetworkEntry {
    /// The struct's size plus the capacity of every string and vector it owns.
    pub(crate) fn accounted_bytes(&self) -> usize {
        size_of::<NetworkEntry>()
            + self.method.capacity()
            + self.url.capacity()
            + self.error.as_ref().map_or(0, String::capacity)
            + pairs_bytes(&self.request_headers)
            + self.request_body.owned_bytes()
            + self.response_body.owned_bytes()
            + self.hops.capacity() * size_of::<LoggedHop>()
            + self.hops.iter().map(LoggedHop::owned_bytes).sum::<usize>()
    }

    /// Every text field re-redacted with `r` under the truncation invariant; bodies recap at `body_cap`.
    pub(crate) fn rededact(&mut self, r: &Redactor, body_cap: usize) {
        recap_field(r, &mut self.method, &mut self.method_cut, METHOD_CAP);
        recap_field(r, &mut self.url, &mut self.url_cut, URL_CAP);
        if let Some(error) = &mut self.error {
            recap_field(r, error, &mut self.error_cut, ERROR_CAP);
        }
        rededact_headers(r, &mut self.request_headers, &mut self.request_folded);
        self.request_body.rededact(r, body_cap);
        self.response_body.rededact(r, body_cap);
        for hop in &mut self.hops {
            hop.rededact(r);
        }
    }

    /// Replaces with `placeholder` every field a response or a script could echo a secret
    /// into: both bodies, the error, every header value and each URL's query and fragment.
    pub(crate) fn withhold(&mut self, placeholder: &str) {
        withhold_url(&mut self.url, &mut self.url_cut, placeholder);
        if let Some(error) = &mut self.error {
            *error = placeholder.to_owned();
            self.error_cut = None;
        }
        withhold_headers(
            &mut self.request_headers,
            &mut self.request_folded,
            placeholder,
        );
        self.request_body.withhold(placeholder);
        self.response_body.withhold(placeholder);
        for hop in &mut self.hops {
            hop.withhold(placeholder);
        }
    }
}

impl ConsoleLine {
    /// A console line with its request name and text redacted and capped; `seq` is 0.
    pub(crate) fn capture(
        at: SystemTime,
        request: &str,
        phase: Option<ScriptPhase>,
        level: ConsoleLevel,
        text: &str,
        r: &Redactor,
    ) -> ConsoleLine {
        let (request, request_cut) = cap_bytes(r, request.as_bytes(), NAME_CAP);
        let (text, text_cut) = cap_bytes(r, text.as_bytes(), CONSOLE_CAP);
        ConsoleLine {
            seq: 0,
            at,
            request,
            phase,
            level,
            text,
            request_cut,
            text_cut,
        }
    }

    /// The struct's size plus the capacity of its request name and text.
    pub(crate) fn accounted_bytes(&self) -> usize {
        size_of::<ConsoleLine>() + self.request.capacity() + self.text.capacity()
    }

    /// The text and request name re-redacted with `r` under the truncation invariant.
    pub(crate) fn rededact(&mut self, r: &Redactor) {
        recap_field(r, &mut self.text, &mut self.text_cut, CONSOLE_CAP);
        recap_field(r, &mut self.request, &mut self.request_cut, NAME_CAP);
    }

    /// Replaces the whole text with `placeholder`, for a line that cannot be redacted safely.
    pub(crate) fn withhold(&mut self, placeholder: &str) {
        self.text = placeholder.to_owned();
        self.text_cut = None;
    }
}

fn names(headers: &[(String, String)]) -> Vec<&str> {
    headers.iter().map(|(name, _)| name.as_str()).collect()
}

impl fmt::Debug for NetworkEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetworkEntry")
            .field("seq", &self.seq)
            .field("source", &self.source)
            .field("method", &self.method)
            .field("status", &self.status)
            .field("hops", &self.hops.len())
            .field("hops_omitted", &self.hops_omitted)
            .field("request_headers", &names(&self.request_headers))
            .field("request_body", &self.request_body)
            .field("response_body", &self.response_body)
            .finish()
    }
}

impl fmt::Debug for LoggedHop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoggedHop")
            .field("method", &self.method)
            .field("status", &self.status)
            .field("request_headers", &names(&self.request_headers))
            .field("response_headers", &names(&self.response_headers))
            .field("elapsed", &self.elapsed)
            .field("version", &self.version)
            .field("remote_addr", &self.remote_addr)
            .field("via_proxy", &self.via_proxy)
            .finish()
    }
}

impl fmt::Debug for LoggedBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoggedBody")
            .field("size", &self.size)
            .field("truncated", &self.truncated)
            .finish()
    }
}

impl fmt::Debug for ConsoleLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleLine")
            .field("seq", &self.seq)
            .field("phase", &self.phase)
            .field("level", &self.level)
            .field("text_len", &self.text.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_log::cap::marker;
    use crate::session_log::redact::Redactor;
    use crate::session_log::test_support::{entry_texts, entry_with, hop_with, line_with};
    use crate::session_log::{ConsoleLine, LogLimits, LoggedHop, NetworkEntry, SendSource};
    use crate::{ConsoleLevel, ScriptPhase};
    use std::time::SystemTime;

    fn set_request_headers(e: &mut NetworkEntry, r: &Redactor, headers: &[(String, String)]) {
        (e.request_headers, e.request_folded) = logged_headers(r, headers);
    }

    /// Marks the list's last pair as the synthetic overflow pair standing for `count` headers.
    fn set_overflow(e: &mut NetworkEntry, count: usize) {
        e.request_folded = count;
    }

    #[test]
    fn limits_and_source_names() {
        assert_eq!(
            LogLimits::default(),
            LogLimits {
                console: 1000,
                network: 500,
                body_bytes: 64 * 1024,
                budget_bytes: 16 * 1024 * 1024
            }
        );
        let names: Vec<_> = [
            SendSource::Request,
            SendSource::Runner,
            SendSource::Script,
            SendSource::OAuth2,
            SendSource::GraphqlSchema,
        ]
        .map(SendSource::as_str)
        .into();
        assert_eq!(
            names,
            ["request", "runner", "script", "oauth2", "graphql-schema"]
        );
    }

    #[test]
    fn builders_redact_every_field() {
        let r = Redactor::new(&["s3cret"]);
        let e = entry_with(&r, "s3cret", b"{\"k\":\"s3cret\"}", 1024);
        for t in entry_texts(&e) {
            assert!(!t.contains("s3cret"), "{t}");
        }
        assert_eq!(e.request_headers[1].1, "[redacted]");
        assert_eq!(e.hops[0].response_headers[0].1, "[redacted]");
        assert_eq!(e.response_body.size, 14);
    }

    #[test]
    fn header_names_are_redacted_by_value_matching() {
        let r = Redactor::new(&["s3cret"]);
        let (kept, _) = logged_headers(&r, &[("X-s3cret".to_owned(), "v".to_owned())]);
        assert_eq!(kept, vec![("X-[redacted]".to_owned(), "v".to_owned())]);
    }

    #[test]
    fn console_text_and_request_name_are_redacted_in_any_case() {
        let r = Redactor::new(&["s3cret"]);
        let l = ConsoleLine::capture(
            SystemTime::UNIX_EPOCH,
            "req s3cret S3CRET",
            Some(ScriptPhase::PreRequest),
            ConsoleLevel::Log,
            "token=s3cret and S3Cret",
            &r,
        );
        for stored in [&l.text, &l.request] {
            assert!(!stored.to_ascii_lowercase().contains("s3cret"), "{stored}");
        }
    }

    #[test]
    fn a_huge_console_line_keeps_sixteen_kib_and_the_marker() {
        let text = "y".repeat(20 * 1024 * 1024);
        let l = line_with(&Redactor::none(), &text);
        assert_eq!(l.seq, 0);
        assert_eq!(
            l.text,
            format!(
                "{}{}",
                "y".repeat(CONSOLE_CAP),
                marker(CONSOLE_CAP, text.len() as u64)
            )
        );
        let named = ConsoleLine::capture(
            SystemTime::UNIX_EPOCH,
            &"n".repeat(5000),
            Some(ScriptPhase::Tests),
            ConsoleLevel::Warn,
            "t",
            &Redactor::none(),
        );
        assert!(
            named.request.starts_with(&"n".repeat(NAME_CAP)) && named.request.len() < NAME_CAP + 64
        );
    }

    #[test]
    fn short_fields_are_capped() {
        let none = Redactor::none();
        let url = format!("https://user:pw@api.test/{}", "p".repeat(20 * 1024));
        let (logged, _) = logged_url(&none, &url);
        assert!(logged.starts_with("https://api.test/ppp"));
        assert!(logged.len() <= URL_CAP + 64 && logged.contains("\n[truncated: kept "));
        assert_eq!(
            logged_method(&none, &"M".repeat(100)).0,
            format!("{}{}", "M".repeat(METHOD_CAP), marker(METHOD_CAP, 100))
        );
        assert!(
            logged_error(&none, &"e".repeat(10 * 1024))
                .0
                .starts_with(&"e".repeat(ERROR_CAP))
        );
        assert_eq!(logged_error(&none, "short"), ("short".to_owned(), None));
    }

    #[test]
    fn bodies_are_lossy_and_count_raw_bytes() {
        let b = logged_body(&Redactor::none(), &[0xff, 0xfe, b'a'], 1024);
        assert_eq!(
            (b.text.as_str(), b.size, b.truncated),
            ("\u{FFFD}\u{FFFD}a", 3, false)
        );
        let big = logged_body(&Redactor::none(), &[b'z'; 100], 10);
        assert_eq!((big.size, big.truncated), (100, true));
        assert_eq!(big.text, format!("{}{}", "z".repeat(10), marker(10, 100)));
    }

    #[test]
    fn header_lists_keep_whole_pairs_within_the_cap() {
        let headers: Vec<(String, String)> = (0..1000)
            .map(|i| (format!("X-H{i}"), "v".repeat(100)))
            .collect();
        let (kept, folded) = logged_headers(&Redactor::none(), &headers);
        let (overflow, pairs) = kept.split_last().unwrap();
        assert_eq!(folded, 1000 - pairs.len());
        let cost: usize = pairs
            .iter()
            .map(|(n, v)| n.len() + v.len() + PAIR_OVERHEAD)
            .sum();
        assert!(cost <= HEADERS_CAP);
        let next = &headers[pairs.len()];
        assert!(cost + next.0.len() + next.1.len() + PAIR_OVERHEAD > HEADERS_CAP);
        assert_eq!(pairs, &headers[..pairs.len()]);
        assert_eq!(
            overflow,
            &(
                "…".to_owned(),
                format!("{} more headers", 1000 - pairs.len())
            )
        );
        let short = vec![("A".to_owned(), "1".to_owned())];
        assert_eq!(logged_headers(&Redactor::none(), &short), (short, 0));
    }

    #[test]
    fn a_single_oversized_header_value_folds_into_the_overflow_pair() {
        let headers = vec![("X-Big".to_owned(), "b".repeat(40 * 1024))];
        assert_eq!(
            logged_headers(&Redactor::none(), &headers),
            (vec![("…".to_owned(), "1 more headers".to_owned())], 1)
        );
    }

    #[test]
    fn hops_keep_the_first_thirty_one_and_the_last() {
        let base = entry_with(&Redactor::none(), "v", b"", 10).hops.remove(0);
        let hops: Vec<LoggedHop> = (0..40u16)
            .map(|i| LoggedHop {
                status: 300 + i,
                ..base.clone()
            })
            .collect();
        let (kept, omitted) = cap_hops(hops.clone());
        assert_eq!(omitted, 8);
        let statuses: Vec<u16> = kept.iter().map(|h| h.status).collect();
        let mut expected: Vec<u16> = (300..331).collect();
        expected.push(339);
        assert_eq!(statuses, expected);
        let (same, none) = cap_hops(hops[..32].to_vec());
        assert_eq!((same.len(), none), (32, 0));
    }

    #[test]
    fn debug_shows_sizes_and_header_names_only() {
        let e = entry_with(&Redactor::none(), "VISIBLE", b"BODYTEXT", 1024);
        let shown = format!("{e:?}");
        assert!(
            shown.contains("X-Key") && shown.contains("Authorization"),
            "{shown}"
        );
        for hidden in ["VISIBLE", "BODYTEXT", "api.test", "failed near"] {
            assert!(!shown.contains(hidden), "{hidden} in {shown}");
        }
        let l = line_with(&Redactor::none(), "CONSOLETEXT");
        assert!(!format!("{l:?}").contains("CONSOLETEXT"));
    }

    fn pairs_bytes(v: &Vec<(String, String)>) -> usize {
        v.capacity() * std::mem::size_of::<(String, String)>()
            + v.iter()
                .map(|(a, b)| a.capacity() + b.capacity())
                .sum::<usize>()
    }

    #[test]
    fn console_accounting_counts_struct_and_strings() {
        let l = line_with(&Redactor::none(), &"t".repeat(300));
        assert_eq!(
            l.accounted_bytes(),
            std::mem::size_of::<ConsoleLine>() + l.request.capacity() + l.text.capacity()
        );
        let huge = line_with(&Redactor::none(), &"y".repeat(20 * 1024 * 1024));
        assert!(
            huge.accounted_bytes()
                <= std::mem::size_of::<ConsoleLine>() + NAME_CAP + CONSOLE_CAP + 128
        );
    }

    #[test]
    fn network_accounting_counts_every_owned_byte() {
        let e = entry_with(&Redactor::none(), "v", &[b'q'; 500], 1024);
        let hops: usize = e.hops.capacity() * std::mem::size_of::<LoggedHop>()
            + e.hops
                .iter()
                .map(|h| {
                    h.method.capacity()
                        + h.url.capacity()
                        + pairs_bytes(&h.request_headers)
                        + pairs_bytes(&h.response_headers)
                        + h.pac.as_ref().map_or(0, String::capacity)
                })
                .sum::<usize>();
        let expected = std::mem::size_of::<NetworkEntry>()
            + e.method.capacity()
            + e.url.capacity()
            + e.error.as_ref().map_or(0, String::capacity)
            + pairs_bytes(&e.request_headers)
            + e.request_body.text.capacity()
            + e.response_body.text.capacity()
            + hops;
        assert_eq!(e.accounted_bytes(), expected);
    }

    #[test]
    fn the_largest_entry_stays_under_two_and_a_half_mib() {
        let none = Redactor::none();
        let url = format!("https://api.test/{}", "u".repeat(20 * 1024));
        let headers: Vec<(String, String)> = (0..1000)
            .map(|i| (format!("X-H{i}"), "v".repeat(100)))
            .collect();
        let body = vec![b'b'; 1024 * 1024];
        let mut hop = hop_with(&none, &url, &headers, &headers);
        (hop.method, hop.method_cut) = logged_method(&none, &"M".repeat(100));
        hop.status = 302;
        let (hops, omitted) = cap_hops(vec![hop; 40]);
        let mut e = entry_with(&none, "v", &body, LogLimits::default().body_bytes);
        (e.url, e.url_cut) = logged_url(&none, &url);
        set_request_headers(&mut e, &none, &headers);
        let (error, error_cut) = logged_error(&none, &"e".repeat(10 * 1024));
        (e.error, e.error_cut) = (Some(error), error_cut);
        e.hops = hops;
        e.hops_omitted = omitted;
        let bytes = e.accounted_bytes();
        assert!(bytes > 2 * 1024 * 1024, "{bytes}");
        assert!(bytes <= 2_621_440, "{bytes}");
    }

    #[test]
    fn rededact_removes_a_value_learned_after_capture_from_every_field() {
        let late = "LATE-SECRET-42";
        let mut e = entry_with(
            &Redactor::none(),
            late,
            format!("body {late} end").as_bytes(),
            1024,
        );
        let mut l = line_with(&Redactor::none(), &format!("saw {late}"));
        let r = Redactor::new(&[late]);
        e.rededact(&r, 1024);
        l.rededact(&r);
        for t in entry_texts(&e) {
            assert!(!t.contains(late), "{t}");
        }
        assert_eq!(l.text, "saw [redacted]");
        assert_eq!(e.request_body.size, (late.len() + 9) as u64);
    }

    #[test]
    fn rededact_trims_a_late_value_straddling_the_body_cut() {
        let late = "LATESECRET";
        let body = format!("{}{late}tail", ".".repeat(20));
        let mut e = entry_with(&Redactor::none(), "m", body.as_bytes(), 25);
        assert!(e.response_body.truncated);
        assert!(
            e.response_body
                .text
                .starts_with(&format!("{}LATES", ".".repeat(20)))
        );
        e.rededact(&Redactor::new(&[late]), 25);
        assert_eq!(
            e.response_body.text,
            format!("{}{}", ".".repeat(20), marker(20, body.len() as u64))
        );
        assert_eq!(
            (e.response_body.size, e.response_body.truncated),
            (body.len() as u64, true)
        );
    }

    #[test]
    fn rededact_recuts_an_uncut_body_a_late_value_grew() {
        let mut e = entry_with(&Redactor::none(), "m", b"xbxb", 10);
        assert!(!e.response_body.truncated);
        e.rededact(&Redactor::new(&["b"]), 10);
        assert_eq!(e.response_body.text, format!("x[redacted{}", marker(10, 4)));
        assert_eq!((e.response_body.size, e.response_body.truncated), (4, true));
    }

    #[test]
    fn rededact_keeps_header_lists_within_their_cap() {
        let none = Redactor::none();
        let fat = vec![("X".to_owned(), "b".repeat(HEADERS_CAP - PAIR_OVERHEAD - 1))];
        let mut e = entry_with(&none, "v", b"", 1024);
        let hop = hop_with(&none, "https://api.test/", &fat, &fat);
        set_request_headers(&mut e, &none, &fat);
        assert_eq!(e.request_headers, fat);
        e.hops = cap_hops(vec![hop; HOPS_CAP]).0;
        e.rededact(&Redactor::new(&["b"]), 1024);
        let bytes = e.accounted_bytes();
        assert!(bytes <= 2_621_440, "{bytes}");
        assert_eq!(
            e.request_headers,
            vec![("…".to_owned(), "1 more headers".to_owned())]
        );
    }

    #[test]
    fn rededact_adds_newly_folded_pairs_to_the_overflow_count() {
        let none = Redactor::none();
        let headers: Vec<(String, String)> = (0..1000)
            .map(|i| (format!("X-H{i}"), "v".repeat(100)))
            .collect();
        let mut e = entry_with(&none, "m", b"", 1024);
        set_request_headers(&mut e, &none, &headers);
        e.rededact(&Redactor::new(&["v"]), 1024);
        let (overflow, pairs) = e.request_headers.split_last().unwrap();
        let cost: usize = pairs
            .iter()
            .map(|(n, v)| n.len() + v.len() + PAIR_OVERHEAD)
            .sum();
        assert!(cost <= HEADERS_CAP);
        assert_eq!(overflow.1, format!("{} more headers", 1000 - pairs.len()));
    }

    #[test]
    fn rededact_never_trusts_a_marker_shaped_body_or_url() {
        let fake = "abc\n[truncated: kept 3 of 1234567 bytes]";
        let none = Redactor::none();
        let mut e = entry_with(&none, "m", fake.as_bytes(), 1024);
        (e.url, e.url_cut) = logged_url(&none, fake);
        e.rededact(&Redactor::new(&["1234567"]), 1024);
        for t in entry_texts(&e) {
            assert!(!t.contains("1234567"), "{t}");
        }
        assert!(!e.response_body.truncated && !e.request_body.truncated);
        assert_eq!(e.response_body.size, fake.len() as u64);
    }

    #[test]
    fn rededact_trims_late_values_straddling_the_real_caps() {
        let late = "LATESECRET";
        let r = Redactor::new(&[late]);
        let none = Redactor::none();

        let cap = LogLimits::default().body_bytes;
        let body = format!("{}{late}tail", ".".repeat(cap - 5));
        let mut e = entry_with(&none, "m", body.as_bytes(), cap);
        let url = format!("https://api.test/{}", "u".repeat(URL_CAP - 17 - 5));
        let long_url = format!("{url}{late}zz");
        (e.hops[0].url, e.hops[0].url_cut) = logged_url(&none, &long_url);
        assert!(e.hops[0].url.starts_with(&format!("{url}LATES")));
        e.rededact(&r, cap);
        assert_eq!(
            e.response_body.text,
            format!(
                "{}{}",
                ".".repeat(cap - 5),
                marker(cap - 5, body.len() as u64)
            )
        );
        assert_eq!(
            e.hops[0].url,
            format!("{url}{}", marker(url.len(), long_url.len() as u64))
        );

        let text = format!("{}{late}t", "y".repeat(CONSOLE_CAP - 5));
        let mut l = line_with(&none, &text);
        l.rededact(&r);
        assert_eq!(
            l.text,
            format!(
                "{}{}",
                "y".repeat(CONSOLE_CAP - 5),
                marker(CONSOLE_CAP - 5, text.len() as u64)
            )
        );
    }

    #[test]
    fn rededact_never_cuts_into_the_old_marker() {
        // A four-byte character at the cap backs the cut off to K = CONSOLE_CAP - 3.
        let text = format!(
            "{}LATE\u{1D11E}SECRET{}",
            "y".repeat(CONSOLE_CAP - 7),
            "t".repeat(29)
        );
        let mut l = line_with(&Redactor::none(), &text);
        assert!(
            l.text
                .ends_with(&marker(CONSOLE_CAP - 3, text.len() as u64))
        );
        l.rededact(&Redactor::new(&["LATE\u{1D11E}SECRET", "1"]));
        assert!(!l.text.contains("LATE"), "{}", &l.text[CONSOLE_CAP - 20..]);
        assert_eq!(l.text.matches("\n[truncated: kept ").count(), 1);
    }

    #[test]
    fn a_real_header_named_like_the_overflow_pair_is_just_a_header() {
        let none = Redactor::none();
        let mut e = entry_with(&none, "m", b"", 1024);
        let real = [("…".to_owned(), "1234567 more headers".to_owned())];
        set_request_headers(&mut e, &none, &real);
        e.rededact(&Redactor::new(&["1234567"]), 1024);
        for t in entry_texts(&e) {
            assert!(!t.contains("1234567"), "{t}");
        }
    }

    #[test]
    fn a_huge_overflow_count_does_not_panic() {
        let none = Redactor::none();
        let mut e = entry_with(&none, "m", b"", 1024);
        let real = [
            ("A".to_owned(), "1".to_owned()),
            ("…".to_owned(), format!("{} more headers", usize::MAX)),
        ];
        set_request_headers(&mut e, &none, &real);
        set_overflow(&mut e, usize::MAX);
        e.rededact(&Redactor::new(&["zz"]), 1024);
        assert_eq!(e.request_headers.last().map(|p| p.0.as_str()), Some("…"));
    }
}
