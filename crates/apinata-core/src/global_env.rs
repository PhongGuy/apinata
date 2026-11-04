//! Global environments: environment files in the profile directory that every collection can
//! select, created, renamed and deleted with the same operations as a collection's own.

use crate::collection::{Environment, load_environment_dir};
use crate::env_ops::{self, EnvHost, OrphanReport};
use crate::error::{EnvError, FileError, OpenError};
use crate::secrets::{EnvWarning, SecretKey, SecretStore};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The global environments loaded from one directory, sorted by name.
#[derive(Debug)]
#[non_exhaustive]
pub struct GlobalEnvironments {
    dir: PathBuf,
    environments: Vec<Environment>,
    load_errors: Vec<FileError>,
}

impl GlobalEnvironments {
    /// `$APINATA_PROFILE_DIR/global-environments` when that variable is set and non-empty,
    /// else `<config_dir>/apinata/global-environments`; `None` without a config dir.
    pub fn default_dir() -> Option<PathBuf> {
        dir_for(std::env::var_os("APINATA_PROFILE_DIR"), dirs::config_dir())
    }

    /// Load every `*.yml` file in `dir` (no ignore rules), sorted by name; unparsable files go
    /// to [`GlobalEnvironments::load_errors`]. A missing `dir` is an empty set and is not
    /// created; one that exists but cannot be read is [`OpenError::Io`].
    pub fn open(dir: impl Into<PathBuf>) -> Result<GlobalEnvironments, OpenError> {
        let dir = dir.into();
        let mut load_errors = Vec::new();
        let environments =
            load_environment_dir(&dir, |_| false, &mut load_errors).map_err(|source| {
                OpenError::Io {
                    path: dir.clone(),
                    source,
                }
            })?;
        Ok(GlobalEnvironments {
            dir,
            environments,
            load_errors,
        })
    }

    /// The directory the environments live in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Environments, sorted by name.
    pub fn environments(&self) -> &[Environment] {
        &self.environments
    }

    /// The [`EnvError::NotFound`] for `name`, listing the global environments that exist.
    pub fn environment_not_found(&self, name: &str) -> EnvError {
        env_ops::not_found(self, name)
    }

    /// The environment whose file stem is `name`.
    pub fn environment(&self, name: &str) -> Option<&Environment> {
        self.env(name)
    }

    /// The environment whose file stem is `name`, for editing.
    pub fn environment_mut(&mut self, name: &str) -> Option<&mut Environment> {
        self.env_mut(name)
    }

    /// Files that could not be loaded.
    pub fn load_errors(&self) -> &[FileError] {
        &self.load_errors
    }

    /// The key a secret of `environment` is stored under: the canonical directory (else the
    /// directory as given) stands where a collection's root would.
    pub fn secret_key(&self, environment: &str, variable: &str) -> SecretKey {
        let dir = std::fs::canonicalize(&self.dir).unwrap_or_else(|_| self.dir.clone());
        SecretKey::new(dir, environment, variable)
    }

    /// As [`Collection::create_environment`](crate::Collection::create_environment); creates
    /// the directory when it is missing.
    pub fn create_environment(
        &mut self,
        name: &str,
        store: &dyn SecretStore,
    ) -> Result<(String, Vec<EnvWarning>), EnvError> {
        env_ops::create_environment(self, name, store)
    }

    /// As [`Collection::duplicate_environment`](crate::Collection::duplicate_environment).
    pub fn duplicate_environment(
        &mut self,
        from: &str,
        store: &dyn SecretStore,
    ) -> Result<(String, Vec<EnvWarning>), EnvError> {
        env_ops::duplicate_environment(self, from, store)
    }

    /// As [`Collection::rename_environment`](crate::Collection::rename_environment).
    pub fn rename_environment(
        &mut self,
        from: &str,
        to: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        env_ops::rename_environment(self, from, to, store)
    }

    /// As [`Collection::extended_by`](crate::Collection::extended_by).
    pub fn extended_by(&self, name: &str) -> Vec<String> {
        env_ops::extended_by(self, name)
    }

    /// As [`Collection::delete_environment`](crate::Collection::delete_environment).
    pub fn delete_environment(
        &mut self,
        name: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        env_ops::delete_environment(self, name, store)
    }

    /// As [`Collection::set_variable_secret`](crate::Collection::set_variable_secret).
    pub fn set_variable_secret(
        &mut self,
        env: &str,
        var: &str,
        secret: bool,
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        env_ops::set_variable_secret(self, env, var, secret, store)
    }

    /// As [`Collection::orphaned_secrets`](crate::Collection::orphaned_secrets).
    pub fn orphaned_secrets(
        &self,
        env: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<String>, EnvError> {
        env_ops::orphaned_secrets(self, env, store)
    }

    /// As [`Collection::remove_orphaned_secrets`](crate::Collection::remove_orphaned_secrets).
    pub fn remove_orphaned_secrets(
        &self,
        env: &str,
        names: &[String],
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        env_ops::remove_orphaned_secrets(self, env, names, store)
    }

    /// As [`Collection::all_orphaned_secrets`](crate::Collection::all_orphaned_secrets).
    pub fn all_orphaned_secrets(&self, store: &dyn SecretStore) -> Result<OrphanReport, EnvError> {
        env_ops::all_orphaned_secrets(self, store)
    }

    /// As [`Collection::fileless_secrets`](crate::Collection::fileless_secrets).
    pub fn fileless_secrets(
        &self,
        env: &str,
        store: &dyn SecretStore,
    ) -> Result<Vec<String>, EnvError> {
        env_ops::fileless_secrets(self, env, store)
    }

    /// As [`Collection::remove_fileless_secrets`](crate::Collection::remove_fileless_secrets).
    pub fn remove_fileless_secrets(
        &self,
        env: &str,
        names: &[String],
        store: &dyn SecretStore,
    ) -> Result<Vec<EnvWarning>, EnvError> {
        env_ops::remove_fileless_secrets(self, env, names, store)
    }
}

impl EnvHost for GlobalEnvironments {
    fn is_global(&self) -> bool {
        true
    }

    fn env_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn envs(&self) -> &[Environment] {
        &self.environments
    }

    fn envs_mut(&mut self) -> &mut Vec<Environment> {
        &mut self.environments
    }

    fn env_secret_key(&self, env: &str, var: &str) -> SecretKey {
        self.secret_key(env, var)
    }
}

/// The global environment directory under a profile override or the config directory; an
/// empty override counts as unset.
fn dir_for(profile: Option<OsString>, config: Option<PathBuf>) -> Option<PathBuf> {
    match profile.filter(|p| !p.is_empty()) {
        Some(profile) => Some(PathBuf::from(profile).join("global-environments")),
        None => config.map(|c| c.join("apinata").join("global-environments")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_variable_wins_when_non_empty() {
        let config = || Some(PathBuf::from("/c"));
        assert_eq!(
            dir_for(Some("/p".into()), config()),
            Some(PathBuf::from("/p/global-environments"))
        );
        assert_eq!(
            dir_for(Some("".into()), config()),
            Some(PathBuf::from("/c/apinata/global-environments"))
        );
        assert_eq!(
            dir_for(None, config()),
            Some(PathBuf::from("/c/apinata/global-environments"))
        );
        assert_eq!(
            dir_for(Some("/p".into()), None),
            Some(PathBuf::from("/p/global-environments"))
        );
        assert_eq!(dir_for(None, None), None);
    }
}
