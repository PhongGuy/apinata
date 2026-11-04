//! Building a network entry from what one send produced.

use super::entry::{
    cap_hops, logged_body_of, logged_error, logged_headers, logged_method, logged_url,
};
use super::redact::Redactor;
use super::{LoggedHop, NetworkEntry, SendSource};
use crate::ExecError;
use crate::response::{Hop, Response};
use std::time::{Duration, SystemTime};

/// What one send produced, as the executor saw it.
pub(crate) struct Exchange<'a> {
    pub(crate) source: SendSource,
    pub(crate) at: SystemTime,
    pub(crate) duration: Duration,
    pub(crate) method: &'a str,
    pub(crate) url: &'a str,
    pub(crate) headers: &'a [(String, String)],
    /// The first hop's kept body bytes and its original size, when it sent one.
    pub(crate) first_body: Option<(&'a [u8], u64)>,
    pub(crate) hops: &'a [Hop],
    pub(crate) outcome: Result<&'a Response, &'a ExecError>,
}

impl NetworkEntry {
    /// The exchange redacted with `r` and capped field by field, bodies at `body_cap`; `seq` is 0.
    pub(crate) fn capture(r: &Redactor, x: &Exchange<'_>, body_cap: usize) -> NetworkEntry {
        let (method, method_cut) = logged_method(r, x.method);
        let (url, url_cut) = logged_url(r, x.url);
        let (request_headers, request_folded) = logged_headers(r, x.headers);
        let (bytes, size) = x.first_body.unwrap_or((&[], 0));
        let request_body = logged_body_of(r, bytes, size, body_cap);
        let (status, response_body, error, error_cut) = match x.outcome {
            Ok(response) => {
                let body = logged_body_of(r, response.body(), response.size(), body_cap);
                (Some(response.status()), body, None, None)
            }
            Err(err) => {
                let (error, cut) = logged_error(r, &err.to_string());
                (None, logged_body_of(r, &[], 0, body_cap), Some(error), cut)
            }
        };
        // Cap the count before redacting, so a redirect loop costs only the hops kept.
        let (hops, hops_omitted) = cap_hops(x.hops.iter().collect());
        let hops = hops.into_iter().map(|h| LoggedHop::capture(r, h)).collect();
        NetworkEntry {
            seq: 0,
            at: x.at,
            source: x.source,
            method,
            url,
            status,
            error,
            duration: x.duration,
            request_headers,
            request_body,
            response_body,
            hops,
            hops_omitted,
            attempt: None,
            method_cut,
            url_cut,
            error_cut,
            request_folded,
        }
    }
}

impl LoggedHop {
    /// One completed hop redacted with `r` and capped field by field.
    pub(crate) fn capture(r: &Redactor, hop: &Hop) -> LoggedHop {
        let (method, method_cut) = logged_method(r, &hop.method);
        let (url, url_cut) = logged_url(r, &hop.url);
        let (request_headers, request_folded) = logged_headers(r, &hop.request_headers);
        let (response_headers, response_folded) = logged_headers(r, &hop.response_headers);
        // A fallback line quotes the script's error, which may echo the URL, so it is capped and
        // redacted like an error.
        let (pac, pac_cut) = match hop.pac.as_deref().map(|line| logged_error(r, line)) {
            Some((line, cut)) => (Some(line), cut),
            None => (None, None),
        };
        let (override_from, override_cut) =
            match hop.override_from().map(|line| logged_error(r, line)) {
                Some((line, cut)) => (Some(line), cut),
                None => (None, None),
            };
        LoggedHop {
            method,
            url,
            status: hop.status,
            request_headers,
            response_headers,
            elapsed: hop.elapsed,
            pac,
            override_from,
            timing: hop.timing.clone(),
            version: hop.version(),
            remote_addr: hop.remote_addr(),
            via_proxy: hop.via_proxy(),
            method_cut,
            url_cut,
            pac_cut,
            override_cut,
            request_folded,
            response_folded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExecError;
    use crate::response::{Hop, Response};
    use std::time::{Duration, Instant, SystemTime};

    fn hop(url: &str, status: u16, headers: &[(&str, &str)]) -> Hop {
        let pairs: Vec<(String, String)> = headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect();
        Hop {
            method: "GET".into(),
            url: url.into(),
            request_headers: pairs.clone(),
            status,
            response_headers: pairs,
            elapsed: Duration::from_millis(3),
            request_body: b"sent".to_vec(),
            request_body_size: 4,
            pac: None,
            timing: None,
            cookies: Vec::new(),
            version: None,
            remote_addr: None,
            via_proxy: false,
            override_from: None,
        }
    }

    fn exchange<'a>(
        hops: &'a [Hop],
        outcome: Result<&'a Response, &'a ExecError>,
        headers: &'a [(String, String)],
    ) -> Exchange<'a> {
        Exchange {
            source: SendSource::Runner,
            at: SystemTime::UNIX_EPOCH,
            duration: Duration::from_millis(9),
            method: "POST",
            url: "https://api.test/a?key=k-secret-1",
            headers,
            first_body: Some((b"body k-secret-1", 900)),
            hops,
            outcome,
        }
    }

    #[test]
    fn a_success_keeps_status_hops_and_both_bodies_redacted() {
        let r = Redactor::new(&["k-secret-1"]);
        let hops = [
            hop(
                "https://api.test/a?key=k-secret-1",
                302,
                &[("Authorization", "Bearer x")],
            ),
            hop("https://api.test/b", 200, &[]),
        ];
        let response = Response::new(200).with_body(b"got k-secret-1".to_vec(), "got k-secret-1");
        let headers = vec![("X-Key".to_owned(), "k-secret-1".to_owned())];
        let e = NetworkEntry::capture(&r, &exchange(&hops, Ok(&response), &headers), 64 * 1024);
        assert_eq!(
            (e.source, e.status, e.error.as_deref(), e.duration),
            (
                SendSource::Runner,
                Some(200),
                None,
                Duration::from_millis(9)
            )
        );
        assert_eq!(e.url, "https://api.test/a?key=[redacted]");
        assert_eq!(
            (e.request_body.text.as_str(), e.request_body.size),
            ("body [redacted]\n[truncated: kept 15 of 900 bytes]", 900)
        );
        assert_eq!(
            (e.response_body.text.as_str(), e.response_body.size),
            ("got [redacted]", 14)
        );
        assert_eq!(e.request_headers[0].1, "[redacted]");
        assert_eq!(
            (e.hops.len(), e.hops[0].url.as_str()),
            (2, "https://api.test/a?key=[redacted]")
        );
        assert_eq!(e.hops[0].request_headers[0].1, "[redacted]");
    }

    #[test]
    fn a_hop_keeps_its_timing() {
        use crate::{ConnectKind, HopTiming};
        let timing = HopTiming {
            dns: None,
            connect: Some(Duration::from_millis(2)),
            connect_kind: ConnectKind::Connect,
            via_proxy: false,
            wait: Duration::from_millis(5),
            download: Duration::from_millis(1),
            reused: false,
        };
        let mut timed = hop("https://api.test/a", 200, &[]);
        timed.timing = Some(timing.clone());
        let plain = hop("https://api.test/b", 200, &[]);
        let hops = [timed, plain];
        let response = Response::new(200);
        let e = NetworkEntry::capture(&Redactor::none(), &exchange(&hops, Ok(&response), &[]), 64);
        assert_eq!(e.hops[0].timing, Some(timing));
        assert_eq!(e.hops[1].timing, None);
    }

    #[test]
    fn a_failure_keeps_the_error_text_and_the_completed_hops() {
        let hops = [hop("https://api.test/a", 302, &[])];
        let err = ExecError::Cancelled;
        let e = NetworkEntry::capture(
            &Redactor::none(),
            &exchange(&hops, Err(&err), &[]),
            64 * 1024,
        );
        assert_eq!(
            (e.status, e.error.as_deref(), e.hops.len()),
            (None, Some("cancelled"), 1)
        );
        assert_eq!(
            (e.response_body.text.as_str(), e.response_body.size),
            ("", 0)
        );
    }

    #[test]
    fn hops_past_the_cap_keep_the_first_thirty_one_and_the_last() {
        let hops: Vec<Hop> = (0..41)
            .map(|i| {
                hop(
                    &format!("https://api.test/{i}"),
                    if i < 40 { 302 } else { 200 },
                    &[],
                )
            })
            .collect();
        let response = Response::new(200);
        let e = NetworkEntry::capture(
            &Redactor::none(),
            &exchange(&hops, Ok(&response), &[]),
            64 * 1024,
        );
        assert_eq!((e.hops.len(), e.hops_omitted), (32, 9));
        assert_eq!(e.hops[31].url, "https://api.test/40");
    }

    #[test]
    fn a_cut_body_reports_its_original_size_and_bytes_are_lossy() {
        let body = vec![b'x'; 200];
        let response = Response::new(200).with_body(vec![0xff, b'a'], "\u{FFFD}a");
        let mut x = exchange(&[], Ok(&response), &[]);
        x.first_body = Some((&body, 5000));
        let e = NetworkEntry::capture(&Redactor::none(), &x, 100);
        assert!(e.request_body.truncated);
        assert!(
            e.request_body.text.ends_with("of 5000 bytes]"),
            "{}",
            e.request_body.text
        );
        assert_eq!(
            (e.response_body.text.as_str(), e.response_body.size),
            ("\u{FFFD}a", 2)
        );
    }

    #[test]
    #[ignore = "measurement; run in release with --nocapture"]
    fn redaction_cost() {
        let values = ["a/b c\"d-1", "e/f g\"h-2", "i/j k\"l-3", "m/n o\"p-4"];
        let r = Redactor::new(&values);
        assert!(r.forms().len() >= 20, "{}", r.forms().len());
        let mut body = String::new();
        while body.len() < 64 * 1024 {
            body.push_str("lorem ipsum a/b c\"d-1 dolor ");
        }
        let headers: Vec<(String, String)> = (0..20)
            .map(|i| (format!("X-H{i}"), format!("v{i} e/f g\"h-2")))
            .collect();
        let hops: Vec<Hop> = (0..10)
            .map(|i| Hop {
                method: "GET".into(),
                url: format!("https://api.test/{i}?k=i/j%20k%22l-3"),
                request_headers: headers.clone(),
                status: if i < 9 { 302 } else { 200 },
                response_headers: headers.clone(),
                elapsed: Duration::from_millis(1),
                request_body: body.as_bytes().to_vec(),
                request_body_size: body.len() as u64,
                pac: None,
                timing: None,
                cookies: Vec::new(),
                version: None,
                remote_addr: None,
                via_proxy: false,
                override_from: None,
            })
            .collect();
        let response = Response::new(200).with_body(body.clone().into_bytes(), body.clone());
        let mut times: Vec<u128> = (0..200)
            .map(|_| {
                let started = Instant::now();
                let x = Exchange {
                    source: SendSource::Request,
                    at: SystemTime::now(),
                    duration: Duration::ZERO,
                    method: "GET",
                    url: &hops[0].url,
                    headers: &headers,
                    first_body: Some((body.as_bytes(), body.len() as u64)),
                    hops: &hops,
                    outcome: Ok(&response),
                };
                let mut e = NetworkEntry::capture(&r, &x, 64 * 1024);
                e.rededact(&r, 64 * 1024);
                started.elapsed().as_micros()
            })
            .collect();
        times.sort();
        println!(
            "redaction_cost: forms={} median={}us max={}us",
            r.forms().len(),
            times[100],
            times[199]
        );
    }
}
