//! The text of the response status line's hover cards: the size split into header block and
//! body, and the connection the final hop came over. Each line reads `Label  value`, as the
//! timing breakdown does.

use apinata_core::Response;

use crate::body_text::{encoded_size_label, format_size};

/// The size card: the header block when a hop recorded it, the body (with its wire size when
/// compressed), and the cut point when the body was cut. `size_text` and `cut` are the status
/// line's own labels, so the card and the label agree.
pub fn size_lines(r: &Response, size_text: &str, cut: Option<&str>) -> Vec<String> {
    let head = r.timeline().last().map(|hop| hop.response_header_size());
    let body = match r.encoded_size().filter(|&wire| wire > 0) {
        Some(wire) => encoded_size_label(wire, r.size()),
        None => size_text.to_owned(),
    };
    let head = head.map(|bytes| format!("Headers  {}", format_size(bytes)));
    let cut = cut.map(|cut| format!("Cut at {cut}"));
    head.into_iter()
        .chain([format!("Body  {body}")])
        .chain(cut)
        .collect()
}

/// The connection card for the final hop: the remote (or proxy) and local address, the HTTP
/// version and, over TLS, the version and the peer certificate. Empty for a response with no
/// hops, as one restored from history.
pub fn connection_lines(r: &Response) -> Vec<String> {
    let Some(hop) = r.timeline().last() else {
        return Vec::new();
    };
    let remote = if hop.via_proxy() { "Proxy" } else { "Remote" };
    let mut lines: Vec<String> = [
        hop.remote_addr().map(|a| format!("{remote}  {a}")),
        hop.local_addr().map(|a| format!("Local  {a}")),
        hop.version().map(|v| format!("Version  {v}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if let Some(tls) = hop.tls() {
        let version = tls
            .version
            .map_or_else(|| "unknown".to_owned(), |v| v.to_string());
        lines.push(format!("TLS  {version}"));
        lines.extend(tls.subject.as_ref().map(|s| format!("Subject  {s}")));
        lines.extend(tls.issuer.as_ref().map(|s| format!("Issuer  {s}")));
        lines.extend(tls.expires_label().map(|at| format!("Expires  {at}")));
    }
    lines
}
