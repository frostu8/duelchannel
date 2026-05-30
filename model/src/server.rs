//! Servers.

use std::collections::HashMap;

use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};

use serde_repr::{Deserialize_repr, Serialize_repr};

/// A single server registered to the API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Server {
    /// The unique ID of the server.
    pub id: i32,
    /// The name of the server as it appears on UI.
    ///
    /// May not be the "canonical name" on the server list.
    pub name: String,
    /// Map config.
    pub maps: HashMap<String, MapConfig>,
}

/// A config for a specific map.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct MapConfig {
    /// The status of the map.
    pub status: BannedStatus,
    /// A modified wincon for the map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_condition: Option<i32>,
    /// A skill range the map config targets.
    #[serde(default, skip_serializing_if = "SkillRange::is_all")]
    pub skill_range: SkillRange,
    /// A user-defined note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A range of MMRs.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct SkillRange {
    /// The lower bound of the range.
    pub lower: Option<i32>,
    /// The upper bound of the range.
    pub upper: Option<i32>,
}

impl SkillRange {
    /// Checks if the `SkillRange` represents all skills.
    pub fn is_all(&self) -> bool {
        self.lower.is_none() && self.upper.is_none()
    }
}

/// A config for a specific map.
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize_repr,
    PartialEq,
    Eq,
    Serialize_repr,
    IntoPrimitive,
    TryFromPrimitive,
)]
#[repr(u8)]
pub enum BannedStatus {
    /// The map should not be played.
    Blacklist = 0,
    /// The map should be played.
    Whitelist = 1,
    /// The map should be played, but it's inclusion is subject to debate
    /// (informational only).
    Suspect = 2,
}
