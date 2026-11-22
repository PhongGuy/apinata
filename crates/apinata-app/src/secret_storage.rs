//! Where a secret field's value lives, as the hint under every secret field says it.

use crate::auth_section::AuthScope;
use crate::network_secrets::{env_name, is_template};
use crate::snapshot::{DotenvNames, SecretHome};
use apinata_core::{VarScope, VariableSet};

/// The home of a secret field's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SecretStorage {
    /// Written as is in the request, folder or collection file.
    File(AuthScope),
    /// A `.env` reference whose value the unsaved draft holds until it is saved.
    DotenvHeld,
    /// A `.env` reference.
    Dotenv,
    /// A reference to a secret variable, whose value the keychain keeps.
    Keychain,
    /// A reference to a plain variable of an environment file.
    EnvironmentFile,
}

impl SecretStorage {
    /// Where a field whose text is `text` keeps its value; None for an empty field or one
    /// whose reference names nothing the hint can place.
    pub fn of(
        text: &str,
        scope: AuthScope,
        vars: Option<&VariableSet>,
        dotenv: &DotenvNames,
    ) -> Option<SecretStorage> {
        if text.trim().is_empty() {
            return None;
        }
        if let Some(name) = env_name(text) {
            return Some(match dotenv.home(name) {
                Some(SecretHome::Held) => SecretStorage::DotenvHeld,
                _ => SecretStorage::Dotenv,
            });
        }
        if !crate::ui::is_variable_reference(text) {
            // Text around a reference is still written in the file.
            return Some(SecretStorage::File(scope));
        }
        let name = text.trim().trim_start_matches("{{").trim_end_matches("}}");
        let vars = vars?;
        // A secret with no value yet resolves to nothing, but it is still declared.
        let (scope, secret) = match vars.get(name) {
            Some(info) => (info.scope(), info.is_secret()),
            None if vars.is_unstored_secret(name) => (VarScope::Environment, true),
            None => return None,
        };
        Some(match scope {
            VarScope::Environment | VarScope::GlobalEnvironment if secret => {
                SecretStorage::Keychain
            }
            VarScope::Environment | VarScope::GlobalEnvironment => SecretStorage::EnvironmentFile,
            VarScope::ProcessEnv => SecretStorage::Dotenv,
            VarScope::Collection => SecretStorage::File(AuthScope::Collection),
            VarScope::Folder => SecretStorage::File(AuthScope::Folder),
            VarScope::Request => SecretStorage::File(AuthScope::Request),
            _ => return None,
        })
    }

    /// The hint's words.
    pub fn text(self) -> &'static str {
        match self {
            SecretStorage::File(AuthScope::Request) => "Stored in the request file",
            SecretStorage::File(AuthScope::Folder) => "Stored in the folder file",
            SecretStorage::File(AuthScope::Collection) => "Stored in the collection file",
            SecretStorage::DotenvHeld => "Saved to .env when you save",
            SecretStorage::Dotenv => "Stored in .env",
            SecretStorage::Keychain => "Secret, kept in your keychain",
            SecretStorage::EnvironmentFile => "Stored in the environment file",
        }
    }

    /// Whether the value is written as is in a file git may track, so the field offers to
    /// move it to a secret.
    pub fn movable(self, text: &str) -> bool {
        matches!(self, SecretStorage::File(_)) && !is_template(text)
    }
}
