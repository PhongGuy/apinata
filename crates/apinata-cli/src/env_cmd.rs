//! `apinata env`: list, show and edit environments and their variables, secrets masked.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apinata_core::{
    Collection, EnvError, EnvWarning, GlobalEnvironments, MovedRoot, OrphanReport, SecretStore,
    VarValue,
};
use serde::Serialize;

use crate::import::{platform_store, secret_store};
use crate::trust::{self, shown};
use crate::{EXIT_USAGE, EnvAction, EnvArgs, collection_arg};

/// A variable as JSON, the shape of the MCP `read_environment` variable: a secret has no value.
#[derive(Serialize)]
struct VarDoc {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    value_type: Option<String>,
    enabled: bool,
    secret: bool,
}

#[derive(Serialize)]
struct EnvDoc {
    name: String,
    variables: Vec<VarDoc>,
}

#[derive(Serialize)]
struct NamesDoc {
    environments: Vec<String>,
}

#[derive(Serialize)]
struct OrphansDoc {
    orphans: Vec<String>,
}

#[derive(Serialize)]
struct ReportRowDoc<'a> {
    environment: &'a str,
    variable: &'a str,
    has_file: bool,
}

/// `env orphans --all` as JSON.
#[derive(Serialize)]
struct ReportDoc<'a> {
    orphans: Vec<ReportRowDoc<'a>>,
    unreadable: &'a [String],
}

impl<'a> From<&'a OrphanReport> for ReportDoc<'a> {
    fn from(report: &'a OrphanReport) -> Self {
        let row = |o: &'a apinata_core::OrphanedSecret| ReportRowDoc {
            environment: &o.environment,
            variable: &o.variable,
            has_file: o.has_file,
        };
        ReportDoc {
            orphans: report.orphans.iter().map(row).collect(),
            unreadable: &report.unreadable,
        }
    }
}

/// The environments an orphan action works on. One is built per command, so size is no concern.
#[allow(clippy::large_enum_variant)]
enum Envs {
    Collection(Collection),
    Global(GlobalEnvironments),
    MovedRoot(MovedRoot),
}

impl Envs {
    /// The root at `old_root`, else the global environments with `global`, else the collection
    /// `args` names.
    fn open(args: &EnvArgs, global: bool, old_root: Option<&Path>) -> anyhow::Result<Envs> {
        if let Some(root) = old_root {
            anyhow::ensure!(
                args.collection.is_none(),
                "--old-root names a root that has gone, so it cannot be used with --collection"
            );
            return Ok(Envs::MovedRoot(MovedRoot::new(root).map_err(escaped)?));
        }
        if global {
            let dir = GlobalEnvironments::default_dir()
                .ok_or_else(|| anyhow::anyhow!("no config directory for global environments"))?;
            return Ok(Envs::Global(GlobalEnvironments::open(dir)?));
        }
        let root: PathBuf = collection_arg::with_hint(
            trust::directory_root(args.collection.as_deref()),
            "pass the collection with --collection <DIR>",
        )?;
        Ok(Envs::Collection(Collection::open(&root)?))
    }

    fn orphans(&self, name: &str, store: &dyn SecretStore) -> Result<Vec<String>, EnvError> {
        match self {
            Envs::Collection(c) => c.orphaned_secrets(name, store),
            Envs::Global(g) => g.orphaned_secrets(name, store),
            Envs::MovedRoot(m) => m.fileless_secrets(name, store),
        }
    }

    fn fileless(&self, name: &str, store: &dyn SecretStore) -> Result<Vec<String>, EnvError> {
        match self {
            Envs::Collection(c) => c.fileless_secrets(name, store),
            Envs::Global(g) => g.fileless_secrets(name, store),
            Envs::MovedRoot(m) => m.fileless_secrets(name, store),
        }
    }

    fn everywhere(&self, store: &dyn SecretStore) -> Result<OrphanReport, EnvError> {
        match self {
            Envs::Collection(c) => c.all_orphaned_secrets(store),
            Envs::Global(g) => g.all_orphaned_secrets(store),
            Envs::MovedRoot(m) => m.all_orphaned_secrets(store),
        }
    }

    fn remove(
        &self,
        name: &str,
        names: &[String],
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        match self {
            Envs::Collection(c) => c.remove_orphaned_secrets(name, names, store),
            Envs::Global(g) => g.remove_orphaned_secrets(name, names, store),
            Envs::MovedRoot(m) => m.remove_fileless_secrets(name, names, store),
        }
    }

    fn remove_fileless(
        &self,
        name: &str,
        names: &[String],
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        match self {
            Envs::Collection(c) => c.remove_fileless_secrets(name, names, store),
            Envs::Global(g) => g.remove_fileless_secrets(name, names, store),
            Envs::MovedRoot(m) => m.remove_fileless_secrets(name, names, store),
        }
    }
}

/// What one action works on: `name`'s orphans or, raw, whatever the store holds under `name`.
struct Target<'a> {
    envs: &'a Envs,
    name: &'a str,
    raw: bool,
}

impl<'a> Target<'a> {
    /// `--no-file` asks for raw mode; a gone root has no other.
    fn new(envs: &'a Envs, name: &'a str, no_file: bool) -> Target<'a> {
        let raw = no_file || matches!(envs, Envs::MovedRoot(_));
        Target { envs, name, raw }
    }

    fn listed(&self, store: &dyn SecretStore) -> anyhow::Result<Vec<String>> {
        if self.raw {
            return self.envs.fileless(self.name, store).map_err(no_file_hint);
        }
        self.envs.orphans(self.name, store).map_err(escaped)
    }

    fn remove(&self, names: &[String], store: &dyn SecretStore) -> anyhow::Result<Vec<EnvWarning>> {
        if self.raw {
            let removed = self.envs.remove_fileless(self.name, names, store);
            return removed.map_err(no_file_hint);
        }
        self.envs.remove(self.name, names, store).map_err(escaped)
    }

    /// What `--all` says when there is nothing to remove.
    fn nothing_line(&self) -> String {
        let name = shown(self.name);
        if self.raw {
            format!("no stored secrets under {name}")
        } else {
            format!("no orphaned secrets in {name}")
        }
    }

    /// What is printed above the question.
    fn listing_line(&self, list: &str) -> String {
        let name = shown(self.name);
        if self.raw {
            format!("stored secrets of {name}, which has no file: {list}")
        } else {
            format!("orphaned secrets of {name}: {list}")
        }
    }

    /// What every removal with no file must say: a directory on a drive that is not mounted reads
    /// exactly like a deleted one, so only these words can guard a live secret.
    fn out_of_reach(&self) -> Option<&'static str> {
        match self.envs {
            Envs::MovedRoot(_) => Some(
                "The collection may only be elsewhere, for example on a drive that is not mounted.",
            ),
            _ if self.raw => Some(
                "The environment's directory may only be elsewhere, for example on a drive that is not mounted.",
            ),
            _ => None,
        }
    }

    fn question(&self, list: &str) -> String {
        let elsewhere = self.out_of_reach().map(|w| format!(" {w}"));
        let elsewhere = elsewhere.unwrap_or_default();
        format!(
            "Remove the stored values of {list}? This cannot be undone, and another git branch may still use them.{elsewhere} [y/N] "
        )
    }
}

/// Run the action: 0 when printed, 3 for an unopenable collection or an unknown environment.
pub fn run(args: &EnvArgs) -> ExitCode {
    match go(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn go(args: &EnvArgs) -> anyhow::Result<()> {
    // Both work without a collection under `--global`, so they come before `go_with` opens one.
    match &args.action {
        EnvAction::Orphans {
            name,
            all: _,
            old_roots,
            no_file,
            old_root,
            json,
            global,
        } => {
            if *old_roots {
                anyhow::ensure!(
                    args.collection.is_none(),
                    "--old-roots looks across every collection, so it cannot be used with --collection"
                );
                return gone_roots(*json, &*platform_store());
            }
            // The list already covers secrets with no file, so the flag would promise a filter.
            anyhow::ensure!(
                name.is_some() || !*no_file || old_root.is_some(),
                "--all already lists secrets with no file; drop --no-file"
            );
            let envs = Envs::open(args, *global, old_root.as_deref())?;
            let store = platform_store();
            return match name {
                Some(name) => orphans(&Target::new(&envs, name, *no_file), *json, &*store),
                None => orphans_everywhere(&envs, *json, &*store),
            };
        }
        EnvAction::Forget {
            name,
            variables,
            all,
            no_file,
            old_root,
            global,
            yes,
        } => {
            let envs = Envs::open(args, *global, old_root.as_deref())?;
            let store = platform_store();
            let target = Target::new(&envs, name, *no_file);
            return forget(&target, variables, *all, *yes, &*store, &mut trust::confirm);
        }
        _ => {}
    }
    // Every action with the flag must be listed, or the flag is parsed and the keychain opened.
    let no_keychain = matches!(
        args.action,
        EnvAction::Set {
            no_keychain: true,
            ..
        } | EnvAction::Unset {
            no_keychain: true,
            ..
        } | EnvAction::Cp {
            no_keychain: true,
            ..
        } | EnvAction::Mv {
            no_keychain: true,
            ..
        } | EnvAction::Rm {
            no_keychain: true,
            ..
        }
    );
    // The keychain opens on first use, so listing never touches it.
    let store = secret_store(no_keychain);
    go_with(args, &*store, &mut read_secret_value)
}

/// Print what `target` holds, one per line or as JSON.
fn orphans(target: &Target, json: bool, store: &dyn SecretStore) -> anyhow::Result<()> {
    let orphans = target.listed(store)?;
    if json {
        return print_json(&OrphansDoc { orphans });
    }
    for line in orphan_lines(&orphans) {
        outln!("{line}");
    }
    Ok(())
}

/// Print the collection-wide report. An environment that gave no answer fails the command, after
/// everything that could be said has been printed.
fn orphans_everywhere(envs: &Envs, json: bool, store: &dyn SecretStore) -> anyhow::Result<()> {
    let report = envs.everywhere(store).map_err(escaped)?;
    if json {
        print_json(&ReportDoc::from(&report))?;
    } else {
        for line in report_lines(&report) {
            outln!("{line}");
        }
    }
    for env in &report.unreadable {
        eprintln!(
            "error: cannot read environment {}, so nothing is listed for it",
            shown(env)
        );
    }
    anyhow::ensure!(
        report.unreadable.is_empty(),
        "some environments could not be read"
    );
    Ok(())
}

/// `env orphans --old-roots` as JSON.
#[derive(Serialize)]
struct RootsDoc {
    roots: Vec<String>,
}

/// Print every gone root that still holds secrets. A store that cannot list them all fails the
/// command rather than print nothing, which would read as none.
fn gone_roots(json: bool, store: &dyn SecretStore) -> anyhow::Result<()> {
    let roots = MovedRoot::gone_roots(store)
        .map_err(|e| anyhow::anyhow!("cannot list the collections in the secret store: {e}"))?;
    if json {
        let roots = roots.iter().map(|r| r.to_string_lossy().into_owned());
        return print_json(&RootsDoc {
            roots: roots.collect(),
        });
    }
    for line in root_lines(&roots) {
        outln!("{line}");
    }
    Ok(())
}

/// Each root, a tab, and the command that lists its secrets, the path single-quoted for a POSIX
/// shell; a root with control characters shows escaped, so its command names no real path.
/// A root is untrusted text from the store, so both go through [`shown`].
fn root_lines(roots: &[PathBuf]) -> Vec<String> {
    let line = |root: &PathBuf| {
        let text = root.to_string_lossy();
        let quoted = format!("'{}'", text.replace('\'', r"'\''"));
        format!(
            "{}\tapinata env orphans --all --old-root {}",
            shown(&text),
            shown(&quoted)
        )
    };
    roots.iter().map(line).collect()
}

/// The report as printed: tab-separated columns of escaped names, so a stored tab cannot add one.
fn report_lines(report: &OrphanReport) -> Vec<String> {
    let line = |o: &apinata_core::OrphanedSecret| {
        let (env, var) = (shown(&o.environment), shown(&o.variable));
        let mark = if o.has_file { "" } else { "\tno file" };
        format!("{env}\t{var}{mark}")
    };
    report.orphans.iter().map(line).collect()
}

/// `orphans` as printed: a stored name is untrusted text, so each goes through [`shown`].
fn orphan_lines(orphans: &[String]) -> Vec<String> {
    orphans.iter().map(|o| shown(o)).collect()
}

/// What a finished removal prints: a line per name that went, then a line per warning.
fn removal_lines(
    name: &str,
    names: &[String],
    warnings: &[EnvWarning],
) -> (Vec<String>, Vec<String>) {
    let failed = |n: &String| {
        let same =
            |w: &EnvWarning| matches!(w, EnvWarning::Secret { variable, .. } if variable == n);
        warnings.iter().any(same)
    };
    let went = names.iter().filter(|n| !failed(n));
    let went = went.map(|n| format!("removed {} from {}", shown(n), shown(name)));
    let warned = warnings
        .iter()
        .map(|w| format!("warning: {}", shown(&w.to_string())));
    (went.collect(), warned.collect())
}

/// `forget` with its target, store and confirmation given, so tests supply all three.
fn forget(
    target: &Target,
    variables: &[String],
    all: bool,
    yes: bool,
    store: &dyn SecretStore,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> anyhow::Result<()> {
    let warn = &mut |line: &str| eprintln!("{line}");
    forget_with(target, variables, all, yes, store, confirm, warn)
}

/// [`forget`] with the stderr line that comes before a removal also given.
fn forget_with(
    target: &Target,
    variables: &[String],
    all: bool,
    yes: bool,
    store: &dyn SecretStore,
    confirm: &mut dyn FnMut(&str) -> bool,
    warn: &mut dyn FnMut(&str),
) -> anyhow::Result<()> {
    let listed = target.listed(store)?;
    if all && listed.is_empty() {
        outln!("{}", target.nothing_line());
        return Ok(());
    }
    let names = if all { &listed[..] } else { variables };
    // A name not listed gets core's refusal at once: a question or warning could only mislead.
    let known = names.iter().all(|n| listed.contains(n));
    if known && !yes {
        let list = names.iter().map(|n| shown(n)).collect::<Vec<_>>();
        let list = list.join(", ");
        outln!("{}", target.listing_line(&list));
        if !confirm(&target.question(&list)) {
            outln!("nothing removed");
            return Ok(());
        }
    }
    // `--yes` skips the question, so a script still gets what it would have said.
    if known
        && yes
        && let Some(line) = target.out_of_reach()
    {
        warn(&format!("warning: {line}"));
    }
    let warnings = target.remove(names, store)?;
    // What went is said first, so a partial failure still tells a script what is gone.
    let (went, warned) = removal_lines(target.name, names, &warnings);
    for line in went {
        outln!("{line}");
    }
    for line in warned {
        eprintln!("{line}");
    }
    anyhow::ensure!(warnings.is_empty(), "some stored secrets were not removed");
    Ok(())
}

/// A core error whose text can echo a stored name, safe to print.
fn escaped(e: EnvError) -> anyhow::Error {
    anyhow::anyhow!(shown(&e.to_string()))
}

/// As [`escaped`], telling a refused `--no-file` how to proceed.
fn no_file_hint(e: EnvError) -> anyhow::Error {
    match e {
        EnvError::HasFile { .. } => {
            anyhow::anyhow!("{}; drop --no-file", shown(&e.to_string()))
        }
        e => escaped(e),
    }
}

/// The secret value: one line of piped stdin, or a hidden prompt on a terminal. It is never
/// an argument, so it stays out of shell history and the process list.
fn read_secret_value() -> anyhow::Result<String> {
    use std::io::{BufRead, IsTerminal};
    if std::io::stdin().is_terminal() {
        return Ok(rpassword::prompt_password("Secret value: ")?);
    }
    let mut line = String::new();
    let n = std::io::stdin().lock().read_line(&mut line)?;
    anyhow::ensure!(n > 0, "no secret value on stdin");
    Ok(line)
}

/// Run the action with the secret store and the secret value source given, so tests supply both.
fn go_with(
    args: &EnvArgs,
    store: &dyn SecretStore,
    read_value: &mut dyn FnMut() -> anyhow::Result<String>,
) -> anyhow::Result<()> {
    let root: PathBuf = collection_arg::with_hint(
        trust::directory_root(args.collection.as_deref()),
        "pass the collection with --collection <DIR>",
    )?;
    let mut collection = Collection::open(&root)?;
    match &args.action {
        EnvAction::Set {
            name,
            assignment,
            secret,
            ..
        } => {
            if *secret {
                anyhow::ensure!(
                    !assignment.contains('='),
                    "--secret takes only the variable name; the value is read from stdin or a prompt, never the command line"
                );
                let value = read_value()?;
                let value = value.trim_end_matches(['\r', '\n']);
                collection.set_secret_value(name, assignment, value, store)?;
                outln!("set secret {} in {}", shown(assignment), shown(name));
            } else {
                let (key, value) = assignment.split_once('=').ok_or_else(|| {
                    anyhow::anyhow!("expected KEY=VALUE, got `{}`", shown(assignment))
                })?;
                let changed = collection
                    .set_variable_text(name, key, value)
                    .map_err(secret_hint)?;
                let verb = if changed { "set" } else { "unchanged:" };
                outln!("{verb} {} in {}", shown(key), shown(name));
            }
        }
        EnvAction::Unset {
            name, key, secret, ..
        } => {
            let warnings = collection
                .remove_variable(name, key, *secret, store)
                .map_err(secret_hint)?;
            for w in warnings {
                eprintln!("warning: {w}");
            }
            outln!("removed {} from {}", shown(key), shown(name));
        }
        EnvAction::New { .. }
        | EnvAction::Cp { .. }
        | EnvAction::Mv { .. }
        | EnvAction::Rm { .. } => manage(&mut collection, &args.action, store)?,
        _ => show_or_list(&collection, &args.action)?,
    }
    Ok(())
}

/// Create, copy, rename or delete a whole environment through core, which writes every file.
fn manage(
    collection: &mut Collection,
    action: &EnvAction,
    store: &dyn SecretStore,
) -> anyhow::Result<()> {
    // Read before the change: the secrets a `--no-keychain` run leaves alone, named in a note.
    let left = |c: &Collection, name: &str, skipped: bool| {
        if skipped {
            c.saved_secret_names(name)
        } else {
            Vec::new()
        }
    };
    match action {
        EnvAction::New { name, json } => {
            let (made, warnings) = collection.create_environment(name, store)?;
            warn(&warnings);
            print_name(&format!("created {}", shown(&made)), &made, *json)?;
        }
        EnvAction::Cp {
            name,
            json,
            no_keychain,
        } => {
            let secrets = left(collection, name, *no_keychain);
            let (made, warnings) = collection.duplicate_environment(name, store)?;
            warn(&warnings);
            note(
                &secrets,
                &format!("secret values not copied to {}", shown(&made)),
            );
            let text = format!("copied {} to {}", shown(name), shown(&made));
            print_name(&text, &made, *json)?;
        }
        EnvAction::Mv {
            from,
            to,
            no_keychain,
        } => {
            // Core treats this as a no-op; say so, as `set` does.
            if from == to && collection.environment(from).is_some() {
                outln!("unchanged: {}", shown(from));
                return Ok(());
            }
            let secrets = left(collection, from, *no_keychain);
            let warnings = collection.rename_environment(from, to, store)?;
            warn(&warnings);
            note(
                &secrets,
                &format!("secret values left under {}", shown(from)),
            );
            outln!("renamed {} to {}", shown(from), shown(to));
        }
        EnvAction::Rm { name, no_keychain } => {
            if collection.environment(name).is_none() {
                return Err(collection.environment_not_found(name).into());
            }
            let secrets = left(collection, name, *no_keychain);
            // Core refuses a parent others extend; its text echoes their names, so escape it.
            let warnings = collection
                .delete_environment(name, store)
                .map_err(|e| anyhow::anyhow!(shown(&e.to_string())))?;
            warn(&warnings);
            note(
                &secrets,
                &format!("secret values of {} left in the keychain", shown(name)),
            );
            outln!("removed {}", shown(name));
        }
        _ => {}
    }
    Ok(())
}

fn warn(warnings: &[apinata_core::EnvWarning]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

/// The note for secrets a `--no-keychain` run did not touch; nothing when there are none.
fn note(secrets: &[String], what: &str) {
    if !secrets.is_empty() {
        let names: Vec<String> = secrets.iter().map(|s| shown(s)).collect();
        eprintln!("note: --no-keychain: {what}: {}", names.join(", "));
    }
}

fn print_name(text: &str, name: &str, json: bool) -> anyhow::Result<()> {
    if json {
        print_json(&serde_json::json!({ "name": name }))
    } else {
        outln!("{text}");
        Ok(())
    }
}

/// Tell a refused secret edit how to proceed.
fn secret_hint(e: EnvError) -> anyhow::Error {
    match e {
        EnvError::SecretVariable { .. } => anyhow::anyhow!(
            "{e}; use --secret to change or remove it (its value is read from stdin or a prompt)"
        ),
        e => e.into(),
    }
}

fn show_or_list(collection: &Collection, action: &EnvAction) -> anyhow::Result<()> {
    match action {
        EnvAction::Ls { json } => {
            let names: Vec<String> = collection
                .environments()
                .iter()
                .map(|e| e.name().to_owned())
                .collect();
            if *json {
                print_json(&NamesDoc {
                    environments: names,
                })?;
            } else {
                for name in names {
                    outln!("{}", shown(&name));
                }
            }
        }
        // `--env-var` is accepted so a run's flags can be reused, but only declared variables are
        // shown, and a secret never carries a value.
        EnvAction::Show { name, json, .. } => {
            let env = collection
                .environment(name)
                .ok_or_else(|| collection.environment_not_found(name))?;
            let variables: Vec<VarDoc> = env
                .variables()
                .iter()
                .map(|v| {
                    let (value, value_type) = match &v.value {
                        VarValue::Text(t) => (t.clone(), None),
                        VarValue::Typed { kind, data } => (data.clone(), Some(kind.clone())),
                        // A value kind this build does not model is flagged, never shown as empty.
                        _ => ("<unsupported>".to_owned(), None),
                    };
                    VarDoc {
                        name: v.name.clone(),
                        value: (!v.secret).then_some(value),
                        value_type,
                        enabled: v.enabled,
                        secret: v.secret,
                    }
                })
                .collect();
            let doc = EnvDoc {
                name: env.name().to_owned(),
                variables,
            };
            if *json {
                print_json(&doc)?;
            } else {
                for v in &doc.variables {
                    let value = v.value.as_deref().map_or("<secret>".to_owned(), shown);
                    let kind = v
                        .value_type
                        .as_deref()
                        .map_or(String::new(), |k| format!("  ({})", shown(k)));
                    let off = if v.enabled { "" } else { "  (disabled)" };
                    outln!("{} = {value}{kind}{off}", shown(&v.name));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn print_json<T: Serialize>(doc: &T) -> anyhow::Result<()> {
    outln!("{}", serde_json::to_string_pretty(doc)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use apinata_core::{MemoryStore, SecretKey, SecretStore};

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("opencollection.yml"),
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\n",
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("environments")).unwrap();
        std::fs::write(
            dir.path().join("environments/dev.yml"),
            "name: dev\nvariables:\n  - name: host\n    value: a\n",
        )
        .unwrap();
        dir
    }

    fn set_secret(dir: &std::path::Path, key: &str, input: &str, store: &MemoryStore) {
        let action = EnvAction::Set {
            name: "dev".into(),
            assignment: key.into(),
            secret: true,
            no_keychain: false,
        };
        let args = EnvArgs {
            action,
            collection: Some(dir.to_owned()),
        };
        let mut read = || Ok(input.to_owned());
        go_with(&args, store, &mut read).unwrap();
    }

    #[test]
    fn a_secret_value_reaches_the_store_with_its_newline_trimmed_and_not_the_file() {
        let dir = tree();
        let store = MemoryStore::new();
        set_secret(dir.path(), "apikey", "hunter2\r\n", &store);
        let file = std::fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
        assert!(
            file.contains("name: apikey") && !file.contains("hunter2"),
            "{file}"
        );
        let c = Collection::open(dir.path()).unwrap();
        let got = store.get(&c.secret_key("dev", "apikey")).unwrap();
        assert_eq!(got.as_deref(), Some("hunter2"));
        // A second run with the same value leaves the file as it was.
        set_secret(dir.path(), "apikey", "hunter2\n", &store);
        let again = std::fs::read_to_string(dir.path().join("environments/dev.yml")).unwrap();
        assert_eq!(again, file);
    }

    fn unset_secret(dir: &std::path::Path, key: &str, store: &MemoryStore) {
        let action = EnvAction::Unset {
            name: "dev".into(),
            key: key.into(),
            secret: true,
            no_keychain: false,
        };
        let args = EnvArgs {
            action,
            collection: Some(dir.to_owned()),
        };
        let mut read = || Ok(String::new());
        go_with(&args, store, &mut read).unwrap();
    }

    #[test]
    fn secret_set_and_unset_leave_other_hand_ordered_entries_byte_identical() {
        let dir = tree();
        let others = "  - name: token\n    secret: true\n  - secret: true\n    name: key\n  - name: host\n    value: a\n    description: the host\n    extra: 1\n";
        let path = dir.path().join("environments/dev.yml");
        std::fs::write(&path, format!("name: dev\nvariables:\n{others}")).unwrap();
        let store = MemoryStore::new();
        set_secret(dir.path(), "pw", "x", &store);
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains(others), "{file}");
        unset_secret(dir.path(), "pw", &store);
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains(others), "{file}");
    }

    fn act(dir: &std::path::Path, action: EnvAction, store: &MemoryStore) -> anyhow::Result<()> {
        let args = EnvArgs {
            action,
            collection: Some(dir.to_owned()),
        };
        go_with(&args, store, &mut || Ok(String::new()))
    }

    #[test]
    fn mv_moves_the_secret_to_the_new_name_and_repoints_extends() {
        let dir = tree();
        std::fs::write(
            dir.path().join("environments/child.yml"),
            "name: child\nextends: dev\n",
        )
        .unwrap();
        let store = MemoryStore::new();
        set_secret(dir.path(), "apikey", "hunter2", &store);
        let mv = EnvAction::Mv {
            from: "dev".into(),
            to: "staging".into(),
            no_keychain: false,
        };
        act(dir.path(), mv, &store).unwrap();
        assert!(!dir.path().join("environments/dev.yml").exists());
        let child = std::fs::read_to_string(dir.path().join("environments/child.yml")).unwrap();
        assert!(child.contains("extends: staging"), "{child}");
        let c = Collection::open(dir.path()).unwrap();
        let got = store.get(&c.secret_key("staging", "apikey")).unwrap();
        assert_eq!(got.as_deref(), Some("hunter2"));
        assert_eq!(store.get(&c.secret_key("dev", "apikey")).unwrap(), None);
    }

    #[test]
    fn rm_removes_the_stored_secret() {
        let dir = tree();
        let store = MemoryStore::new();
        set_secret(dir.path(), "apikey", "hunter2", &store);
        let rm = EnvAction::Rm {
            name: "dev".into(),
            no_keychain: false,
        };
        act(dir.path(), rm, &store).unwrap();
        assert!(!dir.path().join("environments/dev.yml").exists());
        let c = Collection::open(dir.path()).unwrap();
        assert_eq!(store.get(&c.secret_key("dev", "apikey")).unwrap(), None);
    }

    #[test]
    fn rm_refuses_an_extended_environment_and_keeps_its_secret() {
        let dir = tree();
        std::fs::write(
            dir.path().join("environments/child.yml"),
            "name: child\nextends: dev\n",
        )
        .unwrap();
        let store = MemoryStore::new();
        set_secret(dir.path(), "apikey", "hunter2", &store);
        let rm = EnvAction::Rm {
            name: "dev".into(),
            no_keychain: false,
        };
        let err = act(dir.path(), rm, &store).unwrap_err().to_string();
        assert!(err.contains("`child`"), "{err}");
        assert!(dir.path().join("environments/dev.yml").is_file());
        let c = Collection::open(dir.path()).unwrap();
        let got = store.get(&c.secret_key("dev", "apikey")).unwrap();
        assert_eq!(got.as_deref(), Some("hunter2"));
    }

    #[test]
    fn cp_copies_the_secret_to_the_copy() {
        let dir = tree();
        let store = MemoryStore::new();
        set_secret(dir.path(), "apikey", "hunter2", &store);
        let cp = EnvAction::Cp {
            name: "dev".into(),
            json: false,
            no_keychain: false,
        };
        act(dir.path(), cp, &store).unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let got = store.get(&c.secret_key("dev copy", "apikey")).unwrap();
        assert_eq!(got.as_deref(), Some("hunter2"));
    }

    fn orphan_setup() -> (tempfile::TempDir, Collection, MemoryStore) {
        let dir = tree();
        let c = Collection::open(dir.path()).unwrap();
        let store = MemoryStore::new();
        store.set(&c.secret_key("dev", "old"), "v").unwrap();
        store.set(&c.secret_key("dev", "older"), "v").unwrap();
        (dir, c, store)
    }

    fn held(c: &Collection, store: &MemoryStore, name: &str) -> bool {
        store.get(&c.secret_key("dev", name)).unwrap().is_some()
    }

    #[test]
    fn declining_keeps_the_value_and_the_question_warns() {
        let (dir, c, store) = orphan_setup();
        let mut asked = String::new();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let names = ["old".to_owned()];
        forget(
            &Target::new(&envs, "dev", false),
            &names,
            false,
            false,
            &store,
            &mut |q| {
                asked = q.to_owned();
                false
            },
        )
        .unwrap();
        assert!(held(&c, &store, "old"));
        assert!(
            asked.contains("old") && asked.contains("cannot be undone"),
            "{asked}"
        );
        assert!(asked.contains("branch"), "{asked}");
    }

    #[test]
    fn agreeing_removes_only_what_was_named() {
        let (dir, c, store) = orphan_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let names = ["old".to_owned()];
        forget(
            &Target::new(&envs, "dev", false),
            &names,
            false,
            false,
            &store,
            &mut |_| true,
        )
        .unwrap();
        assert!(!held(&c, &store, "old") && held(&c, &store, "older"));
    }

    #[test]
    fn yes_skips_the_question_and_all_takes_every_orphan() {
        let (dir, c, store) = orphan_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        forget(
            &Target::new(&envs, "dev", false),
            &[],
            true,
            true,
            &store,
            &mut |_| panic!("asked"),
        )
        .unwrap();
        assert!(!held(&c, &store, "old") && !held(&c, &store, "older"));
    }

    #[test]
    fn a_mistyped_name_is_refused_before_any_question() {
        let (dir, c, store) = orphan_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let names = ["olld".to_owned()];
        let got = forget(
            &Target::new(&envs, "dev", false),
            &names,
            false,
            false,
            &store,
            &mut |_| panic!("asked"),
        );
        assert!(got.is_err());
        assert!(held(&c, &store, "old"));
    }

    #[test]
    fn a_global_environment_is_forgotten_the_same_way() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("g.yml"), "name: g\n").unwrap();
        let globals = apinata_core::GlobalEnvironments::open(dir.path()).unwrap();
        let store = MemoryStore::new();
        let key = globals.secret_key("g", "old");
        store.set(&key, "v").unwrap();
        let envs = Envs::Global(globals);
        forget(
            &Target::new(&envs, "g", false),
            &[],
            true,
            true,
            &store,
            &mut |_| panic!("asked"),
        )
        .unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn gone_roots_print_with_the_command_that_cleans_each_up() {
        let here = tempfile::tempdir().unwrap();
        let store = MemoryStore::new();
        let live = here.path().join("live");
        std::fs::create_dir(&live).unwrap();
        let gone = here.path().join("old one");
        for root in [&live, &gone] {
            store
                .set(&apinata_core::SecretKey::new(root, "dev", "t"), "v")
                .unwrap();
        }
        let roots = MovedRoot::gone_roots(&store).unwrap();
        let shown_root = gone.display().to_string();
        assert_eq!(
            root_lines(&roots),
            [format!(
                "{shown_root}\tapinata env orphans --all --old-root '{shown_root}'"
            )]
        );
    }

    #[test]
    fn a_root_with_a_quote_or_a_control_character_stays_one_safe_line() {
        let roots = [PathBuf::from("/old/it's\u{1b}[2J")];
        let line = &root_lines(&roots)[0];
        assert!(!line.contains('\u{1b}'), "{line}");
        assert!(
            line.ends_with(r"--old-root '/old/it'\''s\u{1b}[2J'"),
            "{line}"
        );
    }

    #[test]
    fn a_failed_delete_fails_the_command() {
        struct NoDelete(MemoryStore);
        impl SecretStore for NoDelete {
            fn get(
                &self,
                k: &apinata_core::SecretKey,
            ) -> Result<Option<String>, apinata_core::SecretError> {
                self.0.get(k)
            }
            fn set(
                &self,
                k: &apinata_core::SecretKey,
                v: &str,
            ) -> Result<(), apinata_core::SecretError> {
                self.0.set(k, v)
            }
            fn delete(&self, _: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
                Err(apinata_core::SecretError::Unavailable {
                    message: "locked".into(),
                })
            }
            fn list(
                &self,
                p: &std::path::Path,
            ) -> Result<Vec<apinata_core::SecretKey>, apinata_core::SecretError> {
                self.0.list(p)
            }
            fn list_roots(&self) -> Result<Vec<std::path::PathBuf>, apinata_core::SecretError> {
                self.0.list_roots()
            }
        }
        let (dir, _c, store) = orphan_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let got = forget(
            &Target::new(&envs, "dev", false),
            &[],
            true,
            true,
            &NoDelete(store),
            &mut |_| true,
        );
        assert!(got.is_err());
    }

    /// A store that refuses to delete `locked`.
    struct DeleteRefused {
        inner: MemoryStore,
        locked: &'static str,
    }

    impl SecretStore for DeleteRefused {
        fn get(
            &self,
            k: &apinata_core::SecretKey,
        ) -> Result<Option<String>, apinata_core::SecretError> {
            self.inner.get(k)
        }
        fn set(
            &self,
            k: &apinata_core::SecretKey,
            v: &str,
        ) -> Result<(), apinata_core::SecretError> {
            self.inner.set(k, v)
        }
        fn delete(&self, k: &apinata_core::SecretKey) -> Result<(), apinata_core::SecretError> {
            if k.variable == self.locked {
                return Err(apinata_core::SecretError::Unavailable {
                    message: "locked".into(),
                });
            }
            self.inner.delete(k)
        }
        fn list(
            &self,
            p: &std::path::Path,
        ) -> Result<Vec<apinata_core::SecretKey>, apinata_core::SecretError> {
            self.inner.list(p)
        }
        fn list_roots(&self) -> Result<Vec<std::path::PathBuf>, apinata_core::SecretError> {
            self.inner.list_roots()
        }
    }

    #[test]
    fn a_partial_removal_reports_what_went_and_what_did_not() {
        let (dir, _c, inner) = orphan_setup();
        let store = DeleteRefused {
            inner,
            locked: "older",
        };
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let names = ["old".to_owned(), "older".to_owned()];
        let warnings = envs.remove("dev", &names, &store).unwrap();
        let (went, warned) = removal_lines("dev", &names, &warnings);
        assert_eq!(went, ["removed old from dev"]);
        assert_eq!(warned.len(), 1);
        assert!(warned[0].contains("older"), "{warned:?}");
    }

    #[test]
    fn a_control_character_in_a_stored_name_never_reaches_the_terminal() {
        let name = "a\u{1b}[2Jb";
        let (dir, c, store) = orphan_setup();
        store.set(&c.secret_key("dev", name), "v").unwrap();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let names = [name.to_owned()];
        let mut asked = String::new();
        forget(
            &Target::new(&envs, "dev", false),
            &names,
            false,
            false,
            &store,
            &mut |q| {
                asked = q.to_owned();
                false
            },
        )
        .unwrap();
        assert!(
            !asked.contains('\u{1b}') && asked.contains(r"\u{1b}"),
            "{asked}"
        );
        for line in orphan_lines(&names) {
            assert!(!line.contains('\u{1b}'), "{line}");
        }
        let locked = DeleteRefused {
            inner: store,
            locked: name,
        };
        let warnings = envs.remove("dev", &names, &locked).unwrap();
        let (_, warned) = removal_lines("dev", &names, &warnings);
        assert!(!warned[0].contains('\u{1b}'), "{warned:?}");
        let (went, _) = removal_lines("dev", &names, &[]);
        assert!(!went[0].contains('\u{1b}'), "{went:?}");
    }

    const OUT_OF_REACH_COLLECTION: &str =
        "The collection may only be elsewhere, for example on a drive that is not mounted.";
    const OUT_OF_REACH_DIRECTORY: &str = "The environment's directory may only be elsewhere, for example on a drive that is not mounted.";

    /// A collection with `dev`, and a store holding `gone`'s `a` and `b`, which has no file.
    fn fileless_setup() -> (tempfile::TempDir, Collection, MemoryStore) {
        let dir = tree();
        let c = Collection::open(dir.path()).unwrap();
        let store = MemoryStore::new();
        store.set(&c.secret_key("gone", "a"), "v").unwrap();
        store.set(&c.secret_key("gone", "b"), "v").unwrap();
        (dir, c, store)
    }

    fn holds(c: &Collection, store: &MemoryStore, env: &str, name: &str) -> bool {
        store.get(&c.secret_key(env, name)).unwrap().is_some()
    }

    #[test]
    fn a_no_file_name_stays_not_found_without_the_flag() {
        let (dir, c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let err = forget(
            &Target::new(&envs, "gone", false),
            &[],
            true,
            true,
            &store,
            &mut |_| panic!("asked"),
        )
        .unwrap_err()
        .to_string();
        assert_eq!(err, c.environment_not_found("gone").to_string());
        assert!(holds(&c, &store, "gone", "a"));
    }

    #[test]
    fn declining_a_no_file_removal_keeps_the_values_and_the_question_warns() {
        let (dir, c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, "gone", true);
        let mut asked = String::new();
        forget(&target, &[], true, false, &store, &mut |q| {
            asked = q.to_owned();
            false
        })
        .unwrap();
        assert!(holds(&c, &store, "gone", "a") && holds(&c, &store, "gone", "b"));
        assert!(asked.contains("a, b"), "{asked}");
        assert!(asked.contains("cannot be undone"), "{asked}");
        assert!(asked.contains("branch"), "{asked}");
        assert!(asked.contains(OUT_OF_REACH_DIRECTORY), "{asked}");
    }

    #[test]
    fn yes_removes_a_no_file_secret_by_name() {
        let (dir, c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, "gone", true);
        let names = ["a".to_owned()];
        forget(&target, &names, false, true, &store, &mut |_| {
            panic!("asked")
        })
        .unwrap();
        assert!(!holds(&c, &store, "gone", "a") && holds(&c, &store, "gone", "b"));
    }

    #[test]
    fn no_file_on_an_environment_with_a_file_is_refused_with_the_way_out() {
        let (dir, c, store) = orphan_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, "dev", true);
        let err = forget(&target, &[], true, true, &store, &mut |_| panic!("asked"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("has a file") && err.contains("--no-file"),
            "{err}"
        );
        assert!(held(&c, &store, "old"));
    }

    #[test]
    fn a_mistyped_no_file_name_is_refused_before_any_question() {
        let (dir, c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, "gone", true);
        let names = ["typo".to_owned()];
        let got = forget(&target, &names, false, false, &store, &mut |_| {
            panic!("asked")
        });
        assert!(got.is_err());
        assert!(holds(&c, &store, "gone", "a"));
    }

    #[test]
    fn all_with_nothing_stored_under_a_no_file_name_succeeds_without_asking() {
        let (dir, _c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, "never", true);
        forget(&target, &[], true, false, &store, &mut |_| panic!("asked")).unwrap();
    }

    #[test]
    fn an_old_root_question_says_the_collection_may_only_be_elsewhere() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("gone");
        let key = SecretKey::new(&root, "dev", "a");
        let store = MemoryStore::new();
        store.set(&key, "v").unwrap();
        let envs = Envs::MovedRoot(MovedRoot::new(&root).unwrap());
        // The flag is implied, so no `--no-file` is needed.
        let target = Target::new(&envs, "dev", false);
        let mut asked = String::new();
        forget(&target, &[], true, false, &store, &mut |q| {
            asked = q.to_owned();
            false
        })
        .unwrap();
        assert!(store.get(&key).unwrap().is_some());
        for word in ["cannot be undone", "branch", "elsewhere", "not mounted"] {
            assert!(asked.contains(word), "{word}: {asked}");
        }
        forget(&target, &[], true, true, &store, &mut |_| panic!("asked")).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn only_an_old_root_uses_the_wording_for_a_collection_that_may_be_elsewhere() {
        let dir = tempfile::TempDir::new().unwrap();
        let moved = Envs::MovedRoot(MovedRoot::new(dir.path().join("gone")).unwrap());
        let target = Target::new(&moved, "dev", false);
        assert!(target.question("a").contains(OUT_OF_REACH_COLLECTION));
        assert!(!target.question("a").contains(OUT_OF_REACH_DIRECTORY));
        assert_eq!(
            target.listing_line("a"),
            "stored secrets of dev, which has no file: a"
        );
        assert_eq!(target.nothing_line(), "no stored secrets under dev");

        let (dir, _c, _store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, "gone", true);
        assert!(!target.question("a").contains(OUT_OF_REACH_COLLECTION));
        assert!(target.question("a").contains(OUT_OF_REACH_DIRECTORY));
    }

    /// Run `forget --all --yes`, giving each stderr line and whether `key` was held at that moment.
    fn yes_warnings(target: &Target, store: &MemoryStore, key: &SecretKey) -> Vec<(String, bool)> {
        let mut lines = Vec::new();
        let asked = &mut |_: &str| panic!("asked");
        let warn = &mut |line: &str| {
            lines.push((line.to_owned(), store.get(key).unwrap().is_some()));
        };
        forget_with(target, &[], true, true, store, asked, warn).unwrap();
        lines
    }

    #[test]
    fn no_file_with_yes_warns_on_stderr_before_removing() {
        let (dir, c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let key = c.secret_key("gone", "a");
        let lines = yes_warnings(&Target::new(&envs, "gone", true), &store, &key);
        assert_eq!(
            lines,
            [(format!("warning: {OUT_OF_REACH_DIRECTORY}"), true)]
        );
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn a_confirmed_question_is_not_followed_by_the_warning_again() {
        let (dir, c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, "gone", true);
        let warn = &mut |line: &str| panic!("warned: {line}");
        forget_with(&target, &[], true, false, &store, &mut |_| true, warn).unwrap();
        assert!(!holds(&c, &store, "gone", "a"));
    }

    #[test]
    fn no_file_under_global_warns_when_asking_and_with_yes() {
        let dir = tempfile::TempDir::new().unwrap();
        let globals = apinata_core::GlobalEnvironments::open(dir.path()).unwrap();
        let store = MemoryStore::new();
        let key = globals.secret_key("gone", "a");
        store.set(&key, "v").unwrap();
        let envs = Envs::Global(globals);
        let target = Target::new(&envs, "gone", true);
        assert!(target.question("a").contains(OUT_OF_REACH_DIRECTORY));
        let lines = yes_warnings(&target, &store, &key);
        assert_eq!(
            lines,
            [(format!("warning: {OUT_OF_REACH_DIRECTORY}"), true)]
        );
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn an_old_root_with_yes_warns_on_stderr_before_removing() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("gone");
        let key = SecretKey::new(&root, "dev", "a");
        let store = MemoryStore::new();
        store.set(&key, "v").unwrap();
        let envs = Envs::MovedRoot(MovedRoot::new(&root).unwrap());
        let lines = yes_warnings(&Target::new(&envs, "dev", false), &store, &key);
        assert_eq!(
            lines,
            [(format!("warning: {OUT_OF_REACH_COLLECTION}"), true)]
        );
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn a_plain_forget_says_nothing_of_a_directory_out_of_reach() {
        let (dir, c, store) = orphan_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, "dev", false);
        assert_eq!(
            target.question("a"),
            "Remove the stored values of a? This cannot be undone, and another git branch may still use them. [y/N] "
        );
        let key = c.secret_key("dev", "old");
        assert_eq!(yes_warnings(&target, &store, &key), []);
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn nothing_to_remove_with_yes_gives_no_warning() {
        let (dir, c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let key = c.secret_key("gone", "a");
        assert_eq!(
            yes_warnings(&Target::new(&envs, "never", true), &store, &key),
            []
        );
        let target = Target::new(&envs, "gone", true);
        let names = ["typo".to_owned()];
        let asked = &mut |_: &str| panic!("asked");
        let warn = &mut |line: &str| panic!("warned: {line}");
        let got = forget_with(&target, &names, false, true, &store, asked, warn);
        assert!(got.is_err());
        assert!(holds(&c, &store, "gone", "a"));
    }

    #[test]
    fn the_collection_wide_report_as_json_and_lines() {
        let (dir, c, store) = fileless_setup();
        store.set(&c.secret_key("dev", "stray"), "v").unwrap();
        std::fs::write(dir.path().join("environments/broken.yml"), "a: [\n").unwrap();
        let c = Collection::open(dir.path()).unwrap();
        store.set(&c.secret_key("broken", "x"), "v").unwrap();
        let report = c.all_orphaned_secrets(&store).unwrap();
        let doc = serde_json::to_value(ReportDoc::from(&report)).unwrap();
        let row = |e: &str, v: &str, f: bool| serde_json::json!({"environment": e, "variable": v, "has_file": f});
        assert_eq!(
            doc,
            serde_json::json!({
                "orphans": [row("dev", "stray", true), row("gone", "a", false), row("gone", "b", false)],
                "unreadable": ["broken"],
            })
        );
        assert_eq!(
            report_lines(&report),
            ["dev\tstray", "gone\ta\tno file", "gone\tb\tno file"]
        );
    }

    #[test]
    fn an_unreadable_environment_fails_the_command_and_nothing_is_lost() {
        let (dir, _c, store) = fileless_setup();
        std::fs::write(dir.path().join("environments/broken.yml"), "a: [\n").unwrap();
        let c = Collection::open(dir.path()).unwrap();
        store.set(&c.secret_key("broken", "x"), "v").unwrap();
        let envs = Envs::Collection(c);
        assert!(orphans_everywhere(&envs, false, &store).is_err());
        assert!(orphans_everywhere(&envs, true, &store).is_err());
        let Envs::Collection(c) = &envs else {
            unreachable!()
        };
        assert!(holds(c, &store, "broken", "x"));
    }

    #[test]
    fn a_clean_collection_wide_report_succeeds() {
        let (dir, _c, store) = fileless_setup();
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        orphans_everywhere(&envs, true, &store).unwrap();
    }

    #[test]
    fn a_hostile_stored_name_never_reaches_the_terminal() {
        let bad = "a\u{1b}[2J\tb";
        let (dir, c, store) = fileless_setup();
        store.set(&c.secret_key(bad, bad), "v").unwrap();
        let report = c.all_orphaned_secrets(&store).unwrap();
        let lines = report_lines(&report);
        let line = lines.iter().find(|l| l.contains("[2J")).unwrap();
        assert!(!line.contains('\u{1b}'), "{line}");
        // Only the separators are tabs, so a stored tab cannot fake a column.
        assert_eq!(line.matches('\t').count(), 2, "{line}");
        let envs = Envs::Collection(Collection::open(dir.path()).unwrap());
        let target = Target::new(&envs, bad, true);
        let mut asked = String::new();
        forget(&target, &[], true, false, &store, &mut |q| {
            asked = q.to_owned();
            false
        })
        .unwrap();
        assert!(!asked.contains('\u{1b}'), "{asked}");
    }
}
