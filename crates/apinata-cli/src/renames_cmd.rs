//! `apinata env renames`: the stored secrets a pulled rename left under an old name, listed by
//! name and status and moved on request. A value is never printed.

use std::path::Path;
use std::process::ExitCode;

use anyhow::anyhow;
use apinata_core::{
    Collection, EnvError, GitOptions, PullRange, RenameRefusal, RenameStatus, SecretName,
    SecretRename, SecretStore,
};

use crate::import::platform_store;
use crate::trust::{self, shown};
use crate::{EXIT_USAGE, collection_arg};

/// The exit code when something was skipped and needs a person.
const EXIT_SKIPPED: u8 = 1;

/// One `--pick`: move the value stored under `from` to `to`.
#[derive(Clone, Debug)]
pub struct Pick {
    from: SecretName,
    to: SecretName,
}

/// Parse `FROM_ENV/FROM_VAR=TO_ENV/TO_VAR`.
pub fn parse_pick(text: &str) -> Result<Pick, String> {
    let (from, to) = text
        .split_once('=')
        .ok_or_else(|| "expected FROM_ENV/FROM_VAR=TO_ENV/TO_VAR".to_owned())?;
    Ok(Pick {
        from: parse_name(from)?,
        to: parse_name(to)?,
    })
}

fn parse_name(text: &str) -> Result<SecretName, String> {
    match text.split_once('/') {
        Some((env, var)) if !env.is_empty() && !var.is_empty() => Ok(SecretName::new(env, var)),
        _ => Err(format!("`{text}` is not ENV/VAR")),
    }
}

/// Run the action: 0 when listed or everything asked for moved, 1 when a rename was skipped,
/// 3 for a usage, collection, git or store error.
pub fn run(
    collection: Option<&Path>,
    since: Option<&str>,
    move_ready: bool,
    picks: &[Pick],
) -> ExitCode {
    match go(collection, since, move_ready, picks) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn go(
    collection: Option<&Path>,
    since: Option<&str>,
    move_ready: bool,
    picks: &[Pick],
) -> anyhow::Result<ExitCode> {
    let root = collection_arg::with_hint(
        trust::directory_root(collection),
        "pass the collection with --collection <DIR>",
    )?;
    let c = Collection::open(&root)?;
    let opts = GitOptions::new();
    let range = PullRange::to_head(c.root(), since.unwrap_or("ORIG_HEAD"), &opts)?
        .ok_or_else(|| match since {
            Some(rev) => anyhow!("{} names no commit in the collection's repository", shown(rev)),
            None => anyhow!(
                "no ORIG_HEAD to start from; pass --since <REV>, the first id apinata git pull printed"
            ),
        })?;
    let store = platform_store();
    let found = c.secret_renames(&range, None, &*store, &opts)?;
    // Every pick is checked before anything moves, so a typo never leaves half the work done.
    for pick in picks {
        let row = (found.renames.iter())
            .find(|r| r.from == pick.from)
            .ok_or_else(|| anyhow!("no pending rename of {}", shown(&pick.from.to_string())))?;
        anyhow::ensure!(
            row.to.iter().any(|(to, _)| *to == pick.to),
            "{} is not a new name of {}",
            shown(&pick.to.to_string()),
            shown(&pick.from.to_string())
        );
    }
    if found.renames.is_empty() {
        outln!("no stored secrets to move");
        return Ok(ExitCode::SUCCESS);
    }
    if !move_ready && picks.is_empty() {
        list(&found.renames);
        return Ok(ExitCode::SUCCESS);
    }
    let mut tally = Tally::default();
    for pick in picks {
        tally.moved(&c, &pick.from, &pick.to, &*store);
    }
    let unpicked = (found.renames.iter()).filter(|r| !picks.iter().any(|p| p.from == r.from));
    for row in unpicked {
        match (move_ready, row.status(), row.to.as_slice()) {
            (true, RenameStatus::Ready, [(to, _)]) => tally.moved(&c, &row.from, to, &*store),
            (true, ..) => {
                outln!("skipped {}", line(row));
                tally.skipped = true;
            }
            (false, ..) => outln!("{}", line(row)),
        }
    }
    Ok(tally.code())
}

/// What the moves came to: a store failure outranks a skip, which outranks success.
#[derive(Default)]
struct Tally {
    skipped: bool,
    failed: bool,
}

impl Tally {
    /// Move `from` to `to` through core's write, read back, delete order and print the outcome.
    fn moved(
        &mut self,
        c: &Collection,
        from: &SecretName,
        to: &SecretName,
        store: &dyn SecretStore,
    ) {
        let pair = format!("{} → {}", spoken(from), spoken(to));
        match c.move_renamed_secret(from, to, store) {
            Ok(()) => outln!("moved {pair}"),
            // Gone from the old name since the scan, as when the app moved it meanwhile.
            Err(EnvError::RenameRefused {
                reason: RenameRefusal::NothingStored,
                ..
            }) => outln!("already moved {pair}"),
            Err(EnvError::RenameRefused {
                reason: RenameRefusal::Taken,
                ..
            }) => {
                outln!("skipped {pair}: taken");
                self.skipped = true;
            }
            Err(EnvError::RenameRefused { reason, .. }) => {
                outln!("skipped {pair}: {reason}");
                self.skipped = true;
            }
            Err(e) => {
                eprintln!("error: {e}");
                self.failed = true;
            }
        }
    }

    fn code(&self) -> ExitCode {
        match (self.failed, self.skipped) {
            (true, _) => ExitCode::from(EXIT_USAGE),
            (false, true) => ExitCode::from(EXIT_SKIPPED),
            (false, false) => ExitCode::SUCCESS,
        }
    }
}

/// Print each rename, then what to run for the ready and the ambiguous ones.
fn list(renames: &[SecretRename]) {
    for row in renames {
        outln!("{}", line(row));
    }
    if renames.iter().any(|r| r.status() == RenameStatus::Ready) {
        eprintln!("hint: add --move to move the ready ones");
    }
    let ambiguous = renames
        .iter()
        .find(|r| r.status() == RenameStatus::Ambiguous);
    if let Some(row) = ambiguous
        && let Some((to, _)) = row.to.first()
    {
        eprintln!(
            "hint: settle an ambiguous one with --pick {}",
            shown(&apinata_core::shell_word(&format!("{}={}", row.from, to)))
        );
    }
}

/// "dev / token → staging / token: ready", candidates joined by "or".
fn line(row: &SecretRename) -> String {
    let to: Vec<String> = row.to.iter().map(|(to, _)| spoken(to)).collect();
    let status = match row.status() {
        RenameStatus::Ready => "ready",
        RenameStatus::Taken => "taken",
        _ => "ambiguous",
    };
    format!("{} → {}: {status}", spoken(&row.from), to.join(" or "))
}

/// "env / var", escaped for the terminal.
fn spoken(name: &SecretName) -> String {
    format!("{} / {}", shown(&name.environment), shown(&name.variable))
}
