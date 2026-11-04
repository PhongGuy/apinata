//! Fetching a PAC script: directly over http(s) or from disk, bounded in size and time.

use std::path::PathBuf;
use std::time::Duration;

use reqwest::Client;
use reqwest::redirect::Policy;
use tokio::io::AsyncReadExt;
use url::Url;

use super::PacSource;
use crate::document::open_regular_async;
use crate::execute::chain;
use crate::network::read_ca;

/// How a PAC file is fetched: the TLS settings of the send, plus fixed size and time bounds.
#[derive(Debug, Clone)]
pub(crate) struct FetchOptions {
    pub(crate) insecure: bool,
    pub(crate) ca_file: Option<PathBuf>,
    pub(crate) keep_default_roots: bool,
    pub(crate) timeout: Duration,
    pub(crate) max_bytes: u64,
}

impl FetchOptions {
    /// Options with the standard bounds: 5 s per fetch and a 1 MiB file.
    pub(crate) fn new(insecure: bool, ca_file: Option<PathBuf>, keep_default_roots: bool) -> Self {
        FetchOptions {
            insecure,
            ca_file,
            keep_default_roots,
            timeout: Duration::from_secs(5),
            max_bytes: 1 << 20,
        }
    }
}

/// The script text: http(s) fetched directly (never through a proxy, at most 3 redirects, 2xx
/// only), file:// read from disk; lossy UTF-8. Errors read `<redacted source>: <cause>`.
pub(crate) async fn fetch(source: &PacSource, opts: &FetchOptions) -> Result<String, String> {
    let bytes = match Url::parse(source.as_str()) {
        Ok(url) if url.scheme() == "file" => read_file(&url, opts).await,
        Ok(url) => download(url, opts).await,
        Err(e) => Err(e.to_string()),
    };
    // The prefix is the redacted source, so credentials in the URL never reach the message.
    bytes
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .map_err(|cause| format!("{source}: {cause}"))
}

fn too_large(max_bytes: u64) -> String {
    format!("the file is larger than {max_bytes} bytes")
}

async fn read_file(url: &Url, opts: &FetchOptions) -> Result<Vec<u8>, String> {
    let path = url
        .to_file_path()
        .map_err(|()| "not a usable file path".to_owned())?;
    let file = open_regular_async(&path).await.map_err(|e| e.to_string())?;
    let meta = file.metadata().await.map_err(|e| e.to_string())?;
    if meta.len() > opts.max_bytes {
        return Err(too_large(opts.max_bytes));
    }
    let mut bytes = Vec::new();
    file.take(opts.max_bytes + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    // The file may have grown since the size check.
    if bytes.len() as u64 > opts.max_bytes {
        return Err(too_large(opts.max_bytes));
    }
    Ok(bytes)
}

async fn download(url: Url, opts: &FetchOptions) -> Result<Vec<u8>, String> {
    let transport = |e: reqwest::Error| {
        if e.is_timeout() {
            format!("timed out after {:?}", opts.timeout)
        } else {
            chain(e)
        }
    };
    let mut response = client(opts)?.get(url).send().await.map_err(transport)?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > opts.max_bytes)
    {
        return Err(too_large(opts.max_bytes));
    }
    // Stop reading as soon as the body passes the cap, whatever the server announced.
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if (body.len() + chunk.len()) as u64 > opts.max_bytes {
            return Err(too_large(opts.max_bytes));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// A one-off client that never uses a proxy and follows at most 3 redirects, all to http(s).
fn client(opts: &FetchOptions) -> Result<Client, String> {
    let policy = Policy::custom(|attempt| {
        if !matches!(attempt.url().scheme(), "http" | "https") {
            attempt.error("redirect to a non-http URL")
        } else if attempt.previous().len() > 3 {
            attempt.error("too many redirects")
        } else {
            attempt.follow()
        }
    });
    // A PAC file is one small GET; it never needs h2, whatever the send policy is.
    let mut builder = Client::builder()
        .no_proxy()
        .http1_only()
        .timeout(opts.timeout)
        .tls_danger_accept_invalid_certs(opts.insecure)
        .redirect(policy);
    if let Some(path) = &opts.ca_file {
        let roots = read_ca(path).map_err(|e| e.to_string())?;
        builder = if opts.keep_default_roots {
            builder.tls_certs_merge(roots)
        } else {
            builder.tls_certs_only(roots)
        };
    }
    builder.build().map_err(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn opts() -> FetchOptions {
        FetchOptions::new(false, None, true)
    }

    fn source(text: &str) -> PacSource {
        PacSource::parse(text, None).expect("source")
    }

    async fn serving(route: &str, response: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(path(route))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    #[test]
    fn the_bounds_are_five_seconds_and_one_mebibyte() {
        let o = opts();
        assert_eq!((o.timeout, o.max_bytes), (Duration::from_secs(5), 1 << 20));
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_file_source_is_refused() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let pipe = dir.path().join("p.pac");
        crate::hang_guard::mkfifo(&pipe);
        let text = pipe.to_str().expect("utf-8").to_owned();
        let fetched = crate::hang_guard::off_the_pipe(&pipe, move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            runtime.block_on(fetch(&source(&text), &opts()))
        });
        let err = fetched.expect_err("a pipe is no script file");
        assert!(err.contains("not a regular file"), "{err}");
    }

    #[tokio::test]
    async fn a_2xx_body_is_the_script_decoded_lossily() {
        let body = ResponseTemplate::new(200).set_body_bytes(b"f\xffx".to_vec());
        let server = serving("/p.pac", body).await;
        let got = fetch(&source(&format!("{}/p.pac", server.uri())), &opts()).await;
        assert_eq!(got, Ok("f\u{FFFD}x".to_owned()));
    }

    #[tokio::test]
    async fn a_non_2xx_status_fails_naming_the_redacted_source() {
        let server = serving("/p.pac", ResponseTemplate::new(404)).await;
        let address = server.address();
        let src = source(&format!("http://user:s3cret@{address}/p.pac"));
        let err = fetch(&src, &opts()).await.expect_err("404");
        assert_eq!(err, format!("http://***@{address}/p.pac: HTTP 404"));
    }

    #[tokio::test]
    async fn three_redirects_are_followed_and_a_fourth_is_refused() {
        let server = MockServer::start().await;
        for (from, to) in [
            ("/a1", "/a2"),
            ("/a2", "/a3"),
            ("/a3", "/p.pac"),
            ("/b1", "/a1"),
        ] {
            Mock::given(path(from))
                .respond_with(ResponseTemplate::new(302).insert_header("location", to))
                .mount(&server)
                .await;
        }
        Mock::given(path("/p.pac"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let three = fetch(&source(&format!("{}/a1", server.uri())), &opts()).await;
        assert_eq!(three, Ok("ok".to_owned()));
        let err = fetch(&source(&format!("{}/b1", server.uri())), &opts())
            .await
            .expect_err("four redirects");
        assert!(err.contains("redirect"), "{err}");
    }

    #[tokio::test]
    async fn a_redirect_off_http_is_refused() {
        let hop =
            ResponseTemplate::new(302).insert_header("location", "ftp://u:secret@a.test/p.pac");
        let server = serving("/p.pac", hop).await;
        let err = fetch(&source(&format!("{}/p.pac", server.uri())), &opts())
            .await
            .expect_err("ftp redirect");
        assert!(err.contains("redirect to a non-http URL"), "{err}");
        assert!(!err.contains("secret"), "{err}");
    }

    #[tokio::test]
    async fn a_failed_redirect_never_shows_the_locations_credentials() {
        let server = MockServer::start().await;
        let address = server.address();
        // One redirect lands on a 404; the other loops past the redirect cap.
        for (from, to) in [
            ("/missing", format!("http://u:secret@{address}/gone")),
            ("/loop", format!("http://u:secret@{address}/loop")),
        ] {
            Mock::given(path(from))
                .respond_with(ResponseTemplate::new(302).insert_header("location", to.as_str()))
                .mount(&server)
                .await;
        }
        for route in ["/missing", "/loop"] {
            let err = fetch(&source(&format!("{}{route}", server.uri())), &opts())
                .await
                .expect_err(route);
            assert!(!err.contains("secret"), "{route}: {err}");
        }
    }

    #[tokio::test]
    async fn a_body_over_the_cap_fails() {
        let server = MockServer::start().await;
        Mock::given(path("/fits"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(16)))
            .mount(&server)
            .await;
        Mock::given(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(17)))
            .mount(&server)
            .await;
        let small = FetchOptions {
            max_bytes: 16,
            ..opts()
        };
        let fits = fetch(&source(&format!("{}/fits", server.uri())), &small).await;
        assert_eq!(fits, Ok("x".repeat(16)));
        let err = fetch(&source(&format!("{}/big", server.uri())), &small)
            .await
            .expect_err("too big");
        assert!(err.contains("larger than 16 bytes"), "{err}");
    }

    #[tokio::test]
    async fn a_silent_server_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("addr");
        // Connections are held open and never answered.
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        let quick = FetchOptions {
            timeout: Duration::from_millis(300),
            ..opts()
        };
        let start = Instant::now();
        let err = fetch(&source(&format!("http://{address}/p.pac")), &quick)
            .await
            .expect_err("silent");
        assert!(err.contains("timed out"), "{err}");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn file_sources_are_read_under_the_same_cap() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("p.pac");
        std::fs::write(&file, "x".repeat(17)).expect("write");
        let path = file.to_str().expect("utf-8");
        assert_eq!(fetch(&source(path), &opts()).await, Ok("x".repeat(17)));
        let small = FetchOptions {
            max_bytes: 16,
            ..opts()
        };
        let err = fetch(&source(path), &small).await.expect_err("too big");
        assert!(err.contains("larger than 16 bytes"), "{err}");
        let missing = dir.path().join("absent.pac");
        let err = fetch(&source(missing.to_str().expect("utf-8")), &opts())
            .await
            .expect_err("missing");
        assert!(err.contains("absent.pac"), "{err}");
    }
}
