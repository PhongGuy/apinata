//! Placing an imported collection's items into a folder of an existing collection.
//!
//! The bundle is written like [`write_collection`](super::write_collection) into a scratch
//! directory and its items are copied across; the imported root's settings go onto the
//! folder and its environments are added beside the target's own, so the target's root file
//! is never touched. This is the one routine the CLI and the MCP server share.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::environment::add_environment;
use super::protect::{Held, keep_taken, protect_collection};
use super::write::{placed, push_unset_note, write_staged};
use super::{
    ImportBundle, ImportError, ImportNote, ImportReport, ImportedCollection, ImportedScripts,
};
use crate::collection::{Collection, FOLDER_FILE, ROOT_FILE};
use crate::edit::{AuthSpec, KeyValue, Variable};
use crate::error::TreeError;
use crate::item_path::ItemPath;
use crate::secrets::SecretStore;

/// What [`place_items`] wrote.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Placed {
    /// The folder the items went into.
    pub folder: ItemPath,
    /// Core's report; its `items` are relative to `folder`.
    pub report: ImportReport,
    /// How many of the items are requests, at any depth.
    pub requests: usize,
}

/// The folder at `segments` under the root, created where missing. Fails with
/// [`ImportError::Folder`] when a name is invalid, passes through a symlink, is a file, or the
/// result resolves outside the collection.
pub fn ensure_folder(
    collection: &mut Collection,
    segments: &[String],
) -> Result<ItemPath, ImportError> {
    let bad = |problem: String| ImportError::Folder { problem };
    let io_error = |path: &Path, source: io::Error| ImportError::Io {
        path: path.to_path_buf(),
        source,
    };
    let root = collection
        .root()
        .canonicalize()
        .map_err(|e| io_error(collection.root(), e))?;
    let mut folder = ItemPath::root();
    for segment in segments {
        let next = folder
            .join(segment)
            .ok_or_else(|| bad(format!("has an invalid folder name: {segment:?}")))?;
        let path = collection.root().join(next.as_str());
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(bad(format!("passes through a symlink: {segment:?}")));
            }
            Ok(meta) if meta.is_dir() => folder = next,
            Ok(_) => {
                return Err(bad(format!("names a file, not a folder: {segment:?}")));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                folder = collection.create_folder(&folder, segment)?;
            }
            Err(e) => return Err(io_error(&path, e)),
        }
    }
    // The checks above look at each name as written; this one looks at where the result is.
    let dir = collection.root().join(folder.as_str());
    let real = dir.canonicalize().map_err(|e| io_error(&dir, e))?;
    if !real.starts_with(&root) {
        return Err(bad("resolves outside the collection".to_owned()));
    }
    Ok(folder)
}

/// Write `bundle` into the existing `folder` of `collection`, then reload. The items go into
/// the folder, the imported root's docs, auth, headers, variables and scripts onto its
/// `folder.yml` (each one present replaces the folder's own), and the environments into the
/// collection as [`add_environment`](super::add_environment) adds them. A credential taken out
/// of the root or an item is kept as a secret variable of every environment (of a new
/// `Environment` when there is none), its value in `secrets`, or noted as not stored. A name
/// already in the folder is refused with [`ImportError::Exists`] and nothing is written. An
/// OpenAPI bundle also records the folder's baseline, which lets a later update find its
/// requests. A failure removes what this call wrote and nothing else.
pub fn place_items(
    collection: &mut Collection,
    folder: &ItemPath,
    bundle: &ImportBundle,
    secrets: &dyn SecretStore,
) -> Result<Placed, ImportError> {
    let scratch = tempfile::tempdir().map_err(|source| ImportError::Io {
        path: std::env::temp_dir(),
        source,
    })?;
    // New secret names avoid every variable the target or the imported environments declare.
    let mut imported = bundle.collection.clone();
    let declared = collection
        .environments()
        .iter()
        .flat_map(|e| e.variables().into_iter().map(|v| v.name))
        .chain(
            bundle
                .environments
                .iter()
                .flat_map(|e| e.variables.iter().map(|v| v.name.clone())),
        )
        .collect::<Vec<_>>();
    let taken = protect_collection(&mut imported, declared, Held::new(), true);
    // The root's settings go onto the folder, so the staged root stays bare and no note of it
    // names the staged root file.
    let root = take_root(&mut imported);
    let (staged, mut report) =
        write_staged(scratch.path(), &imported, &[], secrets, false, &mut |_| {
            Ok(())
        })?;
    // A folder never records a source, so the staged root's note about one does not apply.
    if let Some(skip) = super::update::unrecorded_note(bundle.collection.spec_from.as_ref()) {
        report.notes.retain(|n| n.detail != skip.detail);
    }
    let dest = collection.root().join(folder.as_str());
    let settings_file = settings_path(collection, folder);
    let mut undo = Undo {
        copied: Vec::new(),
        settings: fs::read(&settings_file)
            .ok()
            .map(|bytes| (settings_file, bytes)),
        environments: Vec::new(),
    };
    if let Err(source) = copy_items(staged.root(), &dest, &mut undo.copied) {
        undo.run(collection, secrets);
        return Err(match source.kind() {
            io::ErrorKind::AlreadyExists => ImportError::Exists { path: dest },
            _ => ImportError::Io { path: dest, source },
        });
    }
    // The copied files are not in the tree yet.
    collection.reload();
    match write_root(collection, folder, &root) {
        Ok(file) => {
            let notes = placed(&root.notes, file);
            report.notes.splice(0..0, notes);
        }
        Err(e) => {
            undo.run(collection, secrets);
            return Err(e);
        }
    }
    for env in &bundle.environments {
        let before: Vec<String> = environment_names(collection);
        match add_environment(collection, env, secrets) {
            Ok((name, one)) => {
                undo.environments.push(name);
                report.notes.extend(one.notes);
                report.environments.extend(one.environments);
                report.secrets_to_enter.extend(one.secrets_to_enter);
            }
            Err(e) => {
                // A failed write may have created its file first, so every new name goes.
                let made = environment_names(collection)
                    .into_iter()
                    .filter(|n| !before.contains(n));
                undo.environments.extend(made);
                undo.run(collection, secrets);
                return Err(e);
            }
        }
    }
    if bundle.collection.spec_source {
        let placed: Vec<ItemPath> = report
            .items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .split('/')
                    .try_fold(folder.clone(), |p, segment| p.join(segment))
            })
            .collect();
        let ids = super::update::spec_operation_ids(&imported.items);
        if let Err(e) = super::write::record_baseline(collection, folder, &placed, &ids) {
            undo.run(collection, secrets);
            return Err(e);
        }
    }
    keep_taken(
        collection,
        &taken,
        Some(secrets),
        &mut report.notes,
        &mut report.secrets_to_enter,
    );
    super::unstored::fold(&mut report.notes);
    // Judged against the target, whose variables and environments the placed requests now use;
    // the target's own requests are not this import's concern.
    let placed: Vec<String> = report
        .items
        .iter()
        .map(|item| format!("{}/{}", folder.as_str(), item.as_str()))
        .map(|p| p.trim_start_matches('/').to_owned())
        .collect();
    let written = |item: &str| {
        placed.iter().any(|p| {
            item == p
                || item
                    .strip_prefix(p.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        })
    };
    push_unset_note(collection, &written, &mut report.notes);
    let requests = report
        .items
        .iter()
        .filter(|item| staged.request(item).is_some())
        .count();
    Ok(Placed {
        folder: folder.clone(),
        report,
        requests,
    })
}

/// What a failed placement takes back: the copied top-level paths, the settings file's bytes
/// before it was written, and the environments it added with their stored secrets.
struct Undo {
    copied: Vec<PathBuf>,
    settings: Option<(PathBuf, Vec<u8>)>,
    environments: Vec<String>,
}

impl Undo {
    fn run(&self, collection: &mut Collection, secrets: &dyn SecretStore) {
        for path in &self.copied {
            let _ = if path.is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
        }
        if let Some((path, bytes)) = &self.settings {
            let _ = fs::write(path, bytes);
        }
        for name in &self.environments {
            let _ = collection.delete_environment(name, secrets);
        }
        collection.reload();
    }
}

fn environment_names(collection: &Collection) -> Vec<String> {
    collection
        .environments()
        .iter()
        .map(|e| e.name().to_owned())
        .collect()
}

/// The imported root's own settings and notes, which a placement writes onto the folder.
struct RootSettings {
    docs: String,
    auth: AuthSpec,
    headers: Vec<KeyValue>,
    vars: Vec<Variable>,
    scripts: ImportedScripts,
    notes: Vec<ImportNote>,
}

/// Move the root's settings and notes out of `imported`, leaving it bare.
fn take_root(imported: &mut ImportedCollection) -> RootSettings {
    RootSettings {
        docs: std::mem::take(&mut imported.docs),
        auth: std::mem::replace(&mut imported.auth, AuthSpec::Inherit),
        headers: std::mem::take(&mut imported.headers),
        vars: std::mem::take(&mut imported.vars),
        scripts: std::mem::take(&mut imported.scripts),
        notes: std::mem::take(&mut imported.notes),
    }
}

/// The file that holds `folder`'s settings: its `folder.yml`, or the root file for the root.
fn settings_path(collection: &Collection, folder: &ItemPath) -> PathBuf {
    let dir = collection.root().join(folder.as_str());
    dir.join(if folder.is_root() {
        ROOT_FILE
    } else {
        FOLDER_FILE
    })
}

/// Write `root` onto `folder`'s settings file, each setting present replacing the file's own,
/// and return that file relative to the collection root.
fn write_root(
    collection: &mut Collection,
    folder: &ItemPath,
    root: &RootSettings,
) -> Result<String, ImportError> {
    // Folder and root files share their setters but no trait, so one body serves both.
    macro_rules! apply {
        ($file:expr) => {{
            let file = $file;
            if !root.docs.is_empty() {
                file.set_docs(root.docs.as_str());
            }
            if !matches!(root.auth, AuthSpec::Inherit | AuthSpec::None) {
                file.set_auth(root.auth.clone());
            }
            if !root.headers.is_empty() {
                file.set_headers(root.headers.clone())?;
            }
            if !root.vars.is_empty() {
                file.set_vars(root.vars.clone());
            }
            root.scripts
                .write_with(|phase, code| file.set_script(phase, code));
            file.save()?;
        }};
    }
    if folder.is_root() {
        apply!(collection.file_mut());
        return Ok(ROOT_FILE.to_owned());
    }
    let file = collection
        .folder_file_mut(folder)
        .ok_or_else(|| TreeError::NotFound {
            item: folder.clone(),
        })?;
    apply!(file);
    Ok(format!("{}/{FOLDER_FILE}", folder.as_str()))
}

/// Copy a staged collection's items into `dest`: not its root file, its environments, its
/// hidden files or a root `folder.yml`, which `dest` already has. Each top-level path made is
/// pushed on `copied` so a failure can undo it.
fn copy_items(staged: &Path, dest: &Path, copied: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        let name = entry.file_name();
        let lossy = name.to_string_lossy();
        let own = matches!(
            &*lossy,
            "opencollection.yml" | "environments" | "folder.yml"
        );
        if own || lossy.starts_with('.') {
            continue;
        }
        // A top-level name that exists is refused, so nothing already there is overwritten
        // and the undo list holds only paths this call made.
        let to = dest.join(&name);
        if fs::symlink_metadata(&to).is_ok() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        copied.push(to.clone());
        copy_tree(&entry.path(), &to)?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}
