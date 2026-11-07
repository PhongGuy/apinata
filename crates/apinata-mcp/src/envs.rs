//! Environment tools. Agents never see, set or delete a secret value: a rename moves stored
//! values with the environment, and deleting an environment that holds a secret is refused.

use apinata_core::{
    Collection, EnvVariable, EnvWarning, SecretStore, VarValue, VariableRefusal, without_variable,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dto::EnvironmentDoc;
use crate::reads;
use crate::writes::{Failure, write_retrying};

/// `rename_environment`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RenameEnvArgs {
    /// The environment's current name.
    pub from: String,
    /// Its new name; other environments that extend it are updated to match.
    pub to: String,
}

/// One variable to add or change; absent fields keep their current value, and a new variable
/// is enabled and plain.
#[derive(Deserialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct VarInput {
    /// The variable's name.
    pub name: String,
    /// The value as text; refused for a secret variable or with `secret: true`.
    pub value: Option<String>,
    /// A declared type such as `number`, with the value as its data; refused like a value.
    #[serde(rename = "type")]
    pub value_type: Option<String>,
    /// Whether the variable is set.
    pub enabled: Option<bool>,
    /// `true` moves a plain variable's value out of the file into the keychain, or adds an
    /// empty secret; `false` on a secret is refused, and so is `true` on a plain variable whose
    /// name already has a stored value.
    pub secret: Option<bool>,
}

/// `update_environment`'s arguments.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateEnvArgs {
    /// The environment to edit.
    pub name: String,
    /// Variables to add or change, applied in order after `remove`.
    #[serde(default)]
    pub set: Vec<VarInput>,
    /// Names of plain variables to remove; a secret cannot be removed through MCP.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// An environment's name after a create or rename, and what could not be finished.
#[derive(Serialize, JsonSchema)]
pub(crate) struct EnvNameResult {
    /// The name the environment now has, as sanitised and de-duplicated.
    pub name: String,
    /// Steps that did not complete, naming variables but never values.
    pub warnings: Vec<String>,
}

/// What `delete_environment` removed.
#[derive(Serialize, JsonSchema)]
pub(crate) struct DeletedEnv {
    /// The environment that was deleted.
    pub deleted: String,
}

fn texts(warnings: &[EnvWarning]) -> Vec<String> {
    warnings.iter().map(ToString::to_string).collect()
}

/// Create an empty environment named after `name`, returning the name core settled on.
pub(crate) fn create(
    c: &mut Collection,
    name: &str,
    store: &dyn SecretStore,
) -> Result<EnvNameResult, String> {
    let (name, warnings) = c
        .create_environment(name, store)
        .map_err(|e| e.to_string())?;
    Ok(EnvNameResult {
        name,
        warnings: texts(&warnings),
    })
}

/// Copy an environment and its stored secret values; the copy is named as a new one is.
pub(crate) fn duplicate(
    c: &mut Collection,
    name: &str,
    store: &dyn SecretStore,
) -> Result<EnvNameResult, String> {
    let (name, warnings) = c
        .duplicate_environment(name, store)
        .map_err(|e| e.to_string())?;
    Ok(EnvNameResult {
        name,
        warnings: texts(&warnings),
    })
}

/// Rename an environment, moving its stored secrets; rewriting the environments that extend it
/// can conflict with another writer, so the call is retried once.
pub(crate) fn rename(
    c: &mut Collection,
    a: &RenameEnvArgs,
    store: &dyn SecretStore,
) -> Result<EnvNameResult, String> {
    let warnings = write_retrying(c, |c| {
        c.rename_environment(&a.from, &a.to, store)
            .map_err(Failure::env)
    })?;
    Ok(EnvNameResult {
        name: a.to.clone(),
        warnings: texts(&warnings),
    })
}

/// Delete a plain environment. One holding a secret is refused: its stored values are not in
/// git, so only the user may discard them. Core refuses one that others extend.
pub(crate) fn delete(
    c: &mut Collection,
    name: &str,
    store: &dyn SecretStore,
) -> Result<DeletedEnv, String> {
    let env = c
        .environment(name)
        .ok_or_else(|| c.environment_not_found(name).to_string())?;
    if env.variables().iter().any(|v| v.secret) {
        return Err(format!(
            "environment {name} holds secret variables, whose stored values git cannot restore; \
             the user deletes it in the Apinata app"
        ));
    }
    c.delete_environment(name, store)
        .map_err(|e| e.to_string())?;
    Ok(DeletedEnv {
        deleted: name.to_owned(),
    })
}

/// Apply `a` to environment `a.name` in two phases: the plain edits are saved, then each named
/// plain variable is made secret. Every refusal is decided before the first write; a failure
/// in the second phase leaves the plain edits saved. Returns the environment without secret
/// values.
pub(crate) fn update(
    c: &mut Collection,
    a: &UpdateEnvArgs,
    store: &dyn SecretStore,
) -> Result<EnvironmentDoc, String> {
    let to_secret = write_retrying(c, |c| {
        let not_found =
            |c: &Collection| Failure::refused(c.environment_not_found(&a.name).to_string());
        let Some(env) = c.environment(&a.name) else {
            return Err(not_found(c));
        };
        // Planned from the file as loaded for this attempt, so a retry re-plans after a conflict.
        let (vars, to_secret) = plan(env.variables(), a).map_err(Failure::refused)?;
        refuse_stored(c, &a.name, &to_secret, store).map_err(Failure::refused)?;
        let Some(env) = c.environment_mut(&a.name) else {
            return Err(not_found(c));
        };
        env.set_variables(vars);
        env.save().map_err(Failure::save)?;
        Ok(to_secret)
    })?;
    // Phase 2 is not retried: phase 1 is on disk, so planning again would refuse its removals.
    for (i, var) in to_secret.iter().enumerate() {
        if let Err(e) = c.set_variable_secret(&a.name, var, true, store) {
            if let Some(path) = c.environment(&a.name).map(|env| env.path().to_owned()) {
                let _ = c.revert(&path);
            }
            return Err(format!(
                "the plain edits were saved, but these variables are still plain: {} ({e})",
                to_secret[i..].join(", ")
            ));
        }
    }
    reads::environment(c, &a.name)
}

/// Refuses when any of `names` already has a stored value, which making it secret could
/// replace, or when the store cannot say: only the user may discard a stored value.
fn refuse_stored(
    c: &Collection,
    env: &str,
    names: &[String],
    store: &dyn SecretStore,
) -> Result<(), String> {
    for name in names {
        match store.get(&c.secret_key(env, name)) {
            Ok(None) => {}
            Ok(Some(_)) => {
                return Err(format!(
                    "{name} has a stored value that making it secret would replace; the user \
                     removes it first in the Apinata app or with `apinata env forget`"
                ));
            }
            Err(e) => {
                return Err(format!(
                    "{name} stays plain: could not check for a stored value it would replace \
                     ({e})"
                ));
            }
        }
    }
    Ok(())
}

/// The variables after `a`, and the existing plain variables to make secret once they are
/// saved. Refusals name the variable, never a value.
fn plan(
    mut vars: Vec<EnvVariable>,
    a: &UpdateEnvArgs,
) -> Result<(Vec<EnvVariable>, Vec<String>), String> {
    // Core decides what a removal may drop, so the CLI and this tool refuse alike.
    for name in &a.remove {
        vars = without_variable(vars, name, false).map_err(|r| match r {
            VariableRefusal::Secret => format!(
                "{name} is secret; its stored value is not in git, so the user removes it in \
                 the Apinata app"
            ),
            _ => format!("environment {} has no variable {name}", a.name),
        })?;
    }

    let mut to_secret = Vec::new();
    for input in &a.set {
        let name = &input.name;
        // A name is secret if any of its entries is, so a plain duplicate cannot stand in for
        // it; edits then go to the secret entry.
        let existing = vars
            .iter()
            .position(|v| &v.name == name && v.secret)
            .or_else(|| vars.iter().position(|v| &v.name == name));
        let is_secret = existing.is_some_and(|i| vars[i].secret);
        let wants_secret = input.secret == Some(true);
        if (is_secret || wants_secret) && (input.value.is_some() || input.value_type.is_some()) {
            return Err(format!(
                "{name}: a secret's value and type are set by the user in the Apinata app or \
                 with --env-var"
            ));
        }
        if is_secret && input.secret == Some(false) {
            return Err(format!(
                "{name} is secret; making it plain would write its value where agents read it"
            ));
        }
        let at = existing.unwrap_or_else(|| {
            vars.push(if wants_secret {
                EnvVariable::secret(name.clone())
            } else {
                EnvVariable::new(name.clone(), VarValue::Text(String::new()))
            });
            vars.len() - 1
        });
        let var = &mut vars[at];
        // A value keeps the current type unless one is given; a type alone retypes the data.
        if input.value.is_some() || input.value_type.is_some() {
            let (kind, data) = match &var.value {
                VarValue::Text(data) => (None, data.clone()),
                VarValue::Typed { kind, data } => (Some(kind.clone()), data.clone()),
                _ => (None, String::new()),
            };
            let data = input.value.clone().unwrap_or(data);
            var.value = match input.value_type.clone().or(kind) {
                Some(kind) => VarValue::Typed { kind, data },
                None => VarValue::Text(data),
            };
        }
        if let Some(enabled) = input.enabled {
            var.enabled = enabled;
        }
        if existing.is_some() && !is_secret && wants_secret {
            to_secret.push(name.clone());
        }
    }
    Ok((vars, to_secret))
}

#[cfg(test)]
mod tests {
    use super::UpdateEnvArgs;
    use apinata_core::TimeoutStore;
    use apinata_core::{Collection, EnvVariable, SecretError, SecretKey, SecretStore, VarValue};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A keychain that never answers, as one waiting on an unattended unlock prompt.
    #[derive(Default)]
    struct Hung {
        started: AtomicUsize,
    }

    impl Hung {
        fn hang<T>(&self) -> T {
            self.started.fetch_add(1, Ordering::SeqCst);
            loop {
                std::thread::park();
            }
        }
    }

    impl SecretStore for Hung {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            self.hang()
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            self.hang()
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            self.hang()
        }
    }

    /// A keychain that refuses every write, as one the user declined to unlock.
    struct Locked;

    impl SecretStore for Locked {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Ok(None)
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Unavailable {
                message: "locked".into(),
            })
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            Ok(())
        }
    }

    /// A keychain that cannot be read, so nothing is known about what it holds.
    struct Unreadable;

    impl SecretStore for Unreadable {
        fn get(&self, _: &SecretKey) -> Result<Option<String>, SecretError> {
            Err(SecretError::Unavailable {
                message: "locked".into(),
            })
        }
        fn set(&self, _: &SecretKey, _: &str) -> Result<(), SecretError> {
            panic!("nothing is stored after a failed read")
        }
        fn delete(&self, _: &SecretKey) -> Result<(), SecretError> {
            panic!("nothing is deleted after a failed read")
        }
    }

    #[test]
    fn an_unreadable_keychain_refuses_before_any_write() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let root = dir.path();
        std::fs::write(
            root.join("opencollection.yml"),
            "opencollection: \"1.0.0\"\ninfo:\n  name: c\n",
        )
        .expect("write");
        std::fs::create_dir(root.join("environments")).expect("mkdir");
        let env =
            "name: prod\nvariables:\n  - name: host\n    value: a\n  - name: alpha\n    value: x\n";
        std::fs::write(root.join("environments/prod.yml"), env).expect("write");
        let mut c = Collection::open(root).expect("opens");
        let args: UpdateEnvArgs = serde_json::from_value(serde_json::json!({
            "name": "prod",
            "set": [{"name": "host", "value": "b"}, {"name": "alpha", "secret": true}],
        }))
        .expect("args");
        let Err(err) = super::update(&mut c, &args, &Unreadable) else {
            panic!("an unknown answer must refuse");
        };
        assert!(err.contains("alpha"), "{err}");
        assert!(!err.contains("plain edits were saved"), "{err}");
        let file = std::fs::read_to_string(root.join("environments/prod.yml")).expect("read");
        assert_eq!(file, env);
    }

    #[test]
    fn a_failed_move_to_the_keychain_says_the_plain_edits_were_saved_and_names_what_is_left() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let root = dir.path();
        std::fs::write(
            root.join("opencollection.yml"),
            "opencollection: \"1.0.0\"\ninfo:\n  name: c\n",
        )
        .expect("write");
        std::fs::create_dir(root.join("environments")).expect("mkdir");
        let env = "name: prod\nvariables:\n  - name: host\n    value: a\n  - name: alpha\n    value: x\n  - name: beta\n    value: y\n";
        std::fs::write(root.join("environments/prod.yml"), env).expect("write");
        let mut c = Collection::open(root).expect("opens");
        let args: UpdateEnvArgs = serde_json::from_value(serde_json::json!({
            "name": "prod",
            "set": [{"name": "host", "value": "b"}, {"name": "alpha", "secret": true},
                    {"name": "beta", "secret": true}],
        }))
        .expect("args");
        let Err(err) = super::update(&mut c, &args, &Locked) else {
            panic!("a locked keychain must fail the call");
        };
        assert!(err.contains("plain edits were saved"), "{err}");
        assert!(err.contains("alpha") && err.contains("beta"), "{err}");
        let saved = std::fs::read_to_string(root.join("environments/prod.yml")).expect("read");
        assert!(saved.contains("value: b"), "{saved}");
    }

    #[test]
    fn a_plain_duplicate_does_not_carry_a_secret_of_the_same_name_out_with_it() {
        let vars = vec![
            EnvVariable::new("token", VarValue::Text("x".into())),
            EnvVariable::secret("token"),
        ];
        let args = UpdateEnvArgs {
            name: "prod".into(),
            set: Vec::new(),
            remove: vec!["token".into()],
        };
        let err = super::plan(vars, &args).expect_err("refused");
        assert!(err.contains("token is secret"), "{err}");
    }

    #[test]
    fn a_rename_over_a_hung_store_waits_one_timeout_not_one_per_secret() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let root = dir.path();
        std::fs::write(
            root.join("opencollection.yml"),
            "opencollection: \"1.0.0\"\ninfo:\n  name: c\n",
        )
        .expect("write");
        std::fs::create_dir(root.join("environments")).expect("mkdir");
        let env = "name: prod\nvariables:\n  - secret: true\n    name: a\n  - secret: true\n    name: b\n  - secret: true\n    name: c\n";
        std::fs::write(root.join("environments/prod.yml"), env).expect("write");
        let mut c = Collection::open(root).expect("opens");
        let hung = Arc::new(Hung::default());
        let store = TimeoutStore::new(Arc::<Hung>::clone(&hung), Duration::from_millis(200));
        let warnings = c
            .rename_environment("prod", "live", &store)
            .expect("the rename stands");
        // Counted calls, not elapsed time: only the first secret reaches the keychain.
        assert_eq!(
            hung.started.load(Ordering::SeqCst),
            1,
            "one wait, not three"
        );
        assert_eq!(
            warnings.len(),
            3,
            "each secret reports that it did not move"
        );
    }
}
