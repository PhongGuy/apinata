//! The pure logic behind the response timing bar and the network dock's per-hop line: which
//! phases were measured, their share of the total and how each is worded.

use std::time::Duration;

use apinata_core::{ConnectKind, HopTiming};

use crate::body_text::format_time;

/// The measured phases of one hop, copied out of core's `HopTiming`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phases {
    /// Name resolution, when measured.
    pub dns: Option<Duration>,
    /// Connecting, when measured.
    pub connect: Option<Duration>,
    /// What the connect phase covers.
    pub connect_kind: ConnectKind,
    /// The connect went through a proxy.
    pub via_proxy: bool,
    /// Time to first byte.
    pub wait: Duration,
    /// Headers to the end of the body.
    pub download: Duration,
    /// The hop used a pooled connection.
    pub reused: bool,
}

impl From<&HopTiming> for Phases {
    fn from(t: &HopTiming) -> Phases {
        Phases {
            dns: t.dns,
            connect: t.connect,
            connect_kind: t.connect_kind,
            via_proxy: t.via_proxy,
            wait: t.wait,
            download: t.download,
            reused: t.reused,
        }
    }
}

/// Which phase a segment shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseKind {
    /// Name resolution.
    Dns,
    /// Connect, with TLS when the target is https.
    Connect,
    /// Time to first byte.
    Wait,
    /// Body download.
    Download,
}

/// One measured phase: its label, time and share of the total.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The phase.
    pub kind: PhaseKind,
    /// Its label, such as "Connect and TLS".
    pub label: String,
    /// Its duration.
    pub time: Duration,
    /// Its share of the total, in 0..=1.
    pub fraction: f32,
}

fn connect_label(p: &Phases) -> String {
    let base = match p.connect_kind {
        ConnectKind::ConnectAndTls => "Connect and TLS",
        _ => "Connect",
    };
    if p.via_proxy {
        format!("{base} (via proxy)")
    } else {
        base.to_owned()
    }
}

/// The measured, non-zero phases in order with their shares of the total. A reused connection
/// drops DNS and connect. Empty when the total is zero, so nothing divides by zero.
pub fn segments(p: &Phases) -> Vec<Segment> {
    let (dns, connect) = if p.reused {
        (None, None)
    } else {
        (p.dns, p.connect)
    };
    let all = [
        (PhaseKind::Dns, "DNS".to_owned(), dns.unwrap_or_default()),
        (
            PhaseKind::Connect,
            connect_label(p),
            connect.unwrap_or_default(),
        ),
        (PhaseKind::Wait, "Wait".to_owned(), p.wait),
        (PhaseKind::Download, "Download".to_owned(), p.download),
    ];
    let total: f32 = all.iter().map(|(_, _, t)| t.as_secs_f32()).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    all.into_iter()
        .filter(|(_, _, time)| !time.is_zero())
        .map(|(kind, label, time)| Segment {
            kind,
            label,
            time,
            fraction: time.as_secs_f32() / total,
        })
        .collect()
}

const REUSED: &str = "Connection reused";

/// The hover breakdown: a "Connection reused" line first when it applies, then one
/// "label  time" line per measured phase.
pub fn tooltip_lines(p: &Phases) -> Vec<String> {
    let segs = segments(p);
    if segs.is_empty() {
        return Vec::new();
    }
    let note = p.reused.then(|| REUSED.to_owned());
    note.into_iter()
        .chain(
            segs.iter()
                .map(|s| format!("{}  {}", s.label, format_time(s.time))),
        )
        .collect()
}

/// The same breakdown on one line for the network dock, "Wait 60 ms · Download 10 ms".
pub fn hop_line(p: &Phases) -> String {
    let segs = segments(p);
    if segs.is_empty() {
        return String::new();
    }
    let note = p.reused.then(|| REUSED.to_owned());
    note.into_iter()
        .chain(
            segs.iter()
                .map(|s| format!("{} {}", s.label, format_time(s.time))),
        )
        .collect::<Vec<_>>()
        .join(" · ")
}
