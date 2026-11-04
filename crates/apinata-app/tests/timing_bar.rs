//! The pure logic of the response timing bar and the network dock's per-hop line.

use apinata_app::timing_bar::{PhaseKind, Phases, hop_line, segments, tooltip_lines};
use apinata_core::ConnectKind;
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn fresh() -> Phases {
    Phases {
        dns: Some(ms(10)),
        connect: Some(ms(20)),
        connect_kind: ConnectKind::Connect,
        via_proxy: false,
        wait: ms(60),
        download: ms(10),
        reused: false,
    }
}

#[test]
fn fractions_are_proportional_and_sum_to_one() {
    let segs = segments(&fresh());
    let kinds: Vec<_> = segs.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [
            PhaseKind::Dns,
            PhaseKind::Connect,
            PhaseKind::Wait,
            PhaseKind::Download
        ]
    );
    assert!((segs[2].fraction - 0.6).abs() < 1e-6);
    let sum: f32 = segs.iter().map(|s| s.fraction).sum();
    assert!((sum - 1.0).abs() < 1e-5);
}

#[test]
fn absent_phases_are_omitted() {
    let p = Phases {
        dns: None,
        download: Duration::ZERO,
        ..fresh()
    };
    let kinds: Vec<_> = segments(&p).iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [PhaseKind::Connect, PhaseKind::Wait]);
}

#[test]
fn a_reused_connection_has_no_connect_phase_and_a_note() {
    let p = Phases {
        dns: None,
        connect: None,
        reused: true,
        ..fresh()
    };
    let kinds: Vec<_> = segments(&p).iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [PhaseKind::Wait, PhaseKind::Download]);
    let lines = tooltip_lines(&p);
    assert_eq!(lines[0], "Connection reused");
    assert!(lines.iter().all(|l| !l.starts_with("Connect ")));
    assert_eq!(lines[1], "Wait  60 ms");
    assert_eq!(
        hop_line(&p),
        "Connection reused · Wait 60 ms · Download 10 ms"
    );
}

#[test]
fn connect_labels_follow_the_kind_and_the_proxy() {
    let label = |kind, via_proxy| {
        let p = Phases {
            connect_kind: kind,
            via_proxy,
            ..fresh()
        };
        segments(&p)[1].label.clone()
    };
    assert_eq!(label(ConnectKind::Connect, false), "Connect");
    assert_eq!(label(ConnectKind::ConnectAndTls, false), "Connect and TLS");
    assert_eq!(label(ConnectKind::Connect, true), "Connect (via proxy)");
    assert_eq!(
        label(ConnectKind::ConnectAndTls, true),
        "Connect and TLS (via proxy)"
    );
}

#[test]
fn a_zero_total_has_no_segments() {
    let p = Phases {
        dns: None,
        connect: None,
        wait: Duration::ZERO,
        download: Duration::ZERO,
        ..fresh()
    };
    assert!(segments(&p).is_empty());
    assert!(tooltip_lines(&p).is_empty());
}

#[test]
fn lines_use_the_shared_time_format() {
    assert_eq!(
        hop_line(&fresh()),
        "DNS 10 ms · Connect 20 ms · Wait 60 ms · Download 10 ms"
    );
    assert_eq!(tooltip_lines(&fresh())[0], "DNS  10 ms");
}
