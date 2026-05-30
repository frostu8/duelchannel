//! Server-related requests.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::server::MapConfig;

/// An update server request.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateServerRequest {
    /// The new name of the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The list of map bans.
    ///
    /// These are replaced as-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maps: Option<HashMap<String, MapConfig>>,
}
