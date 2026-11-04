//! Maps the legacy reader's reference-shaped JSON to one OpenCollection YAML document per file.

mod auth;
mod collection;
mod environment;
mod item;
mod plan;
mod request;
mod rows;
mod run_request;
#[cfg(test)]
mod tests;
mod value;

pub(crate) use collection::{map_collection, map_folder};
pub(crate) use environment::map_environment;
#[cfg(test)]
pub(crate) use plan::plan_files;
pub(crate) use plan::{FileError, ROOT_COLLECTION, ignored_names, is_bru, plan_walked, walk_files};
pub(crate) use request::map_request;
#[cfg(test)]
pub(crate) use run_request::strip_bru_ext;
