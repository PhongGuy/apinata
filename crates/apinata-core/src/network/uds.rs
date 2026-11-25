//! HTTP over a Unix socket, written `http://unix:/path/to.sock:/resource`. The URL keeps the
//! host `unix`, so a client that cannot read the form fails on that host rather than sending
//! to TCP localhost; the socket path is split off at the first colon of the URL path.

use std::path::Path;
use std::sync::Arc;

use percent_encoding::percent_decode_str;
use url::Url;

/// A socket URL taken apart: the socket file, the URL to put on the wire, and the socket path
/// as written in the URL, so a relative redirect can be rebuilt on the same socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SocketTarget {
    pub(crate) socket: String,
    pub(crate) wire: Url,
    written: String,
}

impl SocketTarget {
    /// The file as the client connects to it.
    pub(crate) fn path(&self) -> Arc<Path> {
        Arc::from(Path::new(&self.socket))
    }

    /// The name the host policy and the trust lists know this socket by.
    pub(crate) fn trust_key(&self) -> String {
        format!("unix:{}", self.socket)
    }
}

/// The socket `url` names, `None` for any other URL, or why the socket form is malformed.
pub(crate) fn split(url: &Url) -> Result<Option<SocketTarget>, String> {
    let (written, rest) = match url.path().split_once(':') {
        Some(parts) if url.host_str() == Some("unix") && url.port().is_none() => parts,
        _ => return Ok(None),
    };
    if !matches!(url.scheme(), "http" | "https") {
        return Ok(None);
    }
    if url.scheme() == "https" {
        return Err(
            "HTTPS over a Unix socket is not supported; use http://unix:/path:/resource".to_owned(),
        );
    }
    if !rest.starts_with('/') {
        return Err(format!(
            "a Unix socket URL needs a request path starting with / after the socket path, \
             as in http://unix:{written}:/resource"
        ));
    }
    let socket = percent_decode_str(written)
        .decode_utf8()
        .map_err(|_| "a Unix socket path must be UTF-8 text".to_owned())?
        .into_owned();
    if socket.len() < 2 || socket.contains('\0') {
        return Err(format!("`{socket}` is not a Unix socket path"));
    }
    let mut text = format!("http://localhost{rest}");
    if let Some(query) = url.query() {
        text.push('?');
        text.push_str(query);
    }
    let wire = Url::parse(&text).map_err(|e| format!("bad Unix socket request path: {e}"))?;
    Ok(Some(SocketTarget {
        socket,
        wire,
        written: written.to_owned(),
    }))
}

/// Where a redirect from `url` to `location` goes. A relative location from a socket URL stays
/// on that socket; a protocol-relative or absolute one is taken as written.
pub(crate) fn join(url: &Url, location: &str) -> Option<Url> {
    let Ok(Some(target)) = split(url) else {
        return url.join(location).ok();
    };
    // The URL parser drops edge controls and spaces, and tabs and newlines anywhere, before it
    // looks for an authority, and reads a backslash as a slash in http.
    let bare: String = location
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .take(2)
        .collect();
    let authority = bare.chars().all(|c| c == '/' || c == '\\') && bare.len() == 2;
    if authority || Url::parse(location).is_ok() {
        return url.join(location).ok();
    }
    let moved = target.wire.join(location).ok()?;
    let rest = &moved[url::Position::BeforePath..url::Position::AfterQuery];
    Url::parse(&format!("http://unix:{}:{rest}", target.written)).ok()
}

/// Whether `a` and `b` share an origin, the socket counting as part of it.
pub(crate) fn same_origin(a: &Url, b: &Url) -> bool {
    let socket = |url: &Url| split(url).ok().flatten().map(|t| t.socket);
    a.origin() == b.origin() && socket(a) == socket(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> Result<Option<SocketTarget>, String> {
        split(&Url::parse(url).unwrap())
    }

    #[test]
    fn the_socket_is_split_off_at_the_first_colon() {
        let t = target("http://unix:/var/run/docker.sock:/v1.47/a:b?x=1#f")
            .unwrap()
            .unwrap();
        assert_eq!(t.socket, "/var/run/docker.sock");
        assert_eq!(t.wire.as_str(), "http://localhost/v1.47/a:b?x=1");
        assert_eq!(t.trust_key(), "unix:/var/run/docker.sock");
    }

    #[test]
    fn a_percent_encoded_socket_path_is_decoded() {
        let t = target("http://unix:/tmp/my%20dir/a%3Ab.sock:/x")
            .unwrap()
            .unwrap();
        assert_eq!(t.socket, "/tmp/my dir/a:b.sock");
    }

    #[test]
    fn other_urls_are_not_sockets() {
        for url in [
            "http://unix/x",
            "http://unix:8080/a:b",
            "http://example.com/a:/b",
            "ftp://unix/a:/b",
        ] {
            assert_eq!(target(url), Ok(None), "{url}");
        }
    }

    #[test]
    fn malformed_socket_urls_are_refused() {
        for url in [
            "http://unix:/s.sock:x",
            "http://unix:/%FF.sock:/x",
            "http://unix:/%00:/x",
            "http://unix:/:/x",
            "https://unix:/s.sock:/x",
        ] {
            assert!(target(url).is_err(), "{url}");
        }
    }

    #[test]
    fn redirects_keep_or_leave_the_socket() {
        let from = Url::parse("http://unix:/s%20.sock:/a/b?q=1").unwrap();
        let to = |l: &str| join(&from, l).unwrap().to_string();
        assert_eq!(to("/c?d=2"), "http://unix/s%20.sock:/c?d=2");
        assert_eq!(to("c"), "http://unix/s%20.sock:/a/c");
        assert_eq!(to("//evil.test/x"), "http://evil.test/x");
        assert_eq!(to("\\\\evil.test/x"), "http://evil.test/x");
        assert_eq!(to("http://other.test/"), "http://other.test/");
    }
}
