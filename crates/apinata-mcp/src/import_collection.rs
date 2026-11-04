//! `import_collection`: parse a collection or API description and add its requests to the
//! served collection as an ordinary folder, so they appear in every list tool.
//!
//! Only the items are taken. The import is written by core into a scratch directory and copied
//! under `<into>/<collection name>/`; the imported root's own settings and every environment
//! are reported as skipped, and no secret store is ever involved.

use std::fs;

use apinata_core::import::{
    CollectionFormat, CollectionOptions, GroupBy, ImportBundle, ImportError, UpdateOptions,
    UpdateSummary, apply_update, ensure_folder, parse_collections, place_items, plan_update,
};
use apinata_core::{
    AuthSpec, Collection, ItemPath, SecretError, SecretKey, SecretStore, Trash, TrashLog,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::imports::NoteDoc;

/// The folder imports go in when `into` is absent.
const DEFAULT_INTO: &str = "imports";

/// The request label an approval filed for a refused fetch carries.
pub(crate) const FETCH_LABEL: &str = "import_collection";

/// `import_collection`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportCollectionArgs {
    /// An http or https link to a collection or API description. Exactly one of `url` and `text`.
    #[serde(default)]
    pub url: Option<String>,
    /// The collection or API description itself, JSON or YAML. Exactly one of `url` and `text`.
    #[serde(default)]
    pub text: Option<String>,
    /// `auto` (the default) detects the format; otherwise `postman`, `openapi`, `insomnia`,
    /// `hoppscotch` or `har`.
    #[serde(default)]
    pub format: Option<String>,
    /// For OpenAPI only: `tags` (the default) or `path`, how operations become folders.
    #[serde(default)]
    pub group_by: Option<String>,
    /// For HAR only: the collection's name. A link's default is its last path segment.
    #[serde(default)]
    pub name: Option<String>,
    /// The folder, relative to the collection root, to import into; `imports` when absent.
    /// Created when missing. No `..`, no absolute path, no symlink. With `update` it is the
    /// exact folder to refresh, one that an earlier import recorded a baseline for (the
    /// folder the import reported); it is never created, and absent means the root.
    #[serde(default)]
    pub into: Option<String>,
    /// OpenAPI only: refresh the served collection from this description instead of adding a
    /// folder. Works on the folder `into` names, or the collection root when absent; either
    /// needs the baseline an import of the description recorded.
    #[serde(default)]
    pub update: Option<bool>,
    /// With `update`: also move to the system trash requests whose operation left the
    /// description, unless edited; refused when no trash is available. `restore_pruned` puts
    /// them back.
    #[serde(default)]
    pub prune: Option<bool>,
    /// With `update`: report what would change and write nothing.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

/// What was written and what was left out.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ImportedCollections {
    /// One entry per collection in the input, in order.
    pub collections: Vec<ImportedFolder>,
    /// What the input held that was not imported: environments and the imported root's own
    /// headers, variables, auth and scripts.
    pub skipped: Vec<String>,
    /// One note per part of the input that was dropped, converted or needs input.
    pub notes: Vec<NoteEntry>,
    /// Present for `update` only: what the refresh did.
    #[serde(flatten)]
    pub update: Option<UpdateReport>,
}

/// A note of an import, or of an update (which also names its file).
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub(crate) enum NoteEntry {
    Import(NoteDoc),
    Update(UpdateNoteDoc),
}

/// One update note, shaped like the CLI's: `file` is always present, null when absent.
#[derive(Serialize, JsonSchema)]
pub(crate) struct UpdateNoteDoc {
    /// Where in the input the note comes from.
    pub source: String,
    /// `dropped`, `converted`, `renamed` or `needs-input`.
    pub kind: String,
    /// What happened to it.
    pub detail: String,
    /// The collection-relative file the note concerns, or null.
    pub file: Option<String>,
}

/// What an update did, shaped like the CLI's JSON; its `notes` are the response's `notes`.
#[derive(Serialize, JsonSchema)]
pub(crate) struct UpdateReport {
    /// The collection root that was updated.
    pub collection: String,
    /// True when nothing was written.
    pub dry_run: bool,
    /// New requests, by path.
    pub added: Vec<String>,
    /// Requests with refreshed fields.
    pub changed: Vec<ChangedDoc>,
    /// Matched requests with nothing to refresh.
    pub kept: Vec<String>,
    /// Requests whose spec-owned fields were edited locally and stay as they are.
    pub edited: Vec<String>,
    /// For those requests, the spec-owned fields found edited (such as `method`).
    pub edited_fields: Vec<ChangedDoc>,
    /// Requests whose operation left the description.
    pub orphaned: Vec<String>,
    /// Orphans moved to the trash, or that would be.
    pub pruned: Vec<String>,
    /// `"trash"` when orphans were actually moved; absent on a dry run or when none were.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pruned_destination: Option<String>,
}

/// One refreshed request.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ChangedDoc {
    /// The request's collection-relative path.
    pub path: String,
    /// The fields refreshed.
    pub fields: Vec<String>,
}

/// One imported collection's folder.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ImportedFolder {
    /// The folder's collection-relative path.
    pub path: String,
    /// The requests written in it, at any depth.
    pub requests: usize,
    /// The folders written inside it, at any depth.
    pub folders: usize,
}

/// The arguments after validation, before the input is known.
pub(crate) struct Plan {
    pub into: Vec<String>,
    /// The folder an update refreshes; `None` is the collection root.
    folder: Option<Vec<String>>,
    format: Option<CollectionFormat>,
    group_by: Option<GroupBy>,
    name: Option<String>,
    /// `Some((prune, dry_run))` when the call updates the served collection.
    update: Option<(bool, bool)>,
}

/// Check everything that needs no input: one source, a known format and grouping, a safe
/// `into`. Runs before any fetch so a bad argument never costs a request.
pub(crate) fn plan(a: &ImportCollectionArgs) -> Result<Plan, String> {
    if a.url.is_some() == a.text.is_some() {
        return Err("give exactly one of `url` and `text`".to_owned());
    }
    let format = match a.format.as_deref() {
        None | Some("auto") => None,
        Some(id) => Some(CollectionFormat::from_id(id).ok_or_else(|| unknown_format(id))?),
    };
    let group_by = match a.group_by.as_deref() {
        None => None,
        Some("tags") => Some(GroupBy::Tags),
        Some("path") => Some(GroupBy::Path),
        Some(other) => return Err(format!("unknown group_by {other}; expected tags, path")),
    };
    let (prune, dry_run) = (a.prune.unwrap_or(false), a.dry_run.unwrap_or(false));
    let update = a.update.unwrap_or(false);
    if !update && (prune || dry_run) {
        return Err("`prune` and `dry_run` require `update`".to_owned());
    }
    let folder = match (update, a.into.as_deref()) {
        (true, Some(raw)) => Some(into_segments(raw)?),
        _ => None,
    };
    Ok(Plan {
        into: into_segments(a.into.as_deref().unwrap_or(DEFAULT_INTO))?,
        folder,
        format,
        group_by,
        name: a.name.clone(),
        update: update.then_some((prune, dry_run)),
    })
}

impl Plan {
    pub(crate) fn is_update(&self) -> bool {
        self.update.is_some()
    }
}

fn unknown_format(id: &str) -> String {
    let ids: Vec<&str> = std::iter::once("auto")
        .chain(CollectionFormat::ALL.iter().map(|f| f.id()))
        .collect();
    format!("unknown format {id}; expected {}", ids.join(", "))
}

/// `into` as folder names: relative, each a plain name.
fn into_segments(raw: &str) -> Result<Vec<String>, String> {
    let bad = |why: &str| Err(format!("`into` {why}: {raw:?}"));
    if raw.starts_with('/') || raw.starts_with('\\') || raw.contains(':') {
        return bad("must be relative to the collection root");
    }
    let segments: Vec<&str> = raw.split('/').collect();
    for s in &segments {
        if s.is_empty() || s.contains(['\\', '\0']) {
            return bad("must be a folder path of plain names");
        }
        if *s == "." || *s == ".." {
            return bad("must not contain `.` or `..`");
        }
    }
    Ok(segments.into_iter().map(str::to_owned).collect())
}

/// Parse `input`, refusing options its format ignores like the CLI does. `default_name` is the
/// link's name; an explicit `name` wins and is only accepted for HAR.
pub(crate) fn parse(
    plan: &Plan,
    input: &[u8],
    default_name: Option<String>,
) -> Result<Vec<ImportBundle>, String> {
    if let Some(format) = plan.format.or_else(|| CollectionFormat::detect(input)) {
        if plan.group_by.is_some() && format != CollectionFormat::OpenApi {
            return Err("group_by applies to OpenAPI input only".to_owned());
        }
        if plan.name.is_some() && format != CollectionFormat::Har {
            return Err("name applies to HAR input only".to_owned());
        }
    }
    let mut options = CollectionOptions::default();
    options.group_by = plan.group_by.unwrap_or_default();
    options.name = plan.name.clone().or(default_name);
    parse_collections(plan.format, input, &options).map_err(|e| e.to_string())
}

/// A store for `write_collection` that holds nothing: no environment is written, so core never
/// asks, and anything that did ask would learn that no store is available.
pub(crate) struct NoSecrets;

impl NoSecrets {
    fn refuse() -> SecretError {
        SecretError::Unavailable {
            message: "import_collection keeps no secrets".to_owned(),
        }
    }
}

impl SecretStore for NoSecrets {
    fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
        Err(Self::refuse())
    }
    fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
        Err(Self::refuse())
    }
    fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
        Err(Self::refuse())
    }
}

/// Refresh `c` from the description in `bundles` (core's update; a prune moves files to `bin`,
/// recorded in `log` as `delete_item` records, and is refused when there is none), then reload. Needs exactly one OpenAPI bundle. A
/// collection without a recorded baseline is left untouched and the response says so.
pub(crate) fn update(
    c: &mut Collection,
    plan: &Plan,
    bundles: &[ImportBundle],
    bin: &dyn Trash,
    log: Option<TrashLog>,
) -> Result<ImportedCollections, String> {
    let (prune, dry_run) = plan.update.unwrap_or_default();
    if !bundles.iter().all(|b| b.collection.spec_source) {
        return Err("update needs an OpenAPI description".to_owned());
    }
    let [bundle] = bundles else {
        return Err("update needs an input with one collection".to_owned());
    };
    let folder = existing_folder(plan.folder.as_deref())?;
    let mut options = UpdateOptions::default();
    options.prune = prune;
    options.folder = Some(folder.clone());
    options.trash_log = log;
    let has_baseline = c.file().spec_baseline_in(&folder).is_some();
    let summary = if dry_run {
        plan_update(c, bundle, &options)
    } else {
        apply_update(c, bundle, &options, Some(bin))
    };
    c.reload();
    Ok(report(
        c,
        dry_run,
        has_baseline,
        summary.map_err(|e| e.to_string())?,
    ))
}

fn report(
    c: &Collection,
    dry_run: bool,
    has_baseline: bool,
    s: UpdateSummary,
) -> ImportedCollections {
    fn paths(v: &[ItemPath]) -> Vec<String> {
        v.iter().map(|p| p.as_str().to_owned()).collect()
    }
    // Core explains the missing baseline in its one note; say outright that nothing was done.
    let tail = if has_baseline {
        ""
    } else {
        ". Nothing was changed."
    };
    ImportedCollections {
        collections: Vec::new(),
        skipped: Vec::new(),
        notes: s
            .notes
            .iter()
            .map(|n| {
                NoteEntry::Update(UpdateNoteDoc {
                    source: n.source.clone(),
                    kind: n.kind.id().to_owned(),
                    detail: format!("{}{tail}", n.detail),
                    file: n.file.clone(),
                })
            })
            .collect(),
        update: Some(UpdateReport {
            collection: c.root().display().to_string(),
            dry_run,
            added: paths(&s.added),
            changed: s
                .changed
                .iter()
                .map(|ch| ChangedDoc {
                    path: ch.path.as_str().to_owned(),
                    fields: ch.fields.iter().map(|f| f.id().to_owned()).collect(),
                })
                .collect(),
            kept: paths(&s.kept),
            edited: paths(&s.edited),
            edited_fields: s
                .edited_fields
                .iter()
                .map(|ch| ChangedDoc {
                    path: ch.path.as_str().to_owned(),
                    fields: ch.fields.iter().map(|f| f.id().to_owned()).collect(),
                })
                .collect(),
            orphaned: paths(&s.orphaned),
            pruned: paths(&s.pruned),
            pruned_destination: (!dry_run && !s.pruned.is_empty()).then(|| "trash".to_owned()),
        }),
    }
}

/// The folder at `segments` (the root when `None`) for an update, validated as a path only.
/// Nothing is created: a folder that is missing, a file or a symlink holds no baseline, so
/// core ends the update in its no-baseline note instead of writing.
fn existing_folder(segments: Option<&[String]>) -> Result<ItemPath, String> {
    segments
        .unwrap_or_default()
        .iter()
        .try_fold(ItemPath::root(), |folder, segment| {
            folder
                .join(segment)
                .ok_or_else(|| format!("`into` has an invalid folder name: {segment:?}"))
        })
}

/// Add each bundle's items to `c` under `plan.into`, one new folder per bundle, then reload so
/// the new requests are part of the tree.
pub(crate) fn place(
    c: &mut Collection,
    plan: &Plan,
    bundles: &[ImportBundle],
    secrets: &dyn SecretStore,
) -> Result<ImportedCollections, String> {
    let outcome = place_bundles(c, plan, bundles, secrets);
    // Even a failed import may have left files behind that the tree should show or drop.
    c.reload();
    outcome
}

fn place_bundles(
    c: &mut Collection,
    plan: &Plan,
    bundles: &[ImportBundle],
    secrets: &dyn SecretStore,
) -> Result<ImportedCollections, String> {
    let into = ensure_folder(c, &plan.into).map_err(folder_error)?;
    let mut result = ImportedCollections {
        collections: Vec::new(),
        skipped: Vec::new(),
        notes: Vec::new(),
        update: None,
    };
    for bundle in bundles {
        // Environments are never written, so the report cannot mention a secret to enter.
        let folder = c
            .create_folder(&into, &bundle.collection.name)
            .map_err(|e| e.to_string())?;
        let placed = match place_items(c, &folder, bundle, secrets) {
            Ok(placed) => placed,
            Err(e) => {
                let _ = fs::remove_dir_all(c.root().join(folder.as_str()));
                return Err(format!("cannot write the imported items: {e}"));
            }
        };
        result.collections.push(ImportedFolder {
            path: folder.to_string(),
            requests: placed.requests,
            folders: placed.report.items.len() - placed.requests,
        });
        result.skipped.extend(skipped(bundle));
        result
            .notes
            .extend(placed.report.notes.into_iter().map(|n| {
                NoteEntry::Import(NoteDoc {
                    source: n.source,
                    kind: n.kind.id().to_owned(),
                    detail: n.detail,
                })
            }));
    }
    Ok(result)
}

/// A core folder error as this tool words it, naming the `into` argument.
fn folder_error(e: ImportError) -> String {
    match e {
        ImportError::Folder { problem } => format!("`into` {problem}"),
        other => other.to_string(),
    }
}

/// What `bundle` held that the import leaves out.
fn skipped(bundle: &ImportBundle) -> Vec<String> {
    let root = &bundle.collection;
    let name = &root.name;
    let mut out: Vec<String> = bundle
        .environments
        .iter()
        .map(|e| format!("{name}: environment `{}`", e.name))
        .collect();
    let scripts = !root.scripts.pre_request.trim().is_empty()
        || !root.scripts.post_response.trim().is_empty();
    for (present, what) in [
        (!root.headers.is_empty(), "collection headers"),
        (!root.vars.is_empty(), "collection variables"),
        (
            !matches!(root.auth, AuthSpec::None | AuthSpec::Inherit),
            "collection auth",
        ),
        (scripts, "collection scripts"),
    ] {
        if present {
            out.push(format!("{name}: {what}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Counting(AtomicUsize);

    impl SecretStore for Counting {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // A Postman collection with a secret-looking collection variable and an environment-like
    // payload: whatever it holds, the store must see no call.
    const INPUT: &str = r#"{"info":{"name":"Col","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
"item":[{"name":"Get","request":{"method":"GET","url":"http://127.0.0.1:1/x"}}],
"variable":[{"key":"token","value":"s3cret"}]}"#;

    #[test]
    fn the_secret_store_is_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Collection::create(dir.path(), "shop").unwrap();
        let args = ImportCollectionArgs {
            url: None,
            text: Some(INPUT.to_owned()),
            format: None,
            group_by: None,
            name: None,
            into: None,
            update: None,
            prune: None,
            dry_run: None,
        };
        let plan = plan(&args).unwrap();
        let bundles = parse(&plan, INPUT.as_bytes(), None).unwrap();
        let store = Counting::default();
        place(&mut c, &plan, &bundles, &store).unwrap();
        assert_eq!(store.0.load(Ordering::SeqCst), 0);
        assert!(dir.path().join("imports/Col/Get.yml").is_file());
    }
}
