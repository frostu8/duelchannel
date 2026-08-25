//! API error structs.

use derive_more::{Display, Error};

use serde::{Deserialize, Serialize};

use utoipa::ToSchema;

/// An API error.
#[derive(Clone, Debug, Display, Deserialize, Error, Serialize, ToSchema)]
#[display("{message}")]
pub struct ApiError {
    pub message: String,
}
