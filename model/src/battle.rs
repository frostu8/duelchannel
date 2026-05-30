//! Battle data representations.

use derive_more::Deref;
use num_enum::{IntoPrimitive, TryFromPrimitive};

use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};

use serde_repr::{Deserialize_repr, Serialize_repr};

use crate::{profile::Skin, user::User};

/// A single match.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Battle {
    /// The unique identifier of the match.
    pub id: String,
    /// The level name the match played on.
    pub level_name: String,
    /// The status of the match.
    pub status: BattleStatus,
    /// The margin score of the match.
    ///
    /// This is the number of margin boosts the match had. This is typically
    /// zero, and goes up steadily after 3 minutes of playtime.
    pub margin_score: i32,
    /// A link to the replay associated with the match.
    pub replay_url: Option<String>,
    /// When the match started.
    pub started_at: DateTime<Utc>,
    /// The participants.
    pub participants: Vec<Participant>,
}

/// A participant in a match.
#[derive(Clone, Debug, Deref, Deserialize, Serialize)]
pub struct Participant {
    /// The name of the player.
    pub name: String,
    /// The team they are on.
    pub team: PlayerTeam,
    /// The player's finish time, if they finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_time: Option<i32>,
    /// If the player no contest'd.
    #[serde(default)]
    pub no_contest: bool,
    /// The player's skin.
    ///
    /// May not be present, for very old matches.
    pub skin: Option<Skin>,
    /// The internal name of the player's skin color.
    ///
    /// May not be present, for older matches.
    pub skin_color: Option<String>,
    /// The user participating.
    #[deref]
    pub user: User,
}

/// The match's status.
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize_repr,
    Serialize_repr,
    PartialEq,
    Eq,
    Hash,
    TryFromPrimitive,
    IntoPrimitive,
)]
#[repr(u8)]
pub enum BattleStatus {
    /// The match is ongoing. No victors have been determined.
    Ongoing = 0,
    /// The match concluded normally.
    Concluded = 1,
    /// The match was cancelled.
    Cancelled = 2,
}

/// A team side.
#[derive(
    Clone,
    Copy,
    Debug,
    Deserialize_repr,
    Serialize_repr,
    PartialEq,
    Eq,
    Hash,
    TryFromPrimitive,
    IntoPrimitive,
)]
#[repr(u8)]
pub enum PlayerTeam {
    /// The red team.
    ///
    /// Player 1 is on this team.
    Red = 0,
    /// The blue team.
    ///
    /// Player 2 is on this team.
    Blue = 1,
}

/// A compact representation of a match meant to convey some statistics.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BattlePoint {
    /// The ID of the battle.
    pub id: String,
    /// The name of the level the battle took place on.
    pub level_name: String,
    /// The margin score of the battle.
    pub margin_score: Option<i32>,
    /// The statistics of the battle.
    #[serde(flatten)]
    pub statistics: BattleStatistics,
}

/// The statistics of a battle.
///
/// A single battle can be represented as a single point in n-dimensional
/// space.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BattleStatistics {
    /// The average MMR of the match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_mmr: Option<i32>,
    /// The match quality.
    ///
    /// Lower numbers are better.
    pub quality: Option<f32>,
    /// The finish time of the match.
    pub finish_time: Option<i32>,
}

impl BattleStatistics {
    /// Checks if the statistics are empty.
    pub fn is_empty(&self) -> bool {
        self.avg_mmr.is_none() && self.quality.is_none() && self.finish_time.is_none()
    }
}
