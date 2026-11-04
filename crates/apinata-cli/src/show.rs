//! Printing and saving the responses of `run`: `--show-response`, `--body` and `-o`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use apinata_core::{ItemPath, RequestResult, response_view};

/// What goes to stdout for each finished request.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    /// Status line, headers, a blank line and the body.
    Head,
    /// The body alone.
    Body,
}

/// Where `-o` writes: one file (the last body wins) or a directory of numbered files.
#[derive(Clone)]
enum Output {
    File(PathBuf),
    Dir(PathBuf),
}

impl Output {
    /// The file for the request that is number `n` of the run (counting from 1).
    fn path_for(&self, n: usize, item: &ItemPath) -> PathBuf {
        match self {
            Output::File(path) => path.clone(),
            Output::Dir(dir) => dir.join(format!("{n:03}-{}.body", slug(item.as_str()))),
        }
    }
}

/// The response output of one run, owned by the finished-request callback.
pub struct Show {
    mode: Mode,
    tty: bool,
    /// A JSONPath or XPath that narrows each printed body.
    filter: Option<String>,
    output: Option<Output>,
    /// Requests finished so far, shared with the `-o` resolver so both number alike.
    count: Arc<AtomicUsize>,
    failed: Arc<AtomicBool>,
    /// Addresses of the results whose body the filter could not read, for the caller to mark
    /// errored once the run has ended.
    unfiltered: Arc<Mutex<Vec<usize>>>,
}

impl Show {
    /// Set up the output for a run. `tty` says stdout is a terminal. An existing directory, or
    /// a path ending in a separator, becomes a directory (created now); any other path is a
    /// file whose parent must already exist.
    pub fn new(
        mode: Mode,
        tty: bool,
        filter: Option<String>,
        output: Option<&Path>,
    ) -> Result<Show, String> {
        let output = match output {
            None => None,
            Some(path) => Some(prepare(path)?),
        };
        Ok(Show {
            mode,
            tty,
            filter,
            output,
            count: Arc::new(AtomicUsize::new(0)),
            failed: Arc::new(AtomicBool::new(false)),
            unfiltered: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// The results the filter could not read, by address (see `RequestResult` in the report).
    pub fn unfiltered(&self) -> Arc<Mutex<Vec<usize>>> {
        Arc::clone(&self.unfiltered)
    }

    /// A handle that reads true once any `-o` write has failed.
    pub fn failure_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.failed)
    }

    /// The `-o` file for each request, for core to write as the body arrives. Requests run one
    /// at a time, so the next number is the finished count plus one, skipped requests included.
    pub fn body_files(&self) -> Option<impl Fn(usize, &ItemPath) -> Option<PathBuf> + use<>> {
        let output = self.output.clone()?;
        let count = Arc::clone(&self.count);
        Some(move |_: usize, item: &ItemPath| {
            Some(output.path_for(count.load(Ordering::Relaxed) + 1, item))
        })
    }

    /// Print and save the response of a finished request.
    pub fn request_finished(&mut self, result: &RequestResult) {
        let number = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        // A body the filter cannot read prints nothing; the request is marked errored later.
        match render(result, self.mode, self.tty, self.filter.as_deref()) {
            Ok(out) if !out.is_empty() => crate::out::write_bytes(&out),
            Ok(_) => {}
            Err(e) => {
                eprintln!("error: {}: {e}", result.path);
                if let Ok(mut list) = self.unfiltered.lock() {
                    list.push(std::ptr::from_ref(result) as usize);
                }
            }
        }
        if let Err(e) = self.save(result, number) {
            eprintln!("error: cannot write response: {e}");
            self.failed.store(true, Ordering::Relaxed);
        }
    }

    /// Core already wrote the file of a streamed request; only a response without one is
    /// written here.
    fn save(&self, result: &RequestResult, number: usize) -> std::io::Result<()> {
        let (Some(output), Some(response)) = (&self.output, &result.response) else {
            return Ok(());
        };
        if response.body_file().is_some() {
            return Ok(());
        }
        std::fs::write(output.path_for(number, &result.path), response.body())
    }
}

fn prepare(path: &Path) -> Result<Output, String> {
    let text = path.to_string_lossy();
    if path.is_dir() || text.ends_with('/') || text.ends_with(std::path::MAIN_SEPARATOR) {
        std::fs::create_dir_all(path)
            .map_err(|e| format!("cannot create output directory {}: {e}", path.display()))?;
        return Ok(Output::Dir(path.to_path_buf()));
    }
    // A bare file name has an empty parent, which means the current directory.
    let parent = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(dir) = parent
        && !dir.is_dir()
    {
        return Err(format!("output directory {} does not exist", dir.display()));
    }
    Ok(Output::File(path.to_path_buf()))
}

/// The file stem of an item path with every run of other characters folded into one `-`.
fn slug(item_path: &str) -> String {
    let stem = item_path.rsplit_once('.').map_or(item_path, |(s, _)| s);
    let mut out = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// The stdout bytes for one finished request under `mode`. Text is redacted as the session
/// log is. On a terminal a JSON body is indented and a binary one becomes a size line; piped,
/// the body bytes are never reformatted. With a `filter` the body is the redacted text narrowed
/// to the matches, one trailing newline included; a body the filter cannot read is an error.
pub fn render(
    result: &RequestResult,
    mode: Mode,
    tty: bool,
    filter: Option<&str>,
) -> Result<Vec<u8>, String> {
    let Some(response) = result.response.as_ref().filter(|_| mode != Mode::Off) else {
        return Ok(Vec::new());
    };
    let redact = |t: &str| result.redact(t);
    let mut out = Vec::new();
    if mode == Mode::Head {
        out.extend(response_view::head(response, &redact).bytes());
        out.push(b'\n');
    }
    let body = response.body();
    if let Some(expression) = filter {
        let content_type = response
            .headers()
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str());
        let filtered = match std::str::from_utf8(body) {
            Ok(text) => apinata_core::filter_body(content_type, &redact(text), expression),
            Err(_) => Err(apinata_core::BodyFilterError::Unfilterable {
                content_type: Some("binary".to_owned()),
            }),
        }
        .map_err(|e| format!("cannot apply --filter: {e}"))?;
        // A JSON escape can hide a secret from the first pass until the filter decodes it.
        let filtered = redact(&filtered);
        // Matches are our own output, so unlike a verbatim body they end on a line.
        out.extend(filtered.bytes());
        if !filtered.is_empty() && !filtered.ends_with('\n') {
            out.push(b'\n');
        }
        return Ok(out);
    }
    match std::str::from_utf8(body) {
        Err(_) if tty => out.extend(response_view::binary_placeholder(response.size()).bytes()),
        Err(_) => out.extend_from_slice(body),
        Ok(text) => {
            let text = redact(text);
            match tty
                .then(|| response_view::pretty_json(text.as_bytes()))
                .flatten()
            {
                Some(pretty) => out.extend(pretty.bytes()),
                None => out.extend(text.bytes()),
            }
        }
    }
    // A head is followed by the next request's, and a terminal prompt follows the body, so
    // each must start on a fresh line; a piped `--body` stays verbatim.
    if (mode == Mode::Head || tty) && out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apinata_core::{ItemPath, Outcome, Response};

    fn result(body: &[u8]) -> RequestResult {
        let response = Response::new(200)
            .with_status_text("OK")
            .with_body(body.to_vec(), "");
        let path = ItemPath::new("a.yml").expect("path");
        RequestResult::from_parts(path, "A", Outcome::Passed, Some(response), Vec::new())
    }

    fn text(result: &RequestResult, mode: Mode, tty: bool) -> String {
        String::from_utf8(render(result, mode, tty, None).expect("renders")).expect("utf-8")
    }

    #[test]
    fn a_terminal_gets_indented_json_and_a_final_newline() {
        let r = result(br#"{"a":1}"#);
        assert_eq!(text(&r, Mode::Body, true), "{\n  \"a\": 1\n}\n");
    }

    #[test]
    fn a_terminal_keeps_number_text_as_sent() {
        let r = result(br#"{"id":12345678901234567890123,"a":1.50,"b":1e3}"#);
        assert_eq!(
            text(&r, Mode::Body, true),
            "{\n  \"id\": 12345678901234567890123,\n  \"a\": 1.50,\n  \"b\": 1e3\n}\n"
        );
    }

    #[test]
    fn a_pipe_gets_the_body_untouched() {
        let r = result(br#"{"a":1}"#);
        assert_eq!(text(&r, Mode::Body, false), r#"{"a":1}"#);
    }

    #[test]
    fn a_terminal_never_gets_binary_bytes_under_either_mode() {
        let r = result(&[0xff, 0x00, 0xfe]);
        let line = "[binary body, 3 bytes; use -o FILE]\n";
        assert_eq!(text(&r, Mode::Body, true), line);
        assert!(text(&r, Mode::Head, true).ends_with(&format!("\n\n{line}")));
        assert_eq!(
            render(&r, Mode::Body, false, None).unwrap(),
            [0xff, 0x00, 0xfe]
        );
    }

    #[test]
    fn off_prints_nothing_and_slugs_fold_separators() {
        assert!(
            render(&result(b"x"), Mode::Off, true, None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(slug("users/get one.yml"), "users-get-one");
    }
}
