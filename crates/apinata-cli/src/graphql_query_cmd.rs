//! `apinata graphql fmt` and `apinata graphql validate`: format or check a GraphQL query file or
//! a GraphQL request's query.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apinata_core::{
    BodySpec, CompiledSchema, QuerySeverity, QuerySyntaxError, RequestFile, RequestKind,
    SCHEMA_BODY_LIMIT, Schema, format_query, validate_query,
};

use crate::EXIT_USAGE;

/// Exit code for a query that does not format, a `--check` that finds a change, or a query with
/// an error diagnostic.
const EXIT_FINDING: u8 = 1;

#[derive(clap::Args)]
pub struct FmtArgs {
    /// A `.graphql` or `.gql` query file, or a GraphQL request file.
    pub path: PathBuf,
    /// Write nothing; exit 1 if the file would change.
    #[arg(long)]
    pub check: bool,
}

#[derive(clap::Args)]
pub struct ValidateArgs {
    /// A `.graphql` or `.gql` query file, or a GraphQL request file.
    pub path: PathBuf,
    /// Check against this schema: SDL or an introspection JSON answer. Without it only syntax is
    /// checked.
    #[arg(long, value_name = "PATH")]
    pub schema_file: Option<PathBuf>,
}

/// The 1-based line and char column of byte `offset` in `text`.
pub(crate) fn position(text: &str, offset: usize) -> (usize, usize) {
    let (mut line, mut col) = (1, 1);
    for (i, c) in text.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Whether `path` names raw query text rather than a request file.
fn is_query_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("graphql") || e.eq_ignore_ascii_case("gql"))
}

pub fn fmt(args: &FmtArgs) -> ExitCode {
    ExitCode::from(run_fmt(args))
}

fn usage(message: impl std::fmt::Display) -> u8 {
    eprintln!("error: {message}");
    EXIT_USAGE
}

/// Print a formatter failure with its position (`prefix` ends with the position's colon form).
fn report(error: &QuerySyntaxError, text: &str, locate: impl Fn(usize, usize) -> String) -> u8 {
    match error {
        QuerySyntaxError::Syntax { range, .. } => {
            let (line, col) = position(text, range.start);
            eprintln!("error: {} {error}", locate(line, col));
        }
        other => eprintln!("error: {other}"),
    }
    EXIT_FINDING
}

fn run_fmt(args: &FmtArgs) -> u8 {
    let shown = args.path.display();
    if is_query_file(&args.path) {
        let text = match std::fs::read_to_string(&args.path) {
            Ok(text) => text,
            Err(e) => return usage(format!("cannot read {shown}: {e}")),
        };
        let formatted = match format_query(&text) {
            Ok(formatted) => formatted,
            Err(e) => return report(&e, &text, |l, c| format!("{shown}:{l}:{c}:")),
        };
        if formatted == text {
            return 0;
        }
        if args.check {
            eprintln!("{shown}: would be reformatted");
            return EXIT_FINDING;
        }
        return match std::fs::write(&args.path, formatted) {
            Ok(()) => 0,
            Err(e) => usage(format!("cannot write {shown}: {e}")),
        };
    }

    let mut file = match RequestFile::load(&args.path) {
        Ok(file) => file,
        Err(e) => return usage(format!("cannot open {shown}: {e}")),
    };
    if file.kind() != RequestKind::Graphql {
        return usage(format!("{shown} is not a GraphQL request"));
    }
    let BodySpec::Graphql { query, variables } = file.body() else {
        return usage(format!("{shown} is not a GraphQL request"));
    };
    let formatted = match format_query(&query) {
        Ok(formatted) => formatted,
        Err(e) => return report(&e, &query, |l, c| format!("{shown} (query) {l}:{c}:")),
    };
    if formatted == query {
        return 0;
    }
    if args.check {
        eprintln!("{shown}: would be reformatted");
        return EXIT_FINDING;
    }
    file.set_body(BodySpec::Graphql {
        query: formatted,
        variables,
    });
    match file.save() {
        Ok(()) => 0,
        Err(e) => usage(format!("cannot write {shown}: {e}")),
    }
}

pub fn validate(args: &ValidateArgs) -> ExitCode {
    ExitCode::from(run_validate(args))
}

/// The query text of `path` and how a position in it is shown, or the exit code of a refusal.
fn load_query(path: &Path) -> Result<(String, Locate), u8> {
    let shown = path.display().to_string();
    if is_query_file(path) {
        return match std::fs::read_to_string(path) {
            Ok(text) => Ok((text, Locate::File(shown))),
            Err(e) => Err(usage(format!("cannot read {shown}: {e}"))),
        };
    }
    let file = RequestFile::load(path).map_err(|e| usage(format!("cannot open {shown}: {e}")))?;
    match file.body() {
        BodySpec::Graphql { query, .. } if file.kind() == RequestKind::Graphql => {
            Ok((query, Locate::Request(shown)))
        }
        _ => Err(usage(format!("{shown} is not a GraphQL request"))),
    }
}

/// How a line and column are printed: after a query file's path, or after a request's path.
enum Locate {
    File(String),
    Request(String),
}

impl Locate {
    fn at(&self, line: usize, col: usize) -> String {
        match self {
            Locate::File(shown) => format!("{shown}:{line}:{col}:"),
            Locate::Request(shown) => format!("{shown} (query) {line}:{col}:"),
        }
    }
}

/// The compiled schema in `path`, `None` when it parses but cannot be used for checking, or the
/// exit code of an unreadable, oversized or unparsable file.
fn load_schema(path: &Path) -> Result<Option<CompiledSchema>, u8> {
    let shown = path.display();
    let len = std::fs::metadata(path)
        .map_err(|e| usage(format!("cannot read {shown}: {e}")))?
        .len();
    if len > SCHEMA_BODY_LIMIT {
        return Err(usage(format!(
            "{shown} is larger than {} MiB",
            SCHEMA_BODY_LIMIT / (1024 * 1024)
        )));
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| usage(format!("cannot read {shown}: {e}")))?;
    let schema = Schema::from_text(&text).map_err(|e| usage(format!("{shown}: {e}")))?;
    match CompiledSchema::new(&schema) {
        Ok(compiled) => Ok(Some(compiled)),
        Err(e) => {
            eprintln!("note: the schema cannot be used for checking: {e}");
            Ok(None)
        }
    }
}

/// `text` with every control character replaced, so a message stays on one terminal line.
fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}

fn run_validate(args: &ValidateArgs) -> u8 {
    let (text, locate) = match load_query(&args.path) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    let schema = match args.schema_file.as_deref().map(load_schema).transpose() {
        Ok(schema) => schema.flatten(),
        Err(code) => return code,
    };
    let diagnostics = validate_query(&text, schema.as_ref());
    let mut code = 0;
    for diagnostic in &diagnostics {
        let severity = match diagnostic.severity() {
            QuerySeverity::Error => {
                code = EXIT_FINDING;
                "error"
            }
            QuerySeverity::Warning => "warning",
            _ => "info",
        };
        let (line, col) = position(&text, diagnostic.range().start);
        outln!(
            "{} {severity}: {}",
            locate.at(line, col),
            printable(diagnostic.message())
        );
    }
    code
}
