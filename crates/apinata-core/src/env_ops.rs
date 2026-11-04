//! Environment create, rename, delete and the secret flag, as Bruno's Electron handlers do
//! them. File operations run first and a failed secret move or removal after them is an
//! [`EnvWarning`], never a rollback; only the secret flag touches the store first, where the
//! file change would otherwise leave the store holding the only copy of a value.

use crate::collection::{Collection, Environment, name_order};
use crate::document::Document;
use crate::edit::VarValue;
use crate::env_file::EnvVariable;
use crate::env_file::TOP as ENV_TOP;
use crate::error::{EnvError, SaveError};
use crate::naming::{sanitize_name, validate_name};
use crate::secrets::{EnvWarning, SecretKey, SecretStore};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use yaml_serde::{Mapping, Value};

const ENVIRONMENTS_DIR: &str = "environments";

impl Collection {
    /// Create an empty environment. The name is sanitised and, if taken, suffixed as Bruno's
    /// `generateUniqueName` does (`dev copy`, `dev copy 2`, ...); returns the name used.
    /// No secret is written, so the warnings are empty today.
    pub fn create_environment(
        &mut self,
        name: &str,
        store: &dyn SecretStore,
    ) -> Result<(String, Vec<EnvWarning>), EnvError> {
        create_environment(self, name, store)
    }

    /// Copy environment `from` (variables, colour, `extends`) to a new file named as
    /// [`Collection::create_environment`] names one, and copy its stored secret values to the
    /// new keys. A failed secret copy is a warning; the copy stands.
    pub fn duplicate_environment(
        &mut self,
        from: &str,
        store: &dyn SecretStore,
    ) -> Result<(String, Vec<EnvWarning>), EnvError> {
        duplicate_environment(self, from, store)
    }

    /// The names marked secret in environment `env`'s file as it is on disk, ignoring any draft;
    /// empty for an unknown environment or a missing or unparseable file.
    pub fn saved_secret_names(&self, env: &str) -> Vec<String> {
        self.try_saved_secret_names(env).unwrap_or_default()
    }

    /// Like [`Collection::saved_secret_names`], but `None` for an unknown environment or a
    /// missing or unparseable file, so a failed read is not mistaken for "no secrets".
    pub fn try_saved_secret_names(&self, env: &str) -> Option<Vec<String>> {
        // Re-read the file so a draft, reload or revert can never make the answer stale.
        let path = self.environment(env)?.path();
        let doc = Document::load(path).ok()?;
        Some(secret_names(&Environment::from_parts(env.to_owned(), doc)))
    }

    /// The variables of environment `env`'s file as it is on disk, ignoring any draft; `None`
    /// for an unknown environment or a missing or unparseable file.
    pub fn try_saved_variables(&self, env: &str) -> Option<Vec<EnvVariable>> {
        let path = self.environment(env)?.path();
        let doc = Document::load(path).ok()?;
        Some(Environment::from_parts(env.to_owned(), doc).variables())
    }

    /// The orphans of `env`, sorted, from a fresh read of its file and the store: names with a
    /// stored value that the file does not mark `secret: true`.
    pub fn orphaned_secrets(
        &self,
        env: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<String>, EnvError> {
        orphaned_secrets(self, env, store)
    }

    /// Delete the named orphans of `env`. Refused, deleting nothing, unless every name is an
    /// orphan at the time of the call. A failed delete is a warning and the rest still go.
    pub fn remove_orphaned_secrets(
        &self,
        env: &str,
        names: &[String],
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        remove_orphaned_secrets(self, env, names, store)
    }

    /// Every environment's orphans plus every secret with no file, from fresh reads of the
    /// directory, the store and each file with stored keys. Never reads a value.
    pub fn all_orphaned_secrets(&self, store: &dyn SecretStore) -> Result<OrphanReport, EnvError> {
        all_orphaned_secrets(self, store)
    }

    /// The variables stored under the raw name `env`, sorted. [`EnvError::HasFile`] when a file
    /// exists for it, loadable or not. The name is compared, never joined into a path.
    pub fn fileless_secrets(
        &self,
        env: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<String>, EnvError> {
        fileless_secrets(self, env, store)
    }

    /// Delete the named secrets of the raw name `env`. Refused, deleting nothing, unless every
    /// name is stored now ([`EnvError::NotOrphaned`]) and `env` has no file
    /// ([`EnvError::HasFile`]). A failed delete is a warning and the rest still go.
    pub fn remove_fileless_secrets(
        &self,
        env: &str,
        names: &[String],
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        remove_fileless_secrets(self, env, names, store)
    }

    /// Rename environment `from` to `to`: save it with the new `name`, rename its file, move
    /// its secret values to the new keys, then point every environment that extends `from` at
    /// `to`. Failed secret moves and sibling saves are warnings; the rename stands.
    pub fn rename_environment(
        &mut self,
        from: &str,
        to: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        rename_environment(self, from, to, store)
    }

    /// The other environments whose `extends` names `name`, in name order; empty for an
    /// unknown name. An environment extending itself is not counted.
    pub fn extended_by(&self, name: &str) -> Vec<String> {
        extended_by(self, name)
    }

    /// Delete environment `name`'s file, then its secret values. Refused with
    /// [`EnvError::ExtendedBy`] while another environment extends it.
    pub fn delete_environment(
        &mut self,
        name: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        delete_environment(self, name, store)
    }

    /// Mark variable `var` of environment `env` secret or plain and save the file. Making it
    /// secret stores the value, unless it is empty and a value is already stored, then writes
    /// the entry without it. Making it plain reads the stored value (empty when there is none),
    /// writes it to the file, then removes it from the store. A failed store read or write
    /// before the file change is [`EnvError::SecretStore`] and changes nothing on disk; a failed
    /// removal after it is a warning. No change does nothing.
    pub fn set_variable_secret(
        &mut self,
        env: &str,
        var: &str,
        secret: bool,
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        set_variable_secret(self, env, var, secret, store)
    }
}

impl Collection {
    /// The [`EnvError::NotFound`] for `name`, listing the environments this collection has.
    pub fn environment_not_found(&self, name: &str) -> EnvError {
        not_found(self, name)
    }

    /// Set plain variable `var` of environment `env` to `value` and save, adding it when absent.
    /// A typed variable keeps its type. Returns whether anything changed; an unchanged value
    /// writes nothing. A secret variable is [`EnvError::SecretVariable`].
    pub fn set_variable_text(
        &mut self,
        env: &str,
        var: &str,
        value: &str,
    ) -> Result<bool, EnvError> {
        let Some(environment) = self.environment_mut(env) else {
            return Err(not_found(self, env));
        };
        let vars = environment.variables();
        let Some(vars) = with_text(vars, var, value) else {
            return Err(EnvError::SecretVariable {
                environment: env.to_owned(),
                variable: var.to_owned(),
            });
        };
        let (vars, changed) = vars;
        if changed {
            let before = environment.variables();
            environment.set_variables(vars);
            if let Err(e) = environment.save() {
                environment.set_variables(before);
                return Err(e.into());
            }
        }
        Ok(changed)
    }

    /// Remove variable `var` from environment `env` and save. A secret is refused with
    /// [`EnvError::SecretVariable`] unless `with_secret`, which also deletes its stored value;
    /// a failed deletion after the save is a warning.
    pub fn remove_variable(
        &mut self,
        env: &str,
        var: &str,
        with_secret: bool,
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        let key = self.secret_key(env, var);
        let Some(environment) = self.environment_mut(env) else {
            return Err(not_found(self, env));
        };
        let before = environment.variables();
        let was_secret = before.iter().any(|v| v.name == var && v.secret);
        let vars = match without_variable(before.clone(), var, with_secret) {
            Ok(vars) => vars,
            Err(VariableRefusal::Missing) => {
                return Err(EnvError::VariableNotFound {
                    environment: env.to_owned(),
                    variable: var.to_owned(),
                });
            }
            Err(VariableRefusal::Secret) => {
                return Err(EnvError::SecretVariable {
                    environment: env.to_owned(),
                    variable: var.to_owned(),
                });
            }
        };
        environment.set_variables(vars);
        if let Err(e) = environment.save() {
            environment.set_variables(before);
            return Err(e.into());
        }
        let mut warnings = Vec::new();
        if was_secret && let Err(error) = store.delete(&key) {
            warnings.push(EnvWarning::Secret {
                environment: env.to_owned(),
                variable: var.to_owned(),
                error,
            });
        }
        Ok(warnings)
    }

    /// Store `value` as the secret value of `var` in environment `env`, declaring the variable
    /// secret in the file (with no value) when it is plain or absent. The store is written
    /// first, so a refusal is [`EnvError::SecretStore`] and changes nothing on disk; a failed
    /// save puts the store back.
    pub fn set_secret_value(
        &mut self,
        env: &str,
        var: &str,
        value: &str,
        store: &dyn SecretStore,
    ) -> Result<(), EnvError> {
        let key = self.secret_key(env, var);
        let Some(environment) = self.environment_mut(env) else {
            return Err(not_found(self, env));
        };
        let refused = |error| EnvError::SecretStore {
            environment: env.to_owned(),
            variable: var.to_owned(),
            error,
        };
        let before = environment.variables();
        let previous = store.get(&key).map_err(refused)?;
        store.set(&key, value).map_err(refused)?;
        let mut vars = before.clone();
        match vars.iter_mut().find(|v| v.name == var) {
            Some(v) if v.secret => {}
            Some(v) => {
                v.secret = true;
                v.value = VarValue::Text(String::new());
            }
            None => vars.push(EnvVariable::secret(var)),
        }
        if vars.iter().zip(&before).all(|(a, b)| a.secret == b.secret) && vars.len() == before.len()
        {
            return Ok(());
        }
        environment.set_variables(vars);
        if let Err(e) = environment.save() {
            environment.set_variables(before);
            // The save error is the one reported, so a failed restore is ignored.
            let _ = match previous {
                Some(old) => store.set(&key, &old),
                None => store.delete(&key),
            };
            return Err(e.into());
        }
        Ok(())
    }
}

/// Why [`without_variable`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VariableRefusal {
    /// No variable has this name.
    Missing,
    /// An entry of this name is secret.
    Secret,
}

/// `vars` without every entry named `name`. Every entry is checked, since a removal drops them
/// all: a plain duplicate must not carry a secret of the same name out unless `with_secret`.
pub fn without_variable(
    mut vars: Vec<EnvVariable>,
    name: &str,
    with_secret: bool,
) -> Result<Vec<EnvVariable>, VariableRefusal> {
    if !vars.iter().any(|v| v.name == name) {
        return Err(VariableRefusal::Missing);
    }
    if !with_secret && vars.iter().any(|v| v.name == name && v.secret) {
        return Err(VariableRefusal::Secret);
    }
    vars.retain(|v| v.name != name);
    Ok(vars)
}

/// `vars` with plain variable `name` set to `value` (added when absent, type kept), and whether
/// that changed anything; `None` when an entry of the name is secret.
fn with_text(
    mut vars: Vec<EnvVariable>,
    name: &str,
    value: &str,
) -> Option<(Vec<EnvVariable>, bool)> {
    if vars.iter().any(|v| v.name == name && v.secret) {
        return None;
    }
    let Some(var) = vars.iter_mut().find(|v| v.name == name) else {
        vars.push(EnvVariable::new(name, VarValue::Text(value.to_owned())));
        return Some((vars, true));
    };
    let new = match &var.value {
        VarValue::Typed { kind, .. } => VarValue::Typed {
            kind: kind.clone(),
            data: value.to_owned(),
        },
        _ => VarValue::Text(value.to_owned()),
    };
    let changed = var.value != new;
    var.value = new;
    Some((vars, changed))
}

impl EnvHost for Collection {
    fn env_dir(&self) -> PathBuf {
        self.root().join(ENVIRONMENTS_DIR)
    }

    fn envs(&self) -> &[Environment] {
        self.environments()
    }

    fn envs_mut(&mut self) -> &mut Vec<Environment> {
        self.environments_mut()
    }

    fn env_secret_key(&self, env: &str, var: &str) -> SecretKey {
        self.secret_key(env, var)
    }
}

/// The owner of a directory of environment files: a collection's `environments/` or the
/// global environments. The operations below work on either through it.
pub(crate) trait EnvHost {
    /// The directory holding one `<name>.yml` per environment.
    fn env_dir(&self) -> PathBuf;
    /// The loaded environments, sorted by name.
    fn envs(&self) -> &[Environment];
    /// The loaded environments, for editing.
    fn envs_mut(&mut self) -> &mut Vec<Environment>;
    /// Where the secret value of `var` in environment `env` is stored.
    fn env_secret_key(&self, env: &str, var: &str) -> SecretKey;

    /// Whether this host holds the global environments rather than a collection's.
    fn is_global(&self) -> bool {
        false
    }

    /// The environment whose file stem is `name`.
    fn env(&self, name: &str) -> Option<&Environment> {
        self.envs().iter().find(|e| e.name() == name)
    }

    /// The environment whose file stem is `name`, for editing.
    fn env_mut(&mut self, name: &str) -> Option<&mut Environment> {
        self.envs_mut().iter_mut().find(|e| e.name() == name)
    }
}

/// The unknown-name error, carrying the names `host` does have.
pub(crate) fn not_found<H: EnvHost + ?Sized>(host: &H, name: &str) -> EnvError {
    EnvError::NotFound {
        name: name.to_owned(),
        available: host.envs().iter().map(|e| e.name().to_owned()).collect(),
        global: host.is_global(),
    }
}

/// Checks `name` as [`Collection::create_environment`] does before it touches the disk:
/// `Ok` with the sanitised base name, which a taken name still suffixes, or
/// [`EnvError::InvalidName`] when no file could carry it.
pub fn check_environment_name(name: &str) -> Result<String, EnvError> {
    let base = sanitize_name(name);
    match validate_name(&base) {
        true => Ok(base),
        false => Err(EnvError::InvalidName {
            name: name.to_owned(),
        }),
    }
}

/// The body of [`Collection::create_environment`] for any [`EnvHost`].
pub(crate) fn create_environment<H: EnvHost + ?Sized>(
    host: &mut H,
    name: &str,
    _store: &dyn SecretStore,
) -> Result<(String, Vec<EnvWarning>), EnvError> {
    let base = check_environment_name(name)?;
    let dir = host.env_dir();
    let taken = stems_on_disk(&dir)?;
    let unique = unique_name(&base, |n| taken.iter().any(|t| t == n));
    fs::create_dir_all(&dir).map_err(|source| io(&dir, source))?;
    let mut root = Mapping::new();
    root.insert(Value::from("name"), Value::from(unique.as_str()));
    let mut doc = Document::new(dir.join(format!("{unique}.yml")), root);
    doc.save()?;
    let envs = host.envs_mut();
    envs.push(Environment::from_parts(unique.clone(), doc));
    envs.sort_by(|a, b| name_order(a.name(), b.name()));
    Ok((unique, Vec::new()))
}

/// The body of [`Collection::duplicate_environment`] for any [`EnvHost`].
pub(crate) fn duplicate_environment<H: EnvHost + ?Sized>(
    host: &mut H,
    from: &str,
    store: &dyn SecretStore,
) -> Result<(String, Vec<EnvWarning>), EnvError> {
    let source = host.env(from).ok_or_else(|| not_found(host, from))?;
    let variables = source.variables();
    let (color, extends) = (source.color(), source.extends());
    // Same naming and file creation as a new environment, then fill the fresh file.
    let (unique, mut warnings) = create_environment(host, from, store)?;
    let env = host.env_mut(&unique).expect("just created");
    env.set_variables(variables.clone());
    env.set_color(color);
    env.set_extends(extends);
    env.save()?;
    // A secret with no stored value stays unset in the copy.
    for variable in variables.into_iter().filter(|v| v.secret) {
        let copied = store
            .get(&host.env_secret_key(from, &variable.name))
            .and_then(|value| match value {
                Some(value) => store.set(&host.env_secret_key(&unique, &variable.name), &value),
                None => Ok(()),
            });
        if let Err(error) = copied {
            warnings.push(EnvWarning::Secret {
                environment: unique.clone(),
                variable: variable.name,
                error,
            });
        }
    }
    Ok((unique, warnings))
}

/// The body of [`Collection::rename_environment`] for any [`EnvHost`].
pub(crate) fn rename_environment<H: EnvHost + ?Sized>(
    host: &mut H,
    from: &str,
    to: &str,
    store: &dyn SecretStore,
) -> Result<Vec<EnvWarning>, EnvError> {
    let old_path = host
        .env(from)
        .ok_or_else(|| not_found(host, from))?
        .path()
        .to_path_buf();
    if !validate_name(to) {
        return Err(EnvError::InvalidName {
            name: to.to_owned(),
        });
    }
    if from == to {
        return Ok(Vec::new());
    }
    // A file of that name taken by a broken environment counts too; the same file under
    // another spelling, as on a case-insensitive file system, does not.
    let new_path = host.env_dir().join(format!("{to}.yml"));
    if host.env(to).is_some()
        || (fs::symlink_metadata(&new_path).is_ok() && !same_file(&old_path, &new_path))
    {
        return Err(EnvError::Exists {
            name: to.to_owned(),
        });
    }

    // Saving at the old path first lets a conflict stop the rename before anything moves;
    // a failed save puts the draft's name back, so a later save cannot write `to` there.
    let env = host.env_mut(from).expect("found above");
    let draft_name = env.document().get(&["name"]).cloned();
    env.document_mut()
        .set(&["name"], Value::from(to), &[ENV_TOP]);
    if let Err(error) = env.save() {
        match draft_name {
            Some(name) => env.document_mut().set(&["name"], name, &[ENV_TOP]),
            None => {
                env.document_mut().remove(&["name"]);
            }
        }
        return Err(error.into());
    }
    // A failed move would leave the file naming an environment that does not exist, so
    // the old name is written back before the error is returned.
    if let Err(source) = fs::rename(&old_path, &new_path) {
        let rename = io(&old_path, source);
        return Err(restore_name_after_failed_rename(
            env, from, old_path, rename,
        ));
    }
    env.renamed(to.to_owned(), new_path);
    let secrets = secret_names(env);
    host.envs_mut()
        .sort_by(|a, b| name_order(a.name(), b.name()));

    let mut warnings = Vec::new();
    for variable in secrets {
        let old = host.env_secret_key(from, &variable);
        let moved = store.get(&old).and_then(|value| match value {
            Some(value) => store
                .set(&host.env_secret_key(to, &variable), &value)
                .and_then(|()| store.delete(&old)),
            None => Ok(()),
        });
        if let Err(error) = moved {
            warnings.push(EnvWarning::Secret {
                environment: to.to_owned(),
                variable,
                error,
            });
        }
    }
    // Bruno also rewrites each such file's `name` to its file stem.
    for env in host.envs_mut() {
        if env.extends().as_deref() != Some(from) {
            continue;
        }
        let stem = env.name().to_owned();
        env.set_extends(Some(to.to_owned()));
        env.document_mut()
            .set(&["name"], Value::from(stem.as_str()), &[ENV_TOP]);
        if let Err(e) = env.save() {
            warnings.push(EnvWarning::Extends {
                environment: stem,
                message: e.to_string(),
            });
        }
    }
    Ok(warnings)
}

/// The body of [`Collection::extended_by`] for any [`EnvHost`].
pub(crate) fn extended_by<H: EnvHost + ?Sized>(host: &H, name: &str) -> Vec<String> {
    host.envs()
        .iter()
        .filter(|e| e.name() != name && e.extends().as_deref() == Some(name))
        .map(|e| e.name().to_owned())
        .collect()
}

/// The body of [`Collection::delete_environment`] for any [`EnvHost`].
pub(crate) fn delete_environment<H: EnvHost + ?Sized>(
    host: &mut H,
    name: &str,
    store: &dyn SecretStore,
) -> Result<Vec<EnvWarning>, EnvError> {
    let env = host.env(name).ok_or_else(|| not_found(host, name))?;
    // A delete never leaves a child extending nothing; the user re-points it first.
    let by = extended_by(host, name);
    if !by.is_empty() {
        return Err(EnvError::ExtendedBy {
            name: name.to_owned(),
            by,
            global: host.is_global(),
        });
    }
    let path = env.path().to_path_buf();
    let secrets = secret_names(env);
    fs::remove_file(&path).map_err(|source| io(&path, source))?;
    host.envs_mut().retain(|e| e.name() != name);
    let mut warnings = Vec::new();
    for variable in secrets {
        if let Err(error) = store.delete(&host.env_secret_key(name, &variable)) {
            warnings.push(EnvWarning::Secret {
                environment: name.to_owned(),
                variable,
                error,
            });
        }
    }
    Ok(warnings)
}

/// Write the old name back after the file move failed; reports the move error, or
/// `RenameNotRestored` when that write fails too.
pub(crate) fn restore_name_after_failed_rename(
    env: &mut Environment,
    from: &str,
    path: PathBuf,
    rename: SaveError,
) -> EnvError {
    env.document_mut()
        .set(&["name"], Value::from(from), &[ENV_TOP]);
    match env.save() {
        Ok(()) => rename.into(),
        Err(rollback) => EnvError::RenameNotRestored {
            path,
            rename,
            rollback,
        },
    }
}

/// The body of [`Collection::set_variable_secret`] for any [`EnvHost`].
pub(crate) fn set_variable_secret<H: EnvHost + ?Sized>(
    host: &mut H,
    env: &str,
    var: &str,
    secret: bool,
    store: &dyn SecretStore,
) -> Result<Vec<EnvWarning>, EnvError> {
    let key = host.env_secret_key(env, var);
    let Some(environment) = host.env_mut(env) else {
        return Err(not_found(host, env));
    };
    let mut vars = environment.variables();
    let before = vars.clone();
    let index =
        vars.iter()
            .position(|v| v.name == var)
            .ok_or_else(|| EnvError::VariableNotFound {
                environment: env.to_owned(),
                variable: var.to_owned(),
            })?;
    if vars[index].secret == secret {
        return Ok(Vec::new());
    }
    let refused = |error| EnvError::SecretStore {
        environment: env.to_owned(),
        variable: var.to_owned(),
        error,
    };
    let entry = &mut vars[index];
    if secret {
        // Store first: once the file drops the value, the store holds the only copy.
        let value = match &entry.value {
            VarValue::Text(text) | VarValue::Typed { data: text, .. } => text,
        };
        let previous = store.get(&key).map_err(refused)?;
        // An empty plain value says nothing about the secret, so a value already stored stays.
        if !value.is_empty() || previous.is_none() {
            store.set(&key, value).map_err(refused)?;
        }
        entry.secret = true;
        environment.set_variables(vars);
        // On a failed save the file still holds the value: the draft goes back and the store
        // returns to what it held before this call. A failed restore is ignored on purpose:
        // the save error is the one reported.
        if let Err(e) = environment.save() {
            environment.set_variables(before);
            let _ = match previous {
                Some(old) => store.set(&key, &old),
                None => store.delete(&key),
            };
            return Err(e.into());
        }
        return Ok(Vec::new());
    }

    let value = store.get(&key).map_err(refused)?.unwrap_or_default();
    entry.secret = false;
    entry.value = match &entry.value {
        VarValue::Typed { kind, .. } => VarValue::Typed {
            kind: kind.clone(),
            data: value,
        },
        VarValue::Text(_) => VarValue::Text(value),
    };
    environment.set_variables(vars);
    if let Err(e) = environment.save() {
        environment.set_variables(before);
        return Err(e.into());
    }
    let mut warnings = Vec::new();
    if let Err(error) = store.delete(&key) {
        warnings.push(EnvWarning::Secret {
            environment: env.to_owned(),
            variable: var.to_owned(),
            error,
        });
    }
    Ok(warnings)
}

/// Port of `generateUniqueName` (bruno-electron `utils/filesystem.js`).
fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    let mut candidate = format!("{base} copy");
    let mut counter = 1;
    while taken(&candidate) {
        counter += 1;
        candidate = format!("{base} copy {counter}");
    }
    candidate
}

/// The stems of every `.yml` file in `dir`, loadable or not, as Bruno lists them.
fn stems_on_disk(dir: &Path) -> Result<Vec<String>, EnvError> {
    Ok(stems_if_readable(dir)?.unwrap_or_default())
}

/// As [`stems_on_disk`], with `None` when `dir` reads as not found.
fn stems_if_readable(dir: &Path) -> Result<Option<Vec<String>>, EnvError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io(dir, source).into()),
    };
    stems_from(entries.map(|e| e.map(|e| e.file_name())), dir).map(Some)
}

/// Check that a `dir` which reads as not found really means zero files. A dangling link may be
/// a drive that is not mounted, and a gone root says nothing about its files, so both are errors.
/// The caller says whether `dir` lives in a collection: a global directory has no root to check,
/// since one never created is the ordinary fresh install.
fn confirm_no_directory(dir: &Path, in_collection: bool) -> Result<(), EnvError> {
    let unknown = |path: &Path| EnvError::FilesUnknown {
        path: path.to_owned(),
    };
    match fs::symlink_metadata(dir) {
        Ok(_) => return Err(unknown(dir)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(io(dir, source).into()),
    }
    let Some(root) = dir.parent().filter(|_| in_collection) else {
        return Ok(());
    };
    match fs::metadata(root) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(unknown(root)),
        Err(source) => Err(io(root, source).into()),
    }
}

/// The `.yml` stems among directory entry names; one unreadable entry fails the listing. A name
/// that is not text gives the lossy stem the loader names that environment by.
fn stems_from(
    names: impl Iterator<Item = std::io::Result<std::ffi::OsString>>,
    dir: &Path,
) -> Result<Vec<String>, EnvError> {
    let mut stems = Vec::new();
    for name in names {
        let name = name.map_err(|source| io(dir, source))?;
        if let Some(stem) = name.to_string_lossy().strip_suffix(".yml") {
            stems.push(stem.to_owned());
        }
    }
    Ok(stems)
}

/// The names `env`'s file marks secret, read fresh. A file that gives no reliable answer is
/// an error, because every stored value would otherwise look like an orphan.
fn declared_secrets<H: EnvHost + ?Sized>(host: &H, env: &str) -> Result<Vec<String>, EnvError> {
    let environment = host.env(env).ok_or_else(|| not_found(host, env))?;
    let unreadable = |message: String| EnvError::Unreadable {
        environment: env.to_owned(),
        message,
    };
    let doc = Document::load(environment.path()).map_err(|e| unreadable(e.to_string()))?;
    // `Environment::variables` reads a bad shape as empty, so the shape is checked here.
    let well_formed = match doc.get(&["variables"]) {
        None | Some(Value::Null) => true,
        Some(Value::Sequence(items)) => items.iter().all(Value::is_mapping),
        Some(_) => false,
    };
    if !well_formed {
        return Err(unreadable(
            "`variables` is not a list of mappings".to_owned(),
        ));
    }
    Ok(secret_names(&Environment::from_parts(env.to_owned(), doc)))
}

/// The body of [`Collection::orphaned_secrets`] for any [`EnvHost`].
pub(crate) fn orphaned_secrets<H: EnvHost + ?Sized>(
    host: &H,
    env: &str,
    store: &dyn SecretStore,
) -> Result<Vec<String>, EnvError> {
    let declared = declared_secrets(host, env)?;
    let root = host.env_secret_key(env, "").collection;
    let stored = store.list(&root).map_err(|error| EnvError::SecretList {
        environment: env.to_owned(),
        error,
    })?;
    let mut orphans: Vec<String> = stored
        .into_iter()
        .filter(|key| key.environment == env && !declared.contains(&key.variable))
        .map(|key| key.variable)
        .collect();
    orphans.sort();
    orphans.dedup();
    Ok(orphans)
}

/// The body of [`Collection::remove_orphaned_secrets`] for any [`EnvHost`].
pub(crate) fn remove_orphaned_secrets<H: EnvHost + ?Sized>(
    host: &H,
    env: &str,
    names: &[String],
    store: &dyn SecretStore,
) -> Result<Vec<EnvWarning>, EnvError> {
    // Deriving the set here gives one rule for a name declared again, already gone or mistyped.
    let orphans = orphaned_secrets(host, env, store)?;
    delete_allowed(host, env, names, &orphans, store)
}

/// Delete `names` of `env` if `allowed` holds every one, else refuse and delete nothing.
fn delete_allowed<H: SecretHome + ?Sized>(
    home: &H,
    env: &str,
    names: &[String],
    allowed: &[String],
    store: &dyn SecretStore,
) -> Result<Vec<EnvWarning>, EnvError> {
    let refused: Vec<String> = names
        .iter()
        .filter(|name| !allowed.contains(name))
        .cloned()
        .collect();
    if !refused.is_empty() {
        return Err(EnvError::NotOrphaned {
            environment: env.to_owned(),
            variables: refused,
        });
    }
    let failed = names.iter().filter_map(|name| {
        let error = store.delete(&home.key(env, name)).err()?;
        Some(EnvWarning::Secret {
            environment: env.to_owned(),
            variable: name.clone(),
            error,
        })
    });
    Ok(failed.collect())
}

/// A stored secret a person may want gone: an orphan, or one whose environment has no file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OrphanedSecret {
    /// The environment name as stored, which is untrusted text.
    pub environment: String,
    /// The variable name as stored, which is untrusted text.
    pub variable: String,
    /// Whether the environment has a file; false means the name lives only in the store.
    pub has_file: bool,
}

/// What a collection-wide look at the store found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct OrphanReport {
    /// Sorted by environment, then variable, each once.
    pub orphans: Vec<OrphanedSecret>,
    /// Sorted environments with stored keys and a file that gives no answer, so nothing is
    /// listed for them.
    pub unreadable: Vec<String>,
}

/// The secrets left under a root that no longer exists. No file can be read there, so every
/// key under it counts as having none.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MovedRoot {
    root: PathBuf,
}

impl MovedRoot {
    /// Name a gone root. The path is taken as text, not canonicalised, apart from collapsing
    /// `//` and dropping trailing slashes and `.`, which stored roots never have. Refused when
    /// relative ([`EnvError::OldRootRelative`]) or unless it is certainly absent
    /// ([`EnvError::OldRootExists`], [`EnvError::OldRootUnknown`]).
    pub fn new(root: impl Into<PathBuf>) -> Result<MovedRoot, EnvError> {
        let root = root.into().components().collect();
        let moved = MovedRoot { root };
        if !moved.root.is_absolute() {
            return Err(EnvError::OldRootRelative { path: moved.root });
        }
        moved.confirm_absent()?;
        Ok(moved)
    }

    /// Every key under the root, none with a file. Refused if the root has come back.
    pub fn all_orphaned_secrets(&self, store: &dyn SecretStore) -> Result<OrphanReport, EnvError> {
        all_orphaned_secrets(self, store)
    }

    /// The variables stored under `env`, sorted. Refused if the root has come back.
    pub fn fileless_secrets(
        &self,
        env: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<String>, EnvError> {
        fileless_secrets(self, env, store)
    }

    /// Delete the named secrets of `env`. Refused, deleting nothing, unless every name is stored
    /// now and the root is still absent. A failed delete is a warning and the rest still go.
    pub fn remove_fileless_secrets(
        &self,
        env: &str,
        names: &[String],
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        remove_fileless_secrets(self, env, names, store)
    }

    /// A link at the root counts as something there, so its target is not followed.
    fn confirm_absent(&self) -> Result<(), EnvError> {
        let path = || self.root.clone();
        match fs::symlink_metadata(&self.root) {
            Ok(_) => Err(EnvError::OldRootExists { path: path() }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(EnvError::OldRootUnknown {
                path: path(),
                message: e.to_string(),
            }),
        }
    }
}

/// A place secrets are stored under, whether or not it still has files.
pub(crate) trait SecretHome {
    /// Where the secret value of `var` in environment `env` is stored.
    fn key(&self, env: &str, var: &str) -> SecretKey;
    /// Every environment with a file now, from a fresh read. An error means unknown, not none.
    fn file_stems(&self) -> Result<Vec<String>, EnvError>;
    /// The names `env`'s file marks secret, for a name in [`SecretHome::file_stems`].
    fn declared(&self, env: &str) -> Result<Vec<String>, EnvError>;
}

impl<H: EnvHost + ?Sized> SecretHome for H {
    fn key(&self, env: &str, var: &str) -> SecretKey {
        self.env_secret_key(env, var)
    }

    fn file_stems(&self) -> Result<Vec<String>, EnvError> {
        let dir = self.env_dir();
        match stems_if_readable(&dir)? {
            Some(stems) => Ok(stems),
            None => confirm_no_directory(&dir, !self.is_global()).map(|()| Vec::new()),
        }
    }

    fn declared(&self, env: &str) -> Result<Vec<String>, EnvError> {
        declared_secrets(self, env)
    }
}

impl SecretHome for MovedRoot {
    fn key(&self, env: &str, var: &str) -> SecretKey {
        SecretKey::new(&self.root, env, var)
    }

    // Every call passes through here, so a root that came back is refused at that call.
    fn file_stems(&self) -> Result<Vec<String>, EnvError> {
        self.confirm_absent()?;
        Ok(Vec::new())
    }

    fn declared(&self, env: &str) -> Result<Vec<String>, EnvError> {
        Err(EnvError::Unreadable {
            environment: env.to_owned(),
            message: "its collection is gone".to_owned(),
        })
    }
}

/// The body of `all_orphaned_secrets` for anything secrets are stored under.
pub(crate) fn all_orphaned_secrets<H: SecretHome + ?Sized>(
    home: &H,
    store: &dyn SecretStore,
) -> Result<OrphanReport, EnvError> {
    let stems = home.file_stems()?;
    let root = home.key("", "").collection;
    let keys = store
        .list(&root)
        .map_err(|error| EnvError::SecretListRoot { root, error })?;
    let mut by_env: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for key in keys {
        let names = by_env.entry(key.environment).or_default();
        names.insert(key.variable);
    }
    let mut report = OrphanReport::default();
    for (environment, stored) in by_env {
        let has_file = stems.contains(&environment);
        // A file that gives no answer may declare any of these, so none of them is offered.
        let declared = match has_file.then(|| home.declared(&environment)).transpose() {
            Ok(declared) => declared.unwrap_or_default(),
            Err(EnvError::NotFound { .. } | EnvError::Unreadable { .. }) => {
                report.unreadable.push(environment);
                continue;
            }
            Err(e) => return Err(e),
        };
        for variable in stored.into_iter().filter(|v| !declared.contains(v)) {
            report.orphans.push(OrphanedSecret {
                environment: environment.clone(),
                variable,
                has_file,
            });
        }
    }
    Ok(report)
}

/// The body of `fileless_secrets` for anything secrets are stored under.
pub(crate) fn fileless_secrets<H: SecretHome + ?Sized>(
    home: &H,
    env: &str,
    store: &dyn SecretStore,
) -> Result<Vec<String>, EnvError> {
    if home.file_stems()?.iter().any(|stem| stem == env) {
        return Err(EnvError::HasFile {
            environment: env.to_owned(),
        });
    }
    let root = home.key(env, "").collection;
    let stored = store.list(&root).map_err(|error| EnvError::SecretList {
        environment: env.to_owned(),
        error,
    })?;
    let mut names: Vec<String> = stored
        .into_iter()
        .filter(|key| key.environment == env)
        .map(|key| key.variable)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// The body of `remove_fileless_secrets` for anything secrets are stored under.
pub(crate) fn remove_fileless_secrets<H: SecretHome + ?Sized>(
    home: &H,
    env: &str,
    names: &[String],
    store: &dyn SecretStore,
) -> Result<Vec<EnvWarning>, EnvError> {
    // Re-deriving here answers for a file that came back as well as for a mistyped name.
    let stored = fileless_secrets(home, env, store)?;
    delete_allowed(home, env, names, &stored, store)
}

fn secret_names(env: &Environment) -> Vec<String> {
    env.variables()
        .into_iter()
        .filter(|v| v.secret)
        .map(|v| v.name)
        .collect()
}

/// Whether two paths name one file, as on a case-insensitive file system.
fn same_file(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

fn io(path: &Path, source: std::io::Error) -> SaveError {
    SaveError::Io {
        path: PathBuf::from(path),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn a_failed_write_back_reports_rename_not_restored() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("dev.yml");
        fs::write(&path, "name: dev\n").expect("write");
        let mut env = Environment::from_parts("dev".into(), Document::load(&path).expect("load"));
        env.document_mut()
            .set(&["name"], Value::from("new"), &[ENV_TOP]);
        // Another edit keeps the draft dirty after the name is restored.
        env.set_color(Some("#ff0000".into()));
        fs::write(&path, "name: someone else\n").expect("clobber");
        let rename = io(&path, std::io::Error::other("busy"));
        let error = restore_name_after_failed_rename(&mut env, "dev", path.clone(), rename);
        assert!(matches!(
            error,
            EnvError::RenameNotRestored {
                path: p,
                rename: SaveError::Io { .. },
                rollback: SaveError::Conflict { .. },
            } if p == path
        ));
    }

    #[test]
    fn a_successful_write_back_reports_the_rename_error() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("dev.yml");
        fs::write(&path, "name: dev\n").expect("write");
        let mut env = Environment::from_parts("dev".into(), Document::load(&path).expect("load"));
        env.document_mut()
            .set(&["name"], Value::from("new"), &[ENV_TOP]);
        let rename = io(&path, std::io::Error::other("busy"));
        let error = restore_name_after_failed_rename(&mut env, "dev", path, rename);
        assert!(matches!(error, EnvError::Save(SaveError::Io { .. })));
    }

    #[test]
    fn stems_keep_yml_names_only() {
        let names = ["a.yml", "notes.txt", "b.yaml"].map(|n| Ok(OsString::from(n)));
        let stems = stems_from(names.into_iter(), Path::new("d")).expect("ok");
        assert_eq!(stems, ["a"]);
    }

    #[test]
    fn one_unreadable_entry_fails_the_listing() {
        let names = vec![
            Ok(OsString::from("a.yml")),
            Err(std::io::Error::other("bad entry")),
            Ok(OsString::from("b.yml")),
        ];
        let error = stems_from(names.into_iter(), Path::new("d")).expect_err("fails");
        assert!(matches!(error, EnvError::Save(SaveError::Io { .. })));
    }
}
