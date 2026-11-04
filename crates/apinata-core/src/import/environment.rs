//! Writing an imported environment: the file first, then its secret values into the store, so
//! the staged collection writer can run the two halves on either side of its rename.

use super::write::placed;
use super::{ImportError, ImportNote, ImportReport, ImportedEnvironment, NoteKind};
use crate::collection::Collection;
use crate::edit::VarValue;
use crate::env_file::EnvVariable;
use crate::error::EnvError;
use crate::secrets::{MemoryStore, SecretStore};

/// What the file half wrote and what the secret half still has to store.
pub(crate) struct WrittenEnvironment {
    /// The name `create_environment` chose.
    pub(crate) name: String,
    /// `environments/<name>.yml`.
    pub(crate) file: String,
    /// The environment's notes, each tied to `file`.
    pub(crate) notes: Vec<ImportNote>,
    /// `(source, variable name, value)` per secret, in order.
    pub(crate) secrets: Vec<(String, String, String)>,
}

/// Write `env` as a new environment of `collection`, named as `create_environment` does, and
/// store its secret values in `secrets`. Returns the name used and a report; a secret without a
/// value, or one the store refuses, is listed in `secrets_to_enter` and the file is kept.
/// Fails when the environment file cannot be created or saved.
pub fn add_environment(
    collection: &mut Collection,
    env: &ImportedEnvironment,
    secrets: &dyn SecretStore,
) -> Result<(String, ImportReport), ImportError> {
    let written = write_environment_file(collection, env)?;
    let mut report = ImportReport {
        notes: written.notes.clone(),
        environments: vec![written.name.clone()],
        ..ImportReport::default()
    };
    store_secrets(collection, &written, secrets, &mut report);
    Ok((written.name, report))
}

/// Create the environment, set its variables (secret values never written) and save. Touches no
/// store.
pub(crate) fn write_environment_file(
    collection: &mut Collection,
    env: &ImportedEnvironment,
) -> Result<WrittenEnvironment, ImportError> {
    // Creating an environment writes no secret, so a throwaway store keeps the real one untouched.
    let (name, _) = collection.create_environment(&env.name, &MemoryStore::new())?;
    let file = format!("environments/{name}.yml");
    let mut secrets = Vec::new();
    let mut variables = Vec::with_capacity(env.variables.len());
    for (j, variable) in env.variables.iter().enumerate() {
        if !variable.secret {
            variables.push(variable.clone());
            continue;
        }
        let source = env
            .sources
            .get(j)
            .cloned()
            .unwrap_or_else(|| format!("variables[{j}]"));
        let value = match &variable.value {
            VarValue::Text(text) => text.clone(),
            VarValue::Typed { data, .. } => data.clone(),
        };
        secrets.push((source, variable.name.clone(), value));
        let mut secret = EnvVariable::secret(variable.name.clone());
        secret.enabled = variable.enabled;
        variables.push(secret);
    }
    let Some(environment) = collection.environment_mut(&name) else {
        return Err(EnvError::NotFound {
            available: collection
                .environments()
                .iter()
                .map(|e| e.name().to_owned())
                .collect(),
            name,
            global: false,
        }
        .into());
    };
    environment.set_variables(variables);
    environment.save()?;
    let notes = placed(&env.notes, file.clone()).collect();
    Ok(WrittenEnvironment {
        name,
        file,
        notes,
        secrets,
    })
}

/// Store each secret with a non-empty value under `collection.secret_key(&written.name, var)`.
/// An empty value or a failed `set` adds a `NeedsInput` note and a `secrets_to_enter` entry.
pub(crate) fn store_secrets(
    collection: &Collection,
    written: &WrittenEnvironment,
    secrets: &dyn SecretStore,
    report: &mut ImportReport,
) {
    for (source, variable, value) in &written.secrets {
        let detail = if value.is_empty() {
            format!("the secret `{variable}` has no value; enter it before sending")
        } else {
            match secrets.set(&collection.secret_key(&written.name, variable), value) {
                Ok(()) => continue,
                Err(error) => format!(
                    "the secret `{variable}` could not be stored ({error}); enter it before sending"
                ),
            }
        };
        let mut note = ImportNote::new(source.clone(), NoteKind::NeedsInput, detail);
        note.file = Some(written.file.clone());
        report.notes.push(note);
        report
            .secrets_to_enter
            .push((written.name.clone(), variable.clone()));
    }
}
