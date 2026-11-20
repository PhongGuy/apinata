//! The environment a shell uses when the caller names one or none.

use super::Collection;

/// Which environment a run uses, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvironmentChoice {
    /// No environment: none was named and the presets name none.
    None,
    /// The environment the caller named, whether or not it exists.
    Given(String),
    /// The presets' default environment, which the collection has.
    Preset(String),
    /// The presets name an environment the collection lacks, so none is used.
    MissingPreset(String),
}

impl EnvironmentChoice {
    /// The environment to resolve against, if any.
    pub fn name(&self) -> Option<&str> {
        match self {
            EnvironmentChoice::Given(name) | EnvironmentChoice::Preset(name) => Some(name),
            EnvironmentChoice::None | EnvironmentChoice::MissingPreset(_) => None,
        }
    }

    /// What to tell the user about the choice: only a missing preset needs a word.
    pub fn warning(&self) -> Option<String> {
        match self {
            EnvironmentChoice::MissingPreset(name) => Some(format!(
                "default environment \"{name}\" not found; using no environment"
            )),
            _ => None,
        }
    }
}

impl Collection {
    /// `explicit` when given, else the presets' `defaultEnvironment` while an environment of
    /// that name exists. A preset naming a missing environment is not an error; it means none.
    pub fn choose_environment(&self, explicit: Option<&str>) -> EnvironmentChoice {
        if let Some(name) = explicit {
            return EnvironmentChoice::Given(name.to_owned());
        }
        let presets = self.file().presets();
        let preset = presets.default_environment();
        if preset.is_empty() {
            EnvironmentChoice::None
        } else if self.environment(preset).is_some() {
            EnvironmentChoice::Preset(preset.to_owned())
        } else {
            EnvironmentChoice::MissingPreset(preset.to_owned())
        }
    }
}
