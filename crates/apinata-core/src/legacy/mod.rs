//! Collections in the legacy `.bru` format: detection and conversion to OpenCollection YAML.

pub(crate) mod bru;
mod convert;
mod error;
mod plan;
pub(crate) mod yml;

pub use convert::{ConvertTarget, convert, ensure_new_folder_free};
pub use error::{BruError, ConvertError};
pub use plan::{ConversionPlan, is_legacy_collection, plan_conversion};
