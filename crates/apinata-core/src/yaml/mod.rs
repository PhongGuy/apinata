//! A port of the `yaml` 2.3.4 stringifier under Bruno's `stringifyYml` options.

pub(crate) mod emit;
mod scalar;

#[cfg(test)]
mod parity_tests;
