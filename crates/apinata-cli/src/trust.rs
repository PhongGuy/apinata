//! `apinata trust`: edit one collection's trusted and denied hosts in the per-user trust file.
//! `apinata trust scripts` approves the collection's script texts in the script trust file.
//! Every write reloads the file first and a file that fails to load is never saved over.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use apinata_core::{
    Collection, ContextOptions, ScriptDecision, ScriptTextKind, ScriptTrust, TrustError,
    TrustStore, VarContext, bare_host_refusal, is_hidden_char, is_variation_selector,
    normalize_bare_host, plan_script_texts,
};

use crate::collection_arg;
use crate::run::find_root;
use crate::{EXIT_USAGE, TrustAction, TrustArgs};

/// Apply `args` and print its outcome: exit 0 on success, 3 with `error: ...` on any failure.
pub fn run(args: &TrustArgs) -> ExitCode {
    // Scripts never load the hosts file and print as they go, since they ask on stdin.
    let outcome = match &args.action {
        TrustAction::Scripts { yes } => scripts(args, *yes),
        _ => apply(args).map(|lines| {
            for line in lines {
                outln!("{}", shown(&line));
            }
        }),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", shown(&format!("{e:#}")));
            ExitCode::from(EXIT_USAGE)
        }
    }
}

// Each change is applied to a store loaded just before it and saved at once; any error
// (bad host, missing collection, unreadable or malformed file) returns before `save`.
fn apply(args: &TrustArgs) -> anyhow::Result<Vec<String>> {
    let root = collection_arg::with_hint(
        collection_root(args.collection.as_deref()),
        "pass the collection with --collection <DIR>",
    )?;
    let file = match &args.trust_file {
        Some(file) => file.clone(),
        None => TrustStore::default_path().ok_or(TrustError::NoDirectory)?,
    };
    let mut store = TrustStore::load(file)?;
    let shown_root = root.display();
    let line = match &args.action {
        TrustAction::Add { host } => {
            let host = bare_host(host)?;
            store.trust(&root, &host)?;
            store.save()?;
            format!("trusted {host} for {shown_root}")
        }
        TrustAction::Deny { host } => {
            let host = bare_host(host)?;
            store.deny(&root, &host)?;
            store.save()?;
            format!("denied {host} for {shown_root}")
        }
        TrustAction::Remove { host } => {
            let host = bare_host(host)?;
            if !store.remove(&root, &host) {
                return Ok(vec![format!("{host} was not listed for {shown_root}")]);
            }
            store.save()?;
            format!("removed {host} for {shown_root}")
        }
        TrustAction::List => {
            let trusted = store
                .trusted(&root)
                .into_iter()
                .map(|h| format!("trusted {h}"));
            let denied = store
                .denied(&root)
                .into_iter()
                .map(|h| format!("denied {h}"));
            return Ok(trusted.chain(denied).collect());
        }
        TrustAction::Scripts { .. } => unreachable!("`run` handles scripts"),
    };
    Ok(vec![line])
}

// Plan the whole collection as a fresh run would, list each text the trust file does not
// approve, and on confirmation allow scripts and approve exactly those hashes.
fn scripts(args: &TrustArgs, yes: bool) -> anyhow::Result<()> {
    let root = collection_arg::with_hint(
        collection_root(args.collection.as_deref()),
        "pass the collection with --collection <DIR>",
    )?;
    let file = match &args.script_trust_file {
        Some(file) => file.clone(),
        None => ScriptTrust::default_path().ok_or(TrustError::NoDirectory)?,
    };
    let trust = ScriptTrust::load(&file)?;
    let collection = Collection::open(&root)?;
    let ctx = VarContext::new(&collection, ContextOptions::new())?;
    let plan = collection.plan(collection.root())?;
    let pending: Vec<_> = plan_script_texts(&plan, &ctx)
        .into_iter()
        .filter(|(_, t)| !trust.is_approved(&root, &t.hash))
        .collect();
    let shown_root = shown(&root.display().to_string());
    if pending.is_empty() {
        outln!("no scripts need approval for {shown_root}");
        return Ok(());
    }
    for (item, text) in &pending {
        let label = match &text.kind {
            ScriptTextKind::Script { phase, level } => {
                format!("{} script ({level})", phase.as_str())
            }
            ScriptTextKind::Variable { name } => format!("variable {name}"),
            ScriptTextKind::AssertionLeft => "assertion left side".to_owned(),
            ScriptTextKind::AssertionRight => "assertion right side".to_owned(),
            _ => "script text".to_owned(),
        };
        outln!("{}", shown(&format!("{item}: {label}")));
        for line in text.shown.split('\n') {
            outln!("    {}", shown_keeping_tabs(line));
        }
    }
    let n = pending.len();
    let count = if n == 1 {
        "1 script".to_owned()
    } else {
        format!("{n} scripts")
    };
    let question = format!("Approve {count} and allow scripts for {shown_root}? [y/N] ");
    if !yes && !confirm(&question) {
        outln!("nothing was approved");
        return Ok(());
    }
    // Updated under the trust file's lock after the question, so a change the app saved
    // meanwhile is kept; the hashes approved are still exactly those shown.
    ScriptTrust::update(&file, |trust| {
        trust.set_decision(&root, Some(ScriptDecision::Allowed));
        trust.approve(&root, pending.iter().map(|(_, t)| &t.hash));
    })?;
    outln!("approved {count} for {shown_root}");
    Ok(())
}

/// Asks `question` on stdout; only `y` or `yes`, in any case, agrees, and end of input declines.
pub(crate) fn confirm(question: &str) -> bool {
    out!("{question}");
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    let answer = answer.trim();
    answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
}

/// The canonical collection root containing `--collection`, or the working directory, found
/// as `apinata mcp` finds it, so the key matches the one the app and server use.
pub(crate) fn collection_root(collection: Option<&Path>) -> anyhow::Result<PathBuf> {
    let start = match collection {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir().context("cannot read the working directory")?,
    };
    let start = std::fs::canonicalize(&start)
        .with_context(|| format!("cannot open {}", start.display()))?;
    Ok(find_root(&start)?.to_path_buf())
}

/// [`collection_root`] for the listing commands, which refuse a `--collection` that is a file
/// rather than quietly listing the collection around it.
pub(crate) fn directory_root(collection: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(path) = collection
        && !path.is_dir()
    {
        bail!("{} is not a directory", path.display());
    }
    collection_root(collection)
}

/// `typed` as a normalised host, refused unless it is a bare host, so a URL or authority
/// never puts a host on a list other than the one the user read.
fn bare_host(typed: &str) -> anyhow::Result<String> {
    normalize_bare_host(typed).ok_or_else(|| anyhow!(bare_host_refusal(typed)))
}

/// `text` with each [`is_hidden_char`] written as a `\u{..}` escape and variation selectors
/// dropped, so echoed hosts and paths cannot drive or disguise the terminal.
pub(crate) fn shown(text: &str) -> String {
    text.chars()
        .filter(|&c| !is_variation_selector(c))
        .map(|c| {
            if is_hidden_char(c) {
                c.escape_unicode().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// One line of script text as [`shown`] writes it, but with tabs kept for indentation.
fn shown_keeping_tabs(line: &str) -> String {
    line.split('\t').map(shown).collect::<Vec<_>>().join("\t")
}
