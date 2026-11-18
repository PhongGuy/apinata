//! `apinata init`: create an empty collection, the same files the desktop app's New collection writes.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::anyhow;
use apinata_core::{Collection, CreateError};

use crate::EXIT_USAGE;

#[derive(clap::Args)]
pub struct InitArgs {
    /// The directory to create the collection in; created when missing. Defaults to the working
    /// directory.
    #[arg(value_name = "DIR", default_value = ".")]
    pub dir: PathBuf,
    /// The collection's name; defaults to the directory's name.
    #[arg(long)]
    pub name: Option<String>,
}

/// Create the collection and print one line naming it; every failure exits 3.
pub fn run(args: &InitArgs) -> ExitCode {
    match create(args) {
        Ok(collection) => {
            outln!(
                "Created collection \"{}\" in {}",
                collection.name(),
                collection.root().display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn create(args: &InitArgs) -> anyhow::Result<Collection> {
    // Resolve the directory first so `.` or a trailing `..` still yields a real folder name,
    // which is what the app names a new collection after.
    let dir = std::fs::canonicalize(&args.dir).or_else(|_| std::path::absolute(&args.dir))?;
    let root_file = dir.join("opencollection.yml");
    if root_file.exists() {
        return Err(anyhow!(
            "{} already holds a collection ({} exists)",
            dir.display(),
            root_file.display()
        ));
    }
    let name = args.name.clone().unwrap_or_else(|| folder_name(&dir));
    Collection::create(&dir, &name).map_err(|e| match e {
        CreateError::NotEmpty { path } => anyhow!(
            "{} is not empty; choose an empty directory or a new one",
            path.display()
        ),
        e => anyhow!(e),
    })
}

/// The directory's last component, as the app names a collection after its folder.
fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
