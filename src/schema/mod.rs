//! Database schemas and accessing.

use derive_more::Display;

pub mod battle;
pub mod user;

/// Represents an attempt to convert to an API model without the required
/// preloads.
#[derive(Debug, Clone, Display)]
#[display("missing data for field `{field_name}`")]
pub struct MissingData {
    /// The name of the field missing.
    field_name: String,
}

impl MissingData {
    pub fn new(field_name: impl Into<String>) -> MissingData {
        MissingData {
            field_name: field_name.into(),
        }
    }
}

impl std::error::Error for MissingData {}
