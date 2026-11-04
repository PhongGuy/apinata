//! `apinata import`: bring requests from other tools' formats into a collection through core's
//! format dispatch, printing what was written and a note for everything not carried over as is.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use apinata_core::import::{
    CollectionFormat, CollectionOptions, EnvironmentFormat, FetchError, GroupBy, ImportBundle,
    ImportError, ImportNote, ImportReport, ImportSource, RequestFormat, SourceError, SourceFetch,
    UpdateOptions, UpdateSummary, add_environment, add_request, apply_update, classify_source,
    default_import_name, ensure_folder, fetch_source, parse_collections, parse_environments,
    parse_request, place_items, plan_update, store_request_secrets, write_collection,
};
use apinata_core::{
    Collection, Folder, ItemPath, PacSource, SecretError, SecretKey, SecretStore, SystemTrash,
    TrashLog,
};
use clap::Subcommand;
use serde_json::json;

use apinata_core::{KEYCHAIN_TIMEOUT, LazyKeychain, TimeoutStore};

use crate::network;
use crate::run::find_root;
use crate::{EXIT_USAGE, Format};

#[derive(clap::Args)]
pub struct ImportArgs {
    #[command(subcommand)]
    pub kind: ImportKind,
}

#[derive(Subcommand)]
pub enum ImportKind {
    /// Import one request, such as a curl command, from a file or stdin.
    Request(RequestArgs),
    /// Import a collection file or link, with its environments, as a new collection directory,
    /// or with `--folder` into a folder of an existing collection.
    Collection(CollectionArgs),
    /// Import environment files into an existing collection.
    Environment(EnvironmentArgs),
}

#[derive(clap::Args)]
pub struct CollectionArgs {
    /// The collection file, or an http(s) link to one. A link is fetched without approval,
    /// through the system proxy unless a flag below says otherwise.
    #[arg(value_name = "FILE_OR_URL")]
    pub file: PathBuf,
    #[command(flatten)]
    pub fetch: FetchNetworkArgs,
    /// The input format, or `auto` to detect it.
    #[arg(long, value_name = "FORMAT", default_value = "auto", value_parser = collection_format)]
    pub from: Choice<CollectionFormat>,
    /// How OpenAPI operations become folders: by first tag (default) or by path segment.
    #[arg(long, value_enum, value_name = "GROUP")]
    pub group_by: Option<GroupByArg>,
    /// The collection's name for a HAR capture (default: the file name, or the link's last path
    /// segment, without its extension).
    #[arg(long)]
    pub name: Option<String>,
    /// An environment file to import with the collection; may be repeated.
    #[arg(long = "env", value_name = "FILE")]
    pub envs: Vec<PathBuf>,
    /// The directory to create the collection in; defaults to the working directory. The
    /// collection gets a new folder named after it inside DIR (a number is added if the name is
    /// taken), so DIR may already hold other files, but import never fills an existing folder. With
    /// `--update` or `--folder` it is the existing collection's root.
    #[arg(long, value_name = "DIR")]
    pub into: Option<PathBuf>,
    /// Refresh an existing collection from an OpenAPI description: update the fields you have
    /// not edited and add new operations.
    #[arg(long, conflicts_with_all = ["envs", "name"])]
    pub update: bool,
    /// With `--update`, also move to the system trash requests whose operation left the
    /// description, when they are still as generated. Refused when no trash is available.
    /// `apinata item restore --pruned` puts them back.
    #[arg(long, requires = "update")]
    pub prune: bool,
    /// A folder of the existing collection `--into` names, as a path below its root. On its
    /// own, import an OpenAPI description straight into that folder (created when missing,
    /// refused when it already holds requests) and remember it for a later `--update`. With
    /// `--update`, refresh the spec imported into that folder instead of the root's.
    #[arg(long, value_name = "PATH")]
    pub folder: Option<String>,
    /// With `--update`, report what would change and write nothing.
    #[arg(long, requires = "update")]
    pub dry_run: bool,
    /// The report format written to stdout.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
    // Test seam: keep no secret values, so each one becomes a needs-input note.
    #[arg(long, hide = true)]
    pub no_keychain: bool,
}

/// The flags that shape fetching a link, spelled like `run`'s; they do nothing for a file.
#[derive(clap::Args)]
pub struct FetchNetworkArgs {
    /// Accept invalid TLS certificates when fetching a link.
    #[arg(long)]
    pub insecure: bool,
    /// Trust the CA certificates in this PEM file, in addition to the system's.
    #[arg(long, value_name = "FILE")]
    pub cacert: Option<PathBuf>,
    /// With --cacert, trust only its certificates.
    #[arg(long, requires = "cacert")]
    pub ignore_truststore: bool,
    /// Ignore the system proxy.
    #[arg(long)]
    pub noproxy: bool,
    /// Route the fetch through this proxy auto-config file (URL or path).
    #[arg(long, value_name = "URL|PATH", value_parser = network::parse_pac_source, conflicts_with = "noproxy")]
    pub proxy_pac: Option<PacSource>,
}

/// `--group-by`'s values, mapped onto core's `GroupBy`.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum GroupByArg {
    Tags,
    Path,
}

#[derive(clap::Args)]
pub struct EnvironmentArgs {
    /// The environment files.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// The input format, or `auto` to detect it.
    #[arg(long, value_name = "FORMAT", default_value = "auto", value_parser = environment_format)]
    pub from: Choice<EnvironmentFormat>,
    /// The collection directory or a directory inside it; defaults to the working directory.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// The report format written to stdout.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
    // Test seam: keep no secret values, so each one becomes a needs-input note.
    #[arg(long, hide = true)]
    pub no_keychain: bool,
}

#[derive(clap::Args)]
pub struct RequestArgs {
    /// The file holding the command; absent or `-` reads stdin.
    pub file: Option<PathBuf>,
    /// The input format, or `auto` to detect it.
    #[arg(long, value_name = "FORMAT", default_value = "auto", value_parser = request_format)]
    pub from: Choice<RequestFormat>,
    /// The collection directory or a directory inside it; defaults to the working directory.
    #[arg(long)]
    pub collection: Option<PathBuf>,
    /// The folder to create the request in, as a path relative to the collection root.
    #[arg(long, value_name = "PATH")]
    pub folder: Option<String>,
    /// The request's name; defaults to the URL's last path segment, else its host.
    #[arg(long)]
    pub name: Option<String>,
    /// The report format written to stdout.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
    /// The environment that keeps a secret the command carries; defaults to the only one.
    #[arg(long, value_name = "NAME")]
    pub env: Option<String>,
    // Test seam: keep no secret values, so each one becomes a needs-input note.
    #[arg(long, hide = true)]
    pub no_keychain: bool,
}

/// A `--from` value: a named format, or `None` for `auto`. A newtype, because clap reads a bare
/// `Option` as "the argument may be absent".
#[derive(Clone, Copy)]
pub struct Choice<F>(pub Option<F>);

fn choice<F: Copy>(s: &str, all: &[F], id: fn(F) -> &'static str) -> Result<Choice<F>, String> {
    if s == "auto" {
        return Ok(Choice(None));
    }
    all.iter()
        .copied()
        .find(|&f| id(f) == s)
        .map(|f| Choice(Some(f)))
        .ok_or_else(|| {
            let ids: Vec<_> = all.iter().map(|&f| id(f)).collect();
            format!("expected auto, {}", ids.join(", "))
        })
}

fn request_format(s: &str) -> Result<Choice<RequestFormat>, String> {
    choice(s, RequestFormat::ALL, RequestFormat::id)
}

fn collection_format(s: &str) -> Result<Choice<CollectionFormat>, String> {
    choice(s, CollectionFormat::ALL, CollectionFormat::id)
}

fn environment_format(s: &str) -> Result<Choice<EnvironmentFormat>, String> {
    choice(s, EnvironmentFormat::ALL, EnvironmentFormat::id)
}

/// What an import wrote: the collection root for the report, the lines text output lists on
/// stdout, and core's report.
struct Imported {
    root: PathBuf,
    listed: Vec<String>,
    report: ImportReport,
}

/// An import that stopped after writing some of its collections or environments; it displays as
/// the error that stopped it.
#[derive(Debug)]
struct Partial {
    error: ImportError,
    written: Vec<String>,
    not_written: Vec<String>,
}

impl fmt::Display for Partial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for Partial {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.error)
    }
}

/// Run `args` and print its outcome: exit 0 when the import succeeded, 3 with `error: ...` on
/// any failure.
pub(crate) fn run(args: &ImportArgs) -> ExitCode {
    let (format, outcome) = match &args.kind {
        ImportKind::Request(args) => (args.format, import_request(args).map(|one| vec![one])),
        ImportKind::Collection(args) if args.update => {
            let printed = import_update(args).and_then(|u| print_update(args.format, &u));
            return finish(printed);
        }
        ImportKind::Collection(args) if args.folder.is_some() => {
            (args.format, import_into_folder(args).map(|one| vec![one]))
        }
        ImportKind::Collection(args) => (args.format, import_collection(args)),
        ImportKind::Environment(args) => {
            (args.format, import_environments(args).map(|one| vec![one]))
        }
    };
    let printed = outcome.and_then(|all| all.iter().try_for_each(|one| print(format, one)));
    finish(printed)
}

/// Exit 0 for a finished import; otherwise print the error (and what a partial import wrote)
/// and exit with its code.
fn finish(printed: anyhow::Result<()>) -> ExitCode {
    match printed {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            let partial = e.downcast_ref::<Partial>();
            if let Some(p) = partial {
                eprintln!("written: {}", p.written.join(", "));
                eprintln!("not written: {}", p.not_written.join(", "));
            }
            let error = partial
                .map(|p| &p.error)
                .or_else(|| e.downcast_ref::<ImportError>());
            ExitCode::from(error.map_or(EXIT_USAGE, exit_code))
        }
    }
}

/// The exit code for a failed write: every import failure exits 3.
pub(crate) fn exit_code(_: &ImportError) -> u8 {
    EXIT_USAGE
}

// Parse before touching the collection, so a bad command writes nothing; an unknown folder is
// refused by `add_request` before it creates a file.
fn import_request(args: &RequestArgs) -> anyhow::Result<Imported> {
    let input = read_input(args.file.as_deref())?;
    let mut parsed = parse_request(args.from.0, &input)?;
    let root = collection_root(args.collection.as_deref())?;
    let parent = match &args.folder {
        Some(raw) => ItemPath::new(raw).ok_or_else(|| anyhow!("invalid item path: {raw}"))?,
        None => ItemPath::root(),
    };
    let name = match &args.name {
        Some(name) => name.clone(),
        None => default_name(&parsed.request.url),
    };
    let mut collection = Collection::open(&root)?;
    let item = add_request(&mut collection, &parent, &name, &parsed.request)?;
    let secrets = secret_store(args.no_keychain);
    store_request_secrets(
        &mut collection,
        &parsed.request,
        args.env.as_deref(),
        secrets.as_ref(),
        &mut parsed.notes,
    );
    let mut report = ImportReport::default();
    report.notes = parsed.notes;
    for note in &mut report.notes {
        note.file = Some(item.as_str().to_owned());
    }
    report.items.push(item.clone());
    Ok(Imported {
        root,
        listed: vec![item.as_str().to_owned()],
        report,
    })
}

// Every input is parsed before any collection directory is created, so one bad file writes
// nothing. Each bundle then gets its own write; a failure keeps the collections already written.
fn import_collection(args: &CollectionArgs) -> anyhow::Result<Vec<Imported>> {
    let (input, default_name) = match link(&args.file)? {
        Some(url) => (fetch_link(url, &args.fetch)?, default_import_name(url)),
        None => (read_file(&args.file)?, file_stem(&args.file)),
    };
    let options = collection_options(args, &input, default_name)?;
    let bundles = parse_collections(args.from.0, &input, &options)?;
    let mut extra = Vec::new();
    for path in &args.envs {
        let envs = parse_environments(None, &read_file(path)?)
            .with_context(|| format!("cannot import {}", path.display()))?;
        extra.extend(envs);
    }
    // `--env` files have no collection of their own to join when the input holds several.
    if !extra.is_empty() && bundles.len() != 1 {
        return Err(anyhow!(
            "--env needs an input with one collection; this file holds {}",
            bundles.len()
        ));
    }
    let dir = args.into.as_deref().unwrap_or(Path::new("."));
    // Everything is parsed by now, so creating the target cannot leave debris after a bad input.
    let created = create_missing_dirs(dir)?;
    let into = match std::fs::canonicalize(dir) {
        Ok(into) if into.is_dir() => into,
        Ok(into) => return Err(anyhow!("not a directory: {}", into.display())),
        Err(e) => {
            remove_created(&created);
            return Err(anyhow!("cannot open {}: {e}", dir.display()));
        }
    };
    let secrets = secret_store(args.no_keychain);
    let mut done: Vec<Imported> = Vec::new();
    for (i, bundle) in bundles.iter().enumerate() {
        let envs: Vec<_> = bundle.environments.iter().chain(&extra).cloned().collect();
        match write_collection(&into, &bundle.collection, &envs, secrets.as_ref()) {
            Ok((collection, report)) => {
                let root = collection.root().to_path_buf();
                done.push(Imported {
                    listed: vec![root.display().to_string()],
                    root,
                    report,
                });
            }
            Err(error) if done.is_empty() => {
                remove_created(&created);
                return Err(error.into());
            }
            Err(error) => {
                return Err(Partial {
                    error,
                    written: done.iter().map(|d| d.root.display().to_string()).collect(),
                    not_written: bundles[i..]
                        .iter()
                        .map(|b| b.collection.name.clone())
                        .collect(),
                }
                .into());
            }
        }
    }
    Ok(done)
}

// Place the one collection of the input directly in `--folder` of the existing collection at
// `--into`. Everything is checked before the folder is created, so a refusal writes nothing.
fn import_into_folder(args: &CollectionArgs) -> anyhow::Result<Imported> {
    if !args.envs.is_empty() {
        return Err(anyhow!(
            "--env cannot be combined with --folder: environments are not written into a folder"
        ));
    }
    let raw = args.folder.as_deref().unwrap_or_default();
    let target = ItemPath::new(raw)
        .filter(|p| !p.is_root())
        .ok_or_else(|| anyhow!("invalid folder path: {raw:?}"))?;
    let (input, default_name) = match link(&args.file)? {
        Some(url) => (fetch_link(url, &args.fetch)?, default_import_name(url)),
        None => (read_file(&args.file)?, file_stem(&args.file)),
    };
    let options = collection_options(args, &input, default_name)?;
    let mut bundles = parse_collections(args.from.0, &input, &options)?;
    if bundles.len() != 1 {
        return Err(anyhow!(
            "--folder needs an input with one collection; this file holds {}",
            bundles.len()
        ));
    }
    let bundle = bundles.remove(0);
    let root = collection_root(args.into.as_deref())?;
    let mut collection = Collection::open(&root)?;
    if collection.folder(&target).is_some_and(holds_requests) {
        return Err(anyhow!(
            "folder {} already holds requests; use --update to refresh it",
            target.as_str()
        ));
    }
    let segments: Vec<String> = target.as_str().split('/').map(str::to_owned).collect();
    // The topmost folder this run creates, so a failed placement can take back exactly that.
    let created = (1..=segments.len())
        .map(|n| root.join(segments[..n].join("/")))
        .find(|dir| std::fs::symlink_metadata(dir).is_err());
    let take_back = || {
        if let Some(dir) = &created {
            let _ = std::fs::remove_dir_all(dir);
        }
    };
    let folder = ensure_folder(&mut collection, &segments).inspect_err(|_| take_back())?;
    let secrets = secret_store(args.no_keychain);
    let placed = match place_items(&mut collection, &folder, &bundle, secrets.as_ref()) {
        Ok(placed) => placed,
        Err(e) => {
            take_back();
            return Err(e.into());
        }
    };
    // Core reports items relative to the folder; the user sees them from the collection root.
    let mut report = placed.report;
    report.items = report
        .items
        .iter()
        .filter_map(|item| {
            item.as_str()
                .split('/')
                .try_fold(folder.clone(), |p, segment| p.join(segment))
        })
        .collect();
    Ok(Imported {
        listed: report.items.iter().map(|i| i.as_str().to_owned()).collect(),
        root,
        report,
    })
}

/// Whether `folder` or any folder below it holds a request.
fn holds_requests(folder: &Folder) -> bool {
    !folder.requests().is_empty() || folder.folders().iter().any(holds_requests)
}

/// Create `dir` and any missing parents, returning what this call made, deepest first.
fn create_missing_dirs(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let missing: Vec<PathBuf> = dir
        .ancestors()
        .take_while(|a| !a.as_os_str().is_empty() && !a.exists())
        .map(Path::to_path_buf)
        .collect();
    // An existing path, even a file, is left for the caller's directory check to judge.
    if missing.is_empty() {
        return Ok(missing);
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        remove_created(&missing);
        return Err(anyhow!("cannot create {}: {e}", dir.display()));
    }
    Ok(missing)
}

// `remove_dir` only removes empty directories, so nothing but this run's leftovers can go.
fn remove_created(dirs: &[PathBuf]) {
    for dir in dirs {
        let _ = std::fs::remove_dir(dir);
    }
}

/// What `--update` did: the collection root and core's summary.
struct Updated {
    root: PathBuf,
    folder: Option<ItemPath>,
    dry_run: bool,
    summary: UpdateSummary,
}

// Parse and open the target before anything is written. A prune moves files to the system
// trash, and core refuses it up front when none is available.
fn import_update(args: &CollectionArgs) -> anyhow::Result<Updated> {
    let bundle = update_bundle(args)?;
    let dir = args.into.as_deref().unwrap_or(Path::new("."));
    let root =
        std::fs::canonicalize(dir).with_context(|| format!("cannot open {}", dir.display()))?;
    let mut collection = Collection::open(&root)?;
    let update = update_options(args)?;
    let summary = if args.dry_run {
        plan_update(&collection, &bundle, &update)?
    } else {
        apply_update(&mut collection, &bundle, &update, Some(&SystemTrash::new()))?
    };
    Ok(Updated {
        root,
        folder: update.folder,
        dry_run: args.dry_run,
        summary,
    })
}

// Read the input and require exactly one collection that came from an OpenAPI description.
fn update_bundle(args: &CollectionArgs) -> anyhow::Result<ImportBundle> {
    let (input, default_name) = match link(&args.file)? {
        Some(url) => (fetch_link(url, &args.fetch)?, default_import_name(url)),
        None => (read_file(&args.file)?, file_stem(&args.file)),
    };
    let options = collection_options(args, &input, default_name)?;
    let mut bundles = parse_collections(args.from.0, &input, &options)?;
    if !bundles.iter().all(|b| b.collection.spec_source) {
        return Err(anyhow!("--update needs an OpenAPI description"));
    }
    if bundles.len() != 1 {
        return Err(anyhow!("--update needs an input with one collection"));
    }
    Ok(bundles.remove(0))
}

fn update_options(args: &CollectionArgs) -> anyhow::Result<UpdateOptions> {
    let mut update = UpdateOptions::default();
    update.prune = args.prune;
    // Prunes are recorded where `item rm` records, so `item restore` brings them back.
    update.trash_log = TrashLog::default_dir().map(TrashLog::new);
    // An unknown folder is not an error here: core reports the no-baseline note for it.
    update.folder = args
        .folder
        .as_deref()
        .map(|raw| ItemPath::new(raw).ok_or_else(|| anyhow!("invalid folder path: {raw}")))
        .transpose()?;
    Ok(update)
}

// Text puts the counts first, then the changes one per line on stdout; kept requests are only
// counted. JSON carries the same summary as one object.
fn print_update(format: Format, updated: &Updated) -> anyhow::Result<()> {
    match format {
        Format::Text => {
            print_update_text(updated);
            Ok(())
        }
        Format::Json => print_update_json(updated),
    }
}

fn print_update_text(updated: &Updated) {
    let Updated {
        folder,
        dry_run,
        summary: s,
        ..
    } = updated;
    if let Some(folder) = folder {
        outln!("folder: {}", folder.as_str());
    }
    // A dry run names actions in the conditional; edited, orphaned and kept are states, so
    // they read the same in both modes.
    let (add, change, prune) = if *dry_run {
        ("would add", "would change", "would prune")
    } else {
        ("added", "changed", "pruned")
    };
    outln!(
        "{add}: {}, {change}: {}, kept: {}, edited: {}, orphaned: {}, {prune}: {}",
        s.added.len(),
        s.changed.len(),
        s.kept.len(),
        s.edited.len(),
        s.orphaned.len(),
        s.pruned.len()
    );
    for path in &s.added {
        outln!("{add} {}", path.as_str());
    }
    for change_item in &s.changed {
        let fields: Vec<_> = change_item.fields.iter().map(|f| f.id()).collect();
        outln!(
            "{change} {} ({})",
            change_item.path.as_str(),
            fields.join(", ")
        );
    }
    // An edited request names the fields found edited when the planner knows them.
    for path in &s.edited {
        match s.edited_fields.iter().find(|e| e.path == *path) {
            Some(e) => {
                let fields: Vec<_> = e.fields.iter().map(|f| f.id()).collect();
                outln!("edited {} ({})", path.as_str(), fields.join(", "));
            }
            None => outln!("edited {}", path.as_str()),
        }
    }
    for path in &s.orphaned {
        outln!("orphaned {}", path.as_str());
    }
    // Only a real run says where the pruned files went.
    let moved = if *dry_run {
        ""
    } else {
        " (moved to the trash)"
    };
    for path in &s.pruned {
        outln!("{prune} {}{moved}", path.as_str());
    }
    print_notes(&s.notes);
}

fn print_update_json(updated: &Updated) -> anyhow::Result<()> {
    let Updated {
        root,
        folder,
        dry_run,
        summary: s,
    } = updated;
    fn paths(v: &[ItemPath]) -> Vec<&str> {
        v.iter().map(ItemPath::as_str).collect()
    }
    let with_fields = |v: &[apinata_core::import::RequestChange]| -> Vec<serde_json::Value> {
        v.iter()
            .map(|c| {
                let fields: Vec<_> = c.fields.iter().map(|f| f.id()).collect();
                json!({"path": c.path.as_str(), "fields": fields})
            })
            .collect()
    };
    let changed = with_fields(&s.changed);
    let mut value = json!({
        "collection": root.display().to_string(),
        "dry_run": dry_run,
        "added": paths(&s.added),
        "changed": changed,
        "kept": paths(&s.kept),
        "edited": paths(&s.edited),
        "edited_fields": with_fields(&s.edited_fields),
        "orphaned": paths(&s.orphaned),
        "pruned": paths(&s.pruned),
        "notes": notes_json(&s.notes),
    });
    if let Some(folder) = folder {
        value["folder"] = json!(folder.as_str());
    }
    if !*dry_run && !s.pruned.is_empty() {
        value["pruned_destination"] = json!("trash");
    }
    outln!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

// Notes go to stderr so stdout stays the list of what was written.
pub(crate) fn print_notes(notes: &[ImportNote]) {
    for note in notes {
        let file = note
            .file
            .as_ref()
            .map_or_else(String::new, |f| format!(" ({f})"));
        eprintln!(
            "note: {} {}: {}{file}",
            note.kind.id(),
            note.source,
            note.detail
        );
    }
}

fn notes_json(notes: &[ImportNote]) -> Vec<serde_json::Value> {
    notes
        .iter()
        .map(|n| json!({"source": n.source, "kind": n.kind.id(), "detail": n.detail, "file": n.file}))
        .collect()
}

// Refuse a flag the input's format would ignore; undetectable input skips the check and fails
// in the parse instead. Only an explicit `--name` counts, never the file-stem default.
fn collection_options(
    args: &CollectionArgs,
    input: &[u8],
    default_name: Option<String>,
) -> anyhow::Result<CollectionOptions> {
    let format = args.from.0.or_else(|| CollectionFormat::detect(input));
    if let Some(format) = format {
        if args.group_by.is_some() && format != CollectionFormat::OpenApi {
            return Err(anyhow!("--group-by applies to OpenAPI input only"));
        }
        if args.name.is_some() && format != CollectionFormat::Har {
            return Err(anyhow!("--name applies to HAR input only"));
        }
    }
    let mut options = CollectionOptions::default();
    options.group_by = match args.group_by {
        Some(GroupByArg::Path) => GroupBy::Path,
        Some(GroupByArg::Tags) | None => GroupBy::Tags,
    };
    options.name = args.name.clone().or(default_name);
    Ok(options)
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

/// The source as a link to fetch, decided by core's classifier: `Some` for http(s), an error
/// for any other `scheme://` (a `file://` URL included, since the CLI takes plain paths), `None`
/// for a path, which is read later and reports its own failures.
// The classifier also stats an absolute path, but the CLI's own "cannot read" messages differ
// from core's Missing and Folder ones, and reading the file is needed anyway, so the outcome
// is not carried over.
fn link(source: &Path) -> anyhow::Result<Option<&str>> {
    let Some(text) = source.to_str() else {
        return Ok(None);
    };
    let refuse = |scheme: &str| {
        Err(anyhow!(
            "cannot import {scheme}:// links: only http and https links or file paths are accepted"
        ))
    };
    match classify_source(text) {
        Ok(ImportSource::Link(_)) => Ok(Some(text.trim())),
        Err(SourceError::Scheme(scheme)) => refuse(&scheme),
        Err(e @ SourceError::NotALink(_)) => Err(anyhow!("{e}")),
        _ if text.trim().to_ascii_lowercase().starts_with("file://") => refuse("file"),
        _ => Ok(None),
    }
}

/// Fetch `url` once under the network flags, with no approval. Failures read as
/// [`fetch_message`] and never carry the URL.
fn fetch_link(url: &str, args: &FetchNetworkArgs) -> anyhow::Result<Vec<u8>> {
    let settings = network::settings_from(
        args.noproxy,
        args.cacert.clone(),
        args.ignore_truststore,
        args.proxy_pac.clone(),
    );
    let mut fetch = SourceFetch::default().network(settings);
    fetch.insecure = args.insecure;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime
        .block_on(fetch_source(url, &fetch))
        .map_err(|e| anyhow!(fetch_message(&e)))
}

/// One clear sentence per way a fetch can fail.
pub(crate) fn fetch_message(error: &FetchError) -> String {
    match error {
        FetchError::Scheme(why) => format!("cannot import the link: {why}"),
        FetchError::Transport(why) => format!("cannot fetch the link: {why}"),
        FetchError::Timeout => "cannot fetch the link: it timed out".to_owned(),
        FetchError::Status(code) => {
            format!("cannot fetch the link: the server answered HTTP {code}")
        }
        FetchError::TooLarge { max_bytes } => {
            format!("cannot fetch the link: the body is larger than {max_bytes} bytes")
        }
        FetchError::TooManyRedirects => "cannot fetch the link: too many redirects".to_owned(),
        FetchError::HostNotAllowed { host, .. } => {
            format!("cannot fetch the link: host `{host}` is not allowed")
        }
        other => format!("cannot fetch the link: {other}"),
    }
}

#[cfg(test)]
mod fetch_message_tests {
    use super::*;

    #[test]
    fn each_failure_has_its_own_message() {
        let all = [
            FetchError::Scheme("not a valid URL".into()),
            FetchError::Transport("connection refused".into()),
            FetchError::Timeout,
            FetchError::Status(503),
            FetchError::TooLarge {
                max_bytes: 67_108_864,
            },
            FetchError::TooManyRedirects,
            FetchError::HostNotAllowed {
                host: "h.test".into(),
                hop: 0,
            },
        ];
        let messages: Vec<_> = all.iter().map(fetch_message).collect();
        let mut unique = messages.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), all.len(), "{messages:?}");
        assert!(messages[3].contains("HTTP 503"));
        assert!(messages[4].contains("67108864"));
        assert!(messages[5].contains("redirects"));
        assert!(messages[2].contains("timed out"));
    }
}

// Parse every file first; a write failure then keeps the environments already written, each
// complete, and names both sets.
fn import_environments(args: &EnvironmentArgs) -> anyhow::Result<Imported> {
    let mut envs = Vec::new();
    for path in &args.files {
        let parsed = parse_environments(args.from.0, &read_file(path)?)
            .with_context(|| format!("cannot import {}", path.display()))?;
        envs.extend(parsed);
    }
    let root = collection_root(args.collection.as_deref())?;
    let mut collection = Collection::open(&root)?;
    let secrets = secret_store(args.no_keychain);
    let mut report = ImportReport::default();
    for (i, env) in envs.iter().enumerate() {
        match add_environment(&mut collection, env, secrets.as_ref()) {
            Ok((_, one)) => {
                report.notes.extend(one.notes);
                report.items.extend(one.items);
                report.environments.extend(one.environments);
                report.secrets_to_enter.extend(one.secrets_to_enter);
            }
            Err(error) if report.environments.is_empty() => return Err(error.into()),
            Err(error) => {
                return Err(Partial {
                    error,
                    written: report.environments,
                    not_written: envs[i..].iter().map(|e| e.name.clone()).collect(),
                }
                .into());
            }
        }
    }
    Ok(Imported {
        listed: report.environments.clone(),
        root,
        report,
    })
}

fn read_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))
}

pub(crate) fn secret_store(no_keychain: bool) -> Arc<dyn SecretStore> {
    if no_keychain {
        Arc::new(NoKeychain)
    } else {
        platform_store()
    }
}

/// The store `run`, `check`, `import` and `env` read secret values from: the platform keychain,
/// opened on first use and bounded per call. Debug builds only: `APINATA_TEST_NO_KEYCHAIN` set
/// swaps in an empty memory store, so the suite never reaches a real keychain, and
/// `APINATA_TEST_REPORT_STORE` names, once on stderr, the store a command first used.
pub(crate) fn platform_store() -> Arc<dyn SecretStore> {
    #[cfg(debug_assertions)]
    if std::env::var_os("APINATA_TEST_NO_KEYCHAIN").is_some() {
        return reported("memory", Arc::new(apinata_core::MemoryStore::new()));
    }
    bounded_keychain(
        reported("platform", Arc::new(LazyKeychain::default())),
        KEYCHAIN_TIMEOUT,
    )
}

/// `store`, naming itself on stderr at its first call when `APINATA_TEST_REPORT_STORE` is set,
/// so the line says what a command actually opened, not what it constructed. Release builds
/// return `store` unchanged.
fn reported(name: &'static str, store: Arc<dyn SecretStore>) -> Arc<dyn SecretStore> {
    #[cfg(debug_assertions)]
    if std::env::var_os("APINATA_TEST_REPORT_STORE").is_some() {
        return Arc::new(ReportOnUse { name, inner: store });
    }
    let _ = name;
    store
}

#[cfg(debug_assertions)]
struct ReportOnUse {
    name: &'static str,
    inner: Arc<dyn SecretStore>,
}

#[cfg(debug_assertions)]
impl ReportOnUse {
    fn report(&self) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| eprintln!("secret store: {}", self.name));
    }
}

#[cfg(debug_assertions)]
impl SecretStore for ReportOnUse {
    fn get(&self, key: &SecretKey) -> Result<Option<String>, SecretError> {
        self.report();
        self.inner.get(key)
    }

    fn set(&self, key: &SecretKey, value: &str) -> Result<(), SecretError> {
        self.report();
        self.inner.set(key, value)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.report();
        self.inner.delete(key)
    }

    fn list(&self, collection: &Path) -> Result<Vec<SecretKey>, SecretError> {
        self.report();
        self.inner.list(collection)
    }
}

pub(crate) fn default_keychain() -> Arc<dyn SecretStore> {
    platform_store()
}

/// `inner` with every call held to `limit`, so a hung secret service reports the keychain as
/// unavailable (and the caller shows the store-error hint) instead of stalling the command.
pub(crate) fn bounded_keychain(
    inner: Arc<dyn SecretStore>,
    limit: Duration,
) -> Arc<dyn SecretStore> {
    Arc::new(TimeoutStore::new(inner, limit))
}

/// The `--no-keychain` store: nothing is kept, so every secret value needs input.
struct NoKeychain;

impl SecretStore for NoKeychain {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Ok(None)
    }

    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(SecretError::Unavailable {
            message: "--no-keychain".to_owned(),
        })
    }

    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
}

fn read_input(file: Option<&Path>) -> anyhow::Result<String> {
    match file {
        Some(path) if path != Path::new("-") => {
            std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
        }
        _ => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("cannot read stdin")?;
            Ok(text)
        }
    }
}

fn collection_root(dir: Option<&Path>) -> anyhow::Result<PathBuf> {
    let dir = dir.unwrap_or(Path::new("."));
    let path =
        std::fs::canonicalize(dir).with_context(|| format!("cannot open {}", dir.display()))?;
    Ok(find_root(&path)?.to_path_buf())
}

// Text lists what was written on stdout and the notes on stderr; JSON is one object on stdout
// per call, so a multi-collection import prints a stream of values.
fn print(format: Format, imported: &Imported) -> anyhow::Result<()> {
    let Imported {
        root,
        listed,
        report,
    } = imported;
    match format {
        Format::Text => {
            for line in listed {
                outln!("{line}");
            }
            print_notes(&report.notes);
        }
        Format::Json => {
            let secrets: Vec<_> = report
                .secrets_to_enter
                .iter()
                .map(|(env, var)| json!({"environment": env, "variable": var}))
                .collect();
            let items: Vec<_> = report.items.iter().map(ItemPath::as_str).collect();
            let value = json!({
                "collection": root.display().to_string(),
                "items": items,
                "environments": report.environments,
                "notes": notes_json(&report.notes),
                "secrets_to_enter": secrets,
            });
            outln!("{}", serde_json::to_string_pretty(&value)?);
        }
    }
    Ok(())
}

/// The name for a request imported without `--name`: the URL's last non-empty path segment,
/// else its host without userinfo or port, else `request`. No decoding, no template awareness.
pub(crate) fn default_name(url: &str) -> String {
    let mut rest = url.trim();
    if let Some((scheme, after)) = rest.split_once("://") {
        let mut chars = scheme.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if valid {
            rest = after;
        }
    }
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if let Some(segment) = path.rsplit('/').find(|s| !s.is_empty()) {
        return segment.to_owned();
    }
    // Userinfo ends at the last `@`; a port starts at the first `:` past an IPv6 literal.
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let port_from = host.rfind(']').unwrap_or(0);
    let host = match host[port_from..].find(':') {
        Some(i) => &host[..port_from + i],
        None => host,
    };
    if host.is_empty() {
        "request".to_owned()
    } else {
        host.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{default_name, exit_code};
    use apinata_core::CreateError;
    use apinata_core::import::ImportError;
    use std::path::PathBuf;

    #[test]
    fn default_names_follow_the_last_path_segment_else_the_host() {
        let cases = [
            ("https://x.test/users/42?x=1#f", "42"),
            ("https://u@x.test:8080/v1/items/?q=/y#/z", "items"),
            ("{{baseUrl}}/users", "users"),
            ("{{baseUrl}}", "{{baseUrl}}"),
            ("{{baseUrl}}?q=/a", "{{baseUrl}}"),
            ("https://x.test/", "x.test"),
            ("https://x.test#/f", "x.test"),
            ("https://u:p@x.test:8080", "x.test"),
            ("x.test/a/", "a"),
            ("x.test:{{port}}/", "x.test"),
            ("https://", "request"),
            ("  ", "request"),
            ("", "request"),
        ];
        for (url, name) in cases {
            assert_eq!(default_name(url), name, "{url}");
        }
    }

    #[test]
    fn every_import_failure_exits_3() {
        let io = || std::io::Error::other("disk");
        let p = || PathBuf::from("/x");
        assert_eq!(
            exit_code(&ImportError::Io {
                path: p(),
                source: io()
            }),
            3
        );
        assert_eq!(exit_code(&ImportError::Exists { path: p() }), 3);
        assert_eq!(
            exit_code(&ImportError::Create(CreateError::Exists { path: p() })),
            3
        );
        assert_eq!(
            exit_code(&ImportError::Create(CreateError::NotEmpty { path: p() })),
            3
        );
        assert_eq!(
            exit_code(&ImportError::Create(CreateError::Io {
                path: p(),
                source: io()
            })),
            3
        );
        assert_eq!(
            exit_code(&ImportError::Create(CreateError::InvalidName {
                name: String::new()
            })),
            3
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_reporting_wrapper_forwards_list() {
        use super::{Arc, Path, ReportOnUse, SecretKey, SecretStore};
        let inner = apinata_core::MemoryStore::new();
        inner.set(&SecretKey::new("/a", "dev", "x"), "1").unwrap();
        let store = ReportOnUse {
            name: "memory",
            inner: Arc::new(inner),
        };
        assert_eq!(store.list(Path::new("/a")).unwrap().len(), 1);
    }
}
