//! Writing an imported collection: everything is built in a staging directory beside the
//! target and renamed into place, so a failure leaves nothing behind; secrets are stored only
//! once the collection exists at its final root.

use super::environment::{store_secrets, write_environment_file};
use super::update::{SpecBaseline, file_field_hashes, operation_key};
use super::{
    ImportError, ImportNote, ImportReport, ImportedCollection, ImportedEnvironment, ImportedItem,
    NoteKind,
};
use crate::collection::{Collection, ENVIRONMENTS_DIR, FOLDER_FILE, ROOT_FILE};
use crate::error::{OpenError, TreeError};
use crate::item_path::ItemPath;
use crate::naming::sanitize_name;
use crate::secrets::SecretStore;
use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

/// Write `imported` and `envs` as a new collection directory in `parent_dir`, named after the
/// collection as Bruno's import names it, and store the environments' secret values in
/// `secrets`. Returns the opened collection and a report whose notes name the files written.
///
/// Fails with `Create` when the collection's own name cannot be a directory name, `Exists` when
/// the target appears while writing, and otherwise with the failing step's error; a failure
/// leaves nothing in `parent_dir` and stores no secret.
pub fn write_collection(
    parent_dir: &Path,
    imported: &ImportedCollection,
    envs: &[ImportedEnvironment],
    secrets: &dyn SecretStore,
) -> Result<(Collection, ImportReport), ImportError> {
    write_collection_with(parent_dir, imported, envs, secrets, &mut |_| Ok(()))
}

/// The same, with a hook called on the staged root just before the rename (tests inject faults).
pub(crate) fn write_collection_with(
    parent_dir: &Path,
    imported: &ImportedCollection,
    envs: &[ImportedEnvironment],
    secrets: &dyn SecretStore,
    before_rename: &mut dyn FnMut(&Path) -> io::Result<()>,
) -> Result<(Collection, ImportReport), ImportError> {
    // A name that sanitises to nothing would make `unique_dir_name` probe `parent_dir` itself
    // and pick `" - 1"`; keeping it lets `Collection::create` refuse it like any invalid name.
    let name = if sanitize_name(&imported.name).is_empty() {
        imported.name.clone()
    } else {
        Collection::unique_dir_name(parent_dir, &imported.name)
    };
    let target = parent_dir.join(sanitize_name(&name));
    let staging = tempfile::Builder::new()
        .prefix(".apinata-import-")
        .tempdir_in(parent_dir)
        .map_err(|source| io_error(parent_dir, source))?;
    let staged = staging.path().join("collection");

    let mut collection = Collection::create(&staged, &name)?;
    let mut report = ImportReport::default();
    let root_file = collection.file_mut();
    root_file.set_headers(imported.headers.clone())?;
    root_file.set_vars(imported.vars.clone());
    root_file.set_docs(imported.docs.as_str());
    root_file.set_auth(imported.auth.clone());
    imported
        .scripts
        .write_with(|phase, code| root_file.set_script(phase, code));
    root_file.save()?;
    report
        .notes
        .extend(placed(&imported.notes, ROOT_FILE.to_owned()));
    let environments = staged.join(ENVIRONMENTS_DIR);
    fs::create_dir_all(&environments).map_err(|source| io_error(&environments, source))?;
    write_items(
        &mut collection,
        &ItemPath::root(),
        "",
        &imported.items,
        &mut report,
    )?;
    report.notes.extend(imported.unwritten.iter().cloned());
    if imported.spec_source {
        record_spec_baseline(&mut collection, &ItemPath::root(), &report.items)?;
    }
    let written = envs
        .iter()
        .map(|env| write_environment_file(&mut collection, env))
        .collect::<Result<Vec<_>, _>>()?;
    drop(collection);

    before_rename(&staged).map_err(|source| io_error(&staged, source))?;
    // `rename(2)` silently replaces an empty directory on Unix, so a target that appeared since
    // the name was chosen is refused before the rename rather than by it.
    if target.exists() {
        return Err(ImportError::Exists { path: target });
    }
    fs::rename(&staged, &target).map_err(|source| match source.kind() {
        ErrorKind::AlreadyExists | ErrorKind::DirectoryNotEmpty => ImportError::Exists {
            path: target.clone(),
        },
        _ => io_error(&target, source),
    })?;
    drop(staging);

    // The tree at `target` is entirely ours and no secret is stored yet, so a failed reopen
    // removes it to keep the promise that a failure leaves nothing behind.
    let collection = Collection::open(&target).map_err(|e| {
        let _ = fs::remove_dir_all(&target);
        match e {
            OpenError::Io { path, source } => ImportError::Io { path, source },
            other => io_error(&target, io::Error::other(other)),
        }
    })?;
    for env in &written {
        report.notes.extend(env.notes.iter().cloned());
        report.environments.push(env.name.clone());
        store_secrets(&collection, env, secrets, &mut report);
    }
    Ok((collection, report))
}

/// Remember what each written request hashed to, so a later update can tell an untouched
/// field from an edited one, under the entry of `folder` (the root folder is the root baseline).
/// A repeated operation key keeps its first request.
pub fn record_spec_baseline(
    collection: &mut Collection,
    folder: &ItemPath,
    items: &[ItemPath],
) -> Result<(), ImportError> {
    let mut baseline = SpecBaseline::default();
    for (path, file) in items
        .iter()
        .filter_map(|p| collection.request(p).map(|f| (p, f)))
    {
        let key = operation_key(file.method(), file.url());
        if baseline.get(&key).is_none() {
            baseline.insert(key.clone(), file_field_hashes(file));
            baseline.set_file(key, path.as_str().to_owned());
        }
    }
    let root_file = collection.file_mut();
    root_file.set_spec_baseline_in(folder, &baseline);
    root_file.save()?;
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> ImportError {
    ImportError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// `notes`, each tied to `file`.
pub(super) fn placed(notes: &[ImportNote], file: String) -> impl Iterator<Item = ImportNote> + '_ {
    notes.iter().cloned().map(move |mut note| {
        note.file = Some(file.clone());
        note
    })
}

/// Write `items` into the folder `parent`, depth first and in order, whose breadcrumb is
/// `crumb`. Each kind keeps its own set of lower-cased stems, so names that differ only by
/// case never share a file on a case-insensitive file system.
fn write_items(
    collection: &mut Collection,
    parent: &ItemPath,
    crumb: &str,
    items: &[ImportedItem],
    report: &mut ImportReport,
) -> Result<(), ImportError> {
    let mut folder_stems = HashSet::new();
    let mut request_stems = HashSet::new();
    for item in items {
        let (name, seq, kind) = match item {
            ImportedItem::Folder(f) => (f.name.as_str(), f.seq, None),
            ImportedItem::Request(r) => (r.name.as_str(), r.seq, Some(r.request.kind)),
        };
        let is_folder = kind.is_none();
        let item_crumb = if crumb.is_empty() {
            name.to_owned()
        } else {
            format!("{crumb} / {name}")
        };
        let (taken, fallback) = if is_folder {
            (&mut folder_stems, format!("Folder {seq}"))
        } else {
            (&mut request_stems, format!("Request {seq}"))
        };
        let path = place(name, &fallback, taken, |candidate| match kind {
            None => collection.create_folder(parent, candidate),
            Some(kind) => collection.create_request_of_kind(parent, candidate, kind),
        })?;
        let stem = path
            .file_name()
            .strip_suffix(".yml")
            .filter(|_| !is_folder)
            .unwrap_or(path.file_name())
            .to_owned();
        taken.insert(stem.to_lowercase());
        let renamed = stem != sanitize_name(name);
        report.items.push(path.clone());

        match item {
            ImportedItem::Folder(folder) => {
                let file = format!("{}/{FOLDER_FILE}", path.as_str());
                let folder_file = collection
                    .folder_file_mut(&path)
                    .ok_or_else(|| TreeError::NotFound { item: path.clone() })?;
                folder_file.set_seq(seq);
                folder_file.set_docs(folder.docs.as_str());
                folder_file.set_auth(folder.auth.clone());
                folder_file.set_headers(folder.headers.clone())?;
                folder_file.set_vars(folder.vars.clone());
                folder
                    .scripts
                    .write_with(|phase, code| folder_file.set_script(phase, code));
                if renamed {
                    folder_file.set_name(name);
                }
                folder_file.save()?;
                if renamed {
                    report.notes.push(renamed_note(&item_crumb, name, &file));
                }
                report.notes.extend(placed(&folder.notes, file));
                write_items(collection, &path, &item_crumb, &folder.items, report)?;
            }
            ImportedItem::Request(request) => {
                let file = path.as_str().to_owned();
                let request_file = collection
                    .request_mut(&path)
                    .ok_or_else(|| TreeError::NotFound { item: path.clone() })?;
                request_file.set_seq(seq);
                request.request.apply_to(request_file)?;
                if renamed {
                    request_file.set_name(name);
                }
                request_file.save()?;
                if renamed {
                    report.notes.push(renamed_note(&item_crumb, name, &file));
                }
                report.notes.extend(placed(&request.notes, file));
            }
        }
    }
    Ok(())
}

/// Create an item under the first usable name: `name`, `name 1`, `name 2`..., skipping stems in
/// `taken`; a name the tree refuses falls back to `fallback` and its suffixes. `name` itself
/// being refused still tries `name 1` (which rescues reserved names such as `CON`), but a
/// refused suffixed name abandons `name`.
fn place(
    name: &str,
    fallback: &str,
    taken: &HashSet<String>,
    mut create: impl FnMut(&str) -> Result<ItemPath, TreeError>,
) -> Result<ItemPath, ImportError> {
    for (base_index, base) in [name, fallback].into_iter().enumerate() {
        for n in 0u64.. {
            let candidate = if n == 0 {
                base.to_owned()
            } else {
                format!("{base} {n}")
            };
            if taken.contains(&sanitize_name(&candidate).to_lowercase()) {
                continue;
            }
            match create(&candidate) {
                Ok(path) => return Ok(path),
                Err(TreeError::InvalidName { .. }) if base_index == 0 && n == 0 => {}
                Err(TreeError::InvalidName { .. }) if base_index == 0 => break,
                Err(error) => return Err(error.into()),
            }
        }
    }
    unreachable!("the fallback either succeeds or returns its error")
}

fn renamed_note(crumb: &str, name: &str, file: &str) -> ImportNote {
    let mut note = ImportNote::new(
        crumb,
        NoteKind::Renamed,
        format!(
            "the name `{name}` cannot be used as a file name here, so it was written to `{file}`; \
             the display name is kept"
        ),
    );
    note.file = Some(file.to_owned());
    note
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{parse_postman, parse_postman_environment};
    use crate::secrets::{SecretError, SecretKey};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts writes so a test can prove the keychain was never touched.
    #[derive(Default)]
    struct Counting(AtomicUsize);

    impl SecretStore for Counting {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Ok(None)
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            Ok(())
        }
    }

    fn inputs() -> (ImportedCollection, Vec<ImportedEnvironment>) {
        let json = br#"{"info": {"name": "c", "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
            "item": [{"name": "f", "item": [{"name": "r", "request": {"method": "GET", "url": "u"}}]}]}"#;
        let env = br#"{"name": "dev", "values": [{"key": "s", "value": "v", "type": "secret"}]}"#;
        (
            parse_postman(json).expect("parses"),
            vec![parse_postman_environment(env).expect("parses")],
        )
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Files under `dir`, recursively.
    fn walk(dir: &Path) -> usize {
        fs::read_dir(dir)
            .expect("read dir")
            .map(|e| {
                let path = e.expect("entry").path();
                if path.is_dir() { walk(&path) } else { 1 }
            })
            .sum()
    }

    #[test]
    fn a_failure_before_the_rename_leaves_nothing_and_touches_no_secret() {
        let parent = tempfile::TempDir::new().expect("tempdir");
        let (imported, envs) = inputs();
        let store = Counting::default();
        let mut staged_files = 0;
        let result =
            write_collection_with(parent.path(), &imported, &envs, &store, &mut |staged| {
                staged_files = walk(staged); // root, folder.yml, request, environment
                Err(std::io::Error::other("injected"))
            });
        assert!(matches!(result, Err(ImportError::Io { .. })), "{result:?}");
        assert_eq!(staged_files, 4);
        assert!(
            entries(parent.path()).is_empty(),
            "staging removed: {:?}",
            entries(parent.path())
        );
        assert_eq!(store.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_name_taken_before_the_rename_is_exists_and_keeps_the_intruder() {
        let parent = tempfile::TempDir::new().expect("tempdir");
        let (imported, envs) = inputs();
        let store = Counting::default();
        let intruder = parent.path().join("c");
        let result = write_collection_with(parent.path(), &imported, &envs, &store, &mut |_| {
            fs::create_dir(&intruder)?;
            fs::write(intruder.join("mine.txt"), "x")
        });
        assert!(
            matches!(result, Err(ImportError::Exists { .. })),
            "{result:?}"
        );
        assert_eq!(entries(parent.path()), ["c"]);
        assert_eq!(entries(&intruder), ["mine.txt"]);
        assert_eq!(store.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_collection_that_cannot_be_reopened_is_removed_and_touches_no_secret() {
        let parent = tempfile::TempDir::new().expect("tempdir");
        let (imported, envs) = inputs();
        let store = Counting::default();
        let result =
            write_collection_with(parent.path(), &imported, &envs, &store, &mut |staged| {
                fs::write(staged.join(ROOT_FILE), "[not a mapping")
            });
        assert!(matches!(result, Err(ImportError::Io { .. })), "{result:?}");
        assert!(
            entries(parent.path()).is_empty(),
            "target removed: {:?}",
            entries(parent.path())
        );
        assert_eq!(store.0.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn secrets_are_written_only_after_the_rename() {
        let parent = tempfile::TempDir::new().expect("tempdir");
        let (imported, envs) = inputs();
        let store = Counting::default();
        let (collection, _) =
            write_collection_with(parent.path(), &imported, &envs, &store, &mut |_| {
                assert_eq!(
                    store.0.load(Ordering::SeqCst),
                    0,
                    "no secret before the rename"
                );
                Ok(())
            })
            .expect("writes");
        assert_eq!(store.0.load(Ordering::SeqCst), 1);
        assert_eq!(collection.root(), parent.path().join("c"));
    }
}
