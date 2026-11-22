//! A hop's body, decoded from its `Content-Encoding` here rather than by the HTTP stack, whose
//! decoder drops `Content-Encoding` and `Content-Length`. The response keeps the headers the
//! server sent, and the compressed bytes are counted as they arrive.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_compression::tokio::bufread::{BrotliDecoder, GzipDecoder, ZlibDecoder, ZstdDecoder};
use bytes::Bytes;
use futures_util::{StreamExt, stream};
use reqwest::header::{CONTENT_ENCODING, HeaderMap};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::io::StreamReader;

/// Decoded bytes handed out per read, so a small compressed chunk never inflates past the cap.
const DECODED_CHUNK: usize = 16 * 1024;

/// The content codings decoded, matched exactly as the HTTP stack's decoder did; any other
/// value, a list of codings included, leaves the body as it came.
#[derive(Clone, Copy)]
enum Coding {
    Gzip,
    Deflate,
    Brotli,
    Zstd,
}

impl Coding {
    fn of(headers: &HeaderMap) -> Option<Coding> {
        match headers.get(CONTENT_ENCODING)?.as_bytes() {
            b"gzip" => Some(Coding::Gzip),
            b"deflate" => Some(Coding::Deflate),
            b"br" => Some(Coding::Brotli),
            b"zstd" => Some(Coding::Zstd),
            _ => None,
        }
    }
}

/// Why reading a body failed: the connection, or bytes that do not decode under their coding.
#[derive(Debug)]
pub(super) enum BodyError {
    Wire(reqwest::Error),
    Decode(io::Error),
}

impl BodyError {
    // The decoder reports a wire failure as an io::Error wrapping the reqwest one; unwrap it so
    // it is classified like any other transport error.
    fn from_io(e: io::Error) -> BodyError {
        if !e
            .get_ref()
            .is_some_and(|inner| inner.is::<reqwest::Error>())
        {
            return BodyError::Decode(e);
        }
        match e
            .into_inner()
            .map(|inner| inner.downcast::<reqwest::Error>())
        {
            Some(Ok(wire)) => BodyError::Wire(*wire),
            Some(Err(other)) => BodyError::Decode(io::Error::other(other)),
            None => BodyError::Decode(io::Error::other("the body could not be read")),
        }
    }
}

type Decoder = Pin<Box<dyn AsyncRead + Send>>;

enum State {
    Plain(reqwest::Response),
    /// Compressed, with nothing read yet: an empty body never reaches a decoder, which would
    /// call it truncated.
    Pending(reqwest::Response, Coding),
    Decoding(Decoder),
    Done,
}

/// A hop's body read chunk by chunk as decoded bytes, counting the bytes that came over the wire.
pub(super) struct WireBody {
    state: State,
    wire: Arc<AtomicU64>,
    encoded: bool,
}

impl WireBody {
    /// Decodes `res` per its `Content-Encoding` when `decode`, else hands its bytes out as sent.
    pub(super) fn new(res: reqwest::Response, decode: bool) -> WireBody {
        let coding = decode.then(|| Coding::of(res.headers())).flatten();
        WireBody {
            encoded: coding.is_some(),
            state: match coding {
                Some(coding) => State::Pending(res, coding),
                None => State::Plain(res),
            },
            wire: Arc::default(),
        }
    }

    /// The compressed bytes read so far; `None` for a body that was not decoded or was empty.
    pub(super) fn encoded_size(&self) -> Option<u64> {
        let wire = self.wire.load(Ordering::Relaxed);
        (self.encoded && wire > 0).then_some(wire)
    }

    /// The next decoded chunk, or `None` at the end. Cancel safe: a dropped call loses no bytes.
    pub(super) async fn chunk(&mut self) -> Result<Option<Bytes>, BodyError> {
        loop {
            match &mut self.state {
                State::Plain(res) => {
                    let chunk = res.chunk().await.map_err(BodyError::Wire)?;
                    if let Some(chunk) = &chunk {
                        self.wire.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    }
                    return Ok(chunk);
                }
                State::Pending(res, coding) => {
                    let coding = *coding;
                    let first = res.chunk().await.map_err(BodyError::Wire)?;
                    // The response moves into the decoder only once its first chunk is in hand.
                    let state = std::mem::replace(&mut self.state, State::Done);
                    let (Some(first), State::Pending(res, _)) = (first, state) else {
                        return Ok(None);
                    };
                    self.wire.fetch_add(first.len() as u64, Ordering::Relaxed);
                    let wire = Arc::clone(&self.wire);
                    self.state = State::Decoding(decoder(res, first, coding, wire));
                }
                State::Decoding(decoder) => {
                    let mut buf = vec![0; DECODED_CHUNK];
                    let n = decoder.read(&mut buf).await.map_err(BodyError::from_io)?;
                    if n == 0 {
                        self.state = State::Done;
                        return Ok(None);
                    }
                    buf.truncate(n);
                    return Ok(Some(Bytes::from(buf)));
                }
                State::Done => return Ok(None),
            }
        }
    }
}

// The rest of the body follows `first` into the decoder, each chunk counted as it is pulled.
fn decoder(res: reqwest::Response, first: Bytes, coding: Coding, wire: Arc<AtomicU64>) -> Decoder {
    let rest = stream::unfold(res, |mut res| async move {
        match res.chunk().await {
            Ok(Some(chunk)) => Some((Ok(chunk), res)),
            Ok(None) => None,
            Err(e) => Some((Err(io::Error::other(e)), res)),
        }
    })
    .inspect(move |chunk| {
        if let Ok(chunk) = chunk {
            wire.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
    });
    let reader = StreamReader::new(stream::once(async { Ok(first) }).chain(rest));
    match coding {
        Coding::Gzip => Box::pin(GzipDecoder::new(reader)),
        Coding::Deflate => Box::pin(ZlibDecoder::new(reader)),
        Coding::Brotli => Box::pin(BrotliDecoder::new(reader)),
        Coding::Zstd => {
            let mut zstd = ZstdDecoder::new(reader);
            zstd.multiple_members(true);
            Box::pin(zstd)
        }
    }
}
