//! The xtask integration suite.
//!
//! Each file beside this one is a module of this single binary, so the
//! suite links once. A new test file must be declared below, or it never runs.

mod dev_desktop;
mod icons;
mod package_linux_script;
mod packager_config;
mod packager_metadata;
mod script_suites;
mod update_manifest;
mod windows_check;
mod workflows;
