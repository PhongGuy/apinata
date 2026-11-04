//! `apinata send`: one request from flags or a curl command, sent without a collection.
//!
//! The request is written into a private temporary collection and sent through the same path
//! as `apinata run`, so options, output and exit codes are shared. Nothing lands in the
//! working directory unless `--save` names a request file inside a collection.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use apinata_core::import::{ImportedRequest, NoteKind, RequestFormat, add_request, parse_request};
use apinata_core::{BodySpec, Collection, ItemPath, MultipartValue, StatusRule};
use clap::Parser as _;

use crate::{EXIT_USAGE, EnvVar, Format, NetworkArgs, RunArgs, parse_env_var, run};

#[derive(clap::Args)]
pub struct SendArgs {
    /// The URL to request; may hold `{{variables}}`. Omit it with --curl.
    #[arg(value_name = "URL")]
    pub url: Option<String>,
    /// The HTTP method. Defaults to GET, or POST when there is -d data.
    #[arg(short = 'X', long = "request", value_name = "METHOD")]
    pub method: Option<String>,
    /// A header as `Name: value`; repeatable.
    #[arg(short = 'H', long = "header", value_name = "HEADER")]
    pub header: Vec<String>,
    /// The request body; repeatable, joined with `&` as curl does.
    #[arg(short = 'd', long = "data", value_name = "DATA")]
    pub data: Vec<String>,
    /// Send this curl command instead of URL, -X, -H and -d; `-` reads it from stdin.
    #[arg(long, value_name = "COMMAND|-")]
    pub curl: Option<String>,
    /// Also write the request to this file, which must be inside a collection and must not exist.
    #[arg(long, value_name = "PATH")]
    pub save: Option<PathBuf>,
    /// Also save the response as an example named NAME in the --save file, scrubbed of secret
    /// values as `apinata run --save-example` does.
    #[arg(long, value_name = "NAME", requires = "save")]
    pub save_example: Option<String>,
    /// With --save-example, keep the response exactly as received, secret values included.
    #[arg(long, requires = "save_example")]
    pub keep_secrets: bool,
    /// Set a `{{variable}}`; repeatable. `KEY=VALUE`, or `KEY` to read the value from the process
    /// environment. There is no collection, so there is no --env.
    #[arg(long = "env-var", value_name = "KEY[=VALUE]", value_parser = parse_env_var)]
    pub env_var: Vec<EnvVar>,
    /// Do not read secret values from the platform keychain.
    #[arg(long)]
    pub no_keychain: bool,
    /// The report format written to stdout; text prints only the response body.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
    /// Accept invalid TLS certificates.
    #[arg(long)]
    pub insecure: bool,
    /// Do not store or send cookies.
    #[arg(long)]
    pub disable_cookies: bool,
    /// Cap the request's timeout at this many whole seconds.
    #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u64).range(1..=86_400))]
    pub timeout: Option<u64>,
    /// Send again, up to N more times, when the send fails in transit; see `apinata run --help`.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub retries: u32,
    /// Fail a reply whose status is in range (error, 4xx or 5xx), written as --fail-on-status=4xx.
    #[arg(
        long,
        value_name = "RANGE",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "error"
    )]
    pub fail_on_status: Option<StatusRule>,
    /// Print each network exchange to stderr.
    #[arg(long)]
    pub verbose: bool,
    /// Print the status line, headers and body to stdout.
    #[arg(short = 'i', long)]
    pub show_response: bool,
    /// Print only the response body (the default for text output).
    #[arg(long, conflicts_with = "show_response")]
    pub body: bool,
    /// Narrow the printed body to what a JSONPath (JSON) or XPath (XML) matches; see
    /// `apinata run --help`. Cannot be combined with --format json.
    #[arg(long, value_name = "EXPR")]
    pub filter: Option<String>,
    /// Write the response body bytes exactly as received to this file.
    #[arg(short = 'o', long, value_name = "PATH")]
    pub output: Option<PathBuf>,
    #[command(flatten)]
    pub network: NetworkArgs,
}

/// Send the request; every failure before a response exits 3, then `run`'s codes apply.
pub fn run(args: SendArgs) -> ExitCode {
    match prepare(args) {
        Ok((run_args, _temp)) => {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(EXIT_USAGE);
                }
            };
            ExitCode::from(runtime.block_on(crate::run_once(&run_args)))
        }
        Err(e) => {
            if !e.is::<Reported>() {
                eprintln!("error: {e:#}");
            }
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// An error whose message was already printed.
#[derive(Debug)]
struct Reported;

impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("already reported")
    }
}

impl std::error::Error for Reported {}

/// Parse and check everything, save if asked, and return the `run` arguments for the request
/// together with the temporary directory that must outlive the run.
fn prepare(mut args: SendArgs) -> anyhow::Result<(RunArgs, tempfile::TempDir)> {
    let text = command_text(&args)?;
    let mut parsed = parse_request(Some(RequestFormat::Curl), &text)?;
    check_url(&parsed.request.url)?;
    // File bodies and file parts are read from where the user stands, not from the temporary
    // collection, so the scratch run gets absolute paths.
    map_file_paths(&mut parsed.request.body, |path| {
        Ok(std::path::absolute(path)
            .with_context(|| format!("cannot resolve {path}"))?
            .to_string_lossy()
            .into_owned())
    })?;
    // Flags the user typed are not a curl command, so notes about converting one are noise.
    if args.curl.is_none() {
        parsed.notes.retain(|n| n.kind != NoteKind::Converted);
    }
    for note in &mut parsed.notes {
        note.file = None;
    }
    crate::import::print_notes(&parsed.notes);
    let save = match args.save.take() {
        Some(path) => Some(save_target(&path)?),
        None => None,
    };

    // The temporary collection is created only now, after every check that costs nothing.
    let temp = tempfile::tempdir().context("cannot create a temporary directory")?;
    let mut scratch = Collection::create(&temp.path().join("send"), "send")?;
    let item = add_request(&mut scratch, &ItemPath::root(), "request", &parsed.request)?;
    let file = temp.path().join("send").join(item.as_str());

    let mut run_args = run_args(args, &file)?;
    // `validate` has already printed its reason; the empty error only carries the exit.
    if crate::validate(&run_args).is_some() {
        return Err(anyhow!(Reported));
    }
    if let Some((root, parent, name)) = save {
        // A saved file runs from any directory, so one inside the collection is kept relative.
        let mut saved = parsed.request.clone();
        map_file_paths(&mut saved.body, |path| {
            Ok(apinata_core::collection_relative(&root, Path::new(path)))
        })?;
        let item = write_saved(&root, &parent, &name, &saved)?;
        run_args.example_target = Some((root, item));
    }
    run_args.path = Some(file);
    Ok((run_args, temp))
}

/// Rewrite every file path the body names: a file body's entries and multipart file parts.
fn map_file_paths(
    body: &mut BodySpec,
    mut map: impl FnMut(&str) -> anyhow::Result<String>,
) -> anyhow::Result<()> {
    match body {
        BodySpec::File(entries) => {
            for entry in entries {
                entry.path = map(&entry.path)?;
            }
        }
        BodySpec::Multipart(fields) => {
            for field in fields {
                if let MultipartValue::Files(paths) = &mut field.value {
                    for path in paths {
                        *path = map(path)?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The curl command to parse: the --curl text, or one built from URL, -X, -H and -d.
fn command_text(args: &SendArgs) -> anyhow::Result<String> {
    if let Some(curl) = &args.curl {
        if args.url.is_some()
            || args.method.is_some()
            || !args.header.is_empty()
            || !args.data.is_empty()
        {
            bail!("--curl cannot be combined with a URL, -X, -H or -d");
        }
        if curl != "-" {
            return Ok(curl.clone());
        }
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .context("cannot read stdin")?;
        return Ok(text);
    }
    let url = args
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("give a URL, or a curl command with --curl"))?;
    let mut words = vec!["curl".to_owned()];
    if let Some(method) = &args.method {
        words.extend(["-X".to_owned(), quote(method)]);
    }
    for header in &args.header {
        words.extend(["-H".to_owned(), quote(header)]);
    }
    for data in &args.data {
        words.extend(["-d".to_owned(), quote(data)]);
    }
    words.push(quote(url));
    Ok(words.join(" "))
}

/// `word` as one POSIX single-quoted shell word.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// Refuse a URL that could never be sent: blank, spaced, or without a host.
fn check_url(url: &str) -> anyhow::Result<()> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("not a usable URL: {url}");
    }
    Ok(())
}

/// Where `--save` writes: the collection root, the folder below it and the request's name.
/// Fails when no collection holds the path or the file exists.
fn save_target(path: &Path) -> anyhow::Result<(PathBuf, ItemPath, String)> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|_| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("yml" | "yaml")
            )
        })
        .ok_or_else(|| anyhow!("--save needs a .yml file path, got {}", path.display()))?
        .to_owned();
    if path.exists() {
        bail!("{} already exists; --save never overwrites", path.display());
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    let dir = std::fs::canonicalize(dir.unwrap_or(Path::new(".")))
        .with_context(|| format!("cannot open the folder of {}", path.display()))?;
    let root = run::find_root(&dir).map_err(|e| {
        anyhow!("{e:#}; --save writes inside a collection (see `apinata import request`)")
    })?;
    let relative = dir.strip_prefix(root).expect("the root is an ancestor");
    let parent = if relative.as_os_str().is_empty() {
        ItemPath::root()
    } else {
        let raw = relative.to_string_lossy().replace('\\', "/");
        ItemPath::new(&raw).ok_or_else(|| anyhow!("invalid folder: {raw}"))?
    };
    Ok((root.to_path_buf(), parent, name))
}

/// Add the request to the collection at `root`, say where it went and return its item.
fn write_saved(
    root: &Path,
    parent: &ItemPath,
    name: &str,
    request: &ImportedRequest,
) -> anyhow::Result<ItemPath> {
    let mut collection = Collection::open(root)?;
    let item = add_request(&mut collection, parent, name, request)?;
    eprintln!("saved {}", root.join(item.as_str()).display());
    Ok(item)
}

/// The `run` arguments for `file`, taking the shared options from `args`.
fn run_args(args: SendArgs, file: &Path) -> anyhow::Result<RunArgs> {
    let cli = crate::Cli::try_parse_from([
        std::ffi::OsStr::new("apinata"),
        std::ffi::OsStr::new("run"),
        file.as_os_str(),
    ])?;
    let crate::Command::Run(mut run) = cli.command else {
        unreachable!("the arguments name `run`");
    };
    // Text output prints just the body unless -i asks for the head; the JSON report stands alone.
    let json = matches!(args.format, Format::Json);
    run.show_response = args.show_response && !json;
    run.body = !args.show_response && !json;
    run.env_var = args.env_var;
    run.no_keychain = args.no_keychain;
    run.format = args.format;
    run.insecure = args.insecure;
    run.disable_cookies = args.disable_cookies;
    run.timeout = args.timeout;
    run.retries = args.retries;
    run.fail_on_status = args.fail_on_status;
    run.verbose = args.verbose;
    run.filter = args.filter;
    run.output = args.output;
    run.network = args.network;
    run.save_example = args.save_example;
    run.keep_secrets = args.keep_secrets;
    Ok(*run)
}
