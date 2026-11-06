//! `apinata graphql fmt`: format a GraphQL query file or a GraphQL request's query in place.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apinata_core::{BodySpec, QuerySyntaxError, RequestFile, RequestKind, format_query};

use crate::EXIT_USAGE;

/// Exit code for a query that does not format, or a `--check` that finds a change.
const EXIT_FINDING: u8 = 1;

#[derive(clap::Args)]
pub struct FmtArgs {
    /// A `.graphql` or `.gql` query file, or a GraphQL request file.
    pub path: PathBuf,
    /// Write nothing; exit 1 if the file would change.
    #[arg(long)]
    pub check: bool,
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
