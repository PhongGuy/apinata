//! `apinata export`: write a collection, and optionally its environments, as another tool's
//! format or as HTML documentation, printing a note for everything the target cannot carry.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use apinata_core::Collection;
use apinata_core::export::{self, ExportNote};

use crate::run::find_root;
use crate::{EXIT_USAGE, collection_arg};

#[derive(clap::Args)]
pub struct ExportArgs {
    /// The collection directory or a directory inside it.
    #[arg(value_name = "COLLECTION")]
    pub path: Option<PathBuf>,
    /// The same as the positional COLLECTION.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// The output format.
    #[arg(long, value_enum)]
    pub format: ExportFormat,
    /// The file to write, or `-` for standard output.
    #[arg(long, value_name = "FILE")]
    pub out: PathBuf,
    /// Also write each environment to this directory as <name>.postman_environment.json (Postman only).
    #[arg(long, value_name = "DIR")]
    pub env_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum ExportFormat {
    /// Postman v2.1 collection JSON.
    Postman,
    /// OpenAPI 3.1 JSON; environments become servers.
    Openapi,
    /// One self-contained HTML documentation page; environments are left out.
    Html,
}

/// Run `apinata export`; every failure is a usage error.
pub fn run(args: &ExportArgs) -> ExitCode {
    match export_collection(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn export_collection(args: &ExportArgs) -> anyhow::Result<()> {
    // Only Postman writes environment files: OpenAPI carries them as servers and the HTML page
    // leaves them out, so a separate environment directory is refused before anything is opened.
    if args.env_dir.is_some() {
        match args.format {
            ExportFormat::Postman => {}
            ExportFormat::Openapi => anyhow::bail!(
                "--env-dir does not apply to --format openapi; environments are carried as servers"
            ),
            ExportFormat::Html => anyhow::bail!(
                "--env-dir does not apply to --format html; the page leaves environments out"
            ),
        }
    }
    let named = collection_arg::require(
        args.path.as_deref(),
        args.collection.as_deref(),
        "`apinata export <COLLECTION>` or `apinata export --collection <DIR>`",
    )?;
    let start = named
        .canonicalize()
        .with_context(|| format!("cannot open {}", named.display()))?;
    let root = collection_arg::with_hint(
        find_root(&start),
        "pass the collection as `apinata export <COLLECTION>` or `apinata export --collection <DIR>`",
    )?;
    let collection = Collection::open(root)?;
    for error in collection.load_errors() {
        eprintln!("warning: {}: {}", error.path().display(), error.message());
    }
    let (text, notes) = match args.format {
        ExportFormat::Postman => {
            let exported = export::postman(&collection);
            (exported.json, exported.notes)
        }
        ExportFormat::Openapi => {
            let exported = export::openapi(&collection);
            (exported.json, exported.notes)
        }
        ExportFormat::Html => {
            let exported = export::html(&collection);
            (exported.html, exported.notes)
        }
    };
    print_notes(&notes);
    if args.out.as_os_str() == "-" {
        crate::out::write_bytes(text.as_bytes());
    } else {
        write_atomic(&args.out, &text)?;
    }
    if let Some(dir) = &args.env_dir {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        // Environments come sorted by name, so a colliding stem is numbered in a stable order.
        let mut taken = HashSet::new();
        for env in collection.environments() {
            let Some(exported) = export::postman_environment(&collection, env.name()) else {
                continue;
            };
            print_notes(&exported.notes);
            let stem = unique_stem(&mut taken, export::file_stem(env.name()));
            write_atomic(
                &dir.join(format!("{stem}.postman_environment.json")),
                &exported.json,
            )?;
        }
    }
    Ok(())
}

fn print_notes(notes: &[ExportNote]) {
    for note in notes {
        eprintln!("note: {}: {}", note.source, note.detail);
    }
}

/// `stem` if it is free, else the first free `stem (n)` from 2 up; the result is marked taken.
/// Names compare lower-cased, since Windows and macOS file systems ignore case.
fn unique_stem(taken: &mut HashSet<String>, stem: String) -> String {
    let mut candidate = stem.clone();
    let mut n = 2;
    while taken.contains(&candidate.to_lowercase()) {
        candidate = format!("{stem} ({n})");
        n += 1;
    }
    taken.insert(candidate.to_lowercase());
    candidate
}

/// The core's atomic export write, with the path in the error.
fn write_atomic(path: &Path, text: &str) -> anyhow::Result<()> {
    export::write_atomic(path, text.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_that_differ_only_in_case_do_not_collide() {
        let mut taken = HashSet::new();
        assert_eq!(unique_stem(&mut taken, "Dev".to_owned()), "Dev");
        assert_eq!(unique_stem(&mut taken, "dev".to_owned()), "dev (2)");
        assert_eq!(unique_stem(&mut taken, "DEV".to_owned()), "DEV (3)");
    }
}
