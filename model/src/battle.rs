//! Battle data representations.

use derive_more::Deref;
use num_enum::{IntoPrimitive, TryFromPrimitive};

use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};

use serde_repr::{Deserialize_repr, Serialize_repr};

use crate::{player::Player, user::User};

/// A single match.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Battle {
    /// The unique identifier of the match.
    pub id: String,
    /// The level name the match played on.
    pub level_name: String,
    /// The participants.
    pub participants: Vec<Participant>,
    /// The status of the match.
    pub status: BattleStatus,
    /// The margin score of the match.
    ///
    /// This is the number of margin boosts the match had. This is typically
    /// zero, and goes up steadily after 3 minutes of playtime.
    pub margin_score: i32,
    /// Whether the match is accepting bets or not.
    pub accepting_bets: bool,
    /// When the match started.
    pub started_at: DateTime<Utc>,
    /// The amount of time that will pass before wagers close, in ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closes_in: Option<i64>,
}

/// A participant in a match.
#[derive(Clone, Debug, Deref, Deserialize, Serialize)]
pub struct Participant {
    /// The player participating.
    #[deref]
    #[serde(flatten)]
    pub player: Player,
    /// The team they are on.
    pub team: PlayerTeam,
    /// The player's finish time, if they finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_time: Option<i32>,
    /// If the player no contest'd.
    #[serde(default)]
    pub no_contest: bool,
    /// The player's kartspeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kart_speed: Option<i32>,
    /// The player's kartweight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kart_weight: Option<i32>,
    /// The skin the player is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<String>,
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
    ///
    /// Wagers were refunded, and the pot was cancelled.
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

/// A battle bet.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BattleWager {
    /// The user that made this wager.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<User>,
    /// The wager amount.
    pub mobiums: i64,
    /// What team the player is betting to win.
    pub victor: PlayerTeam,
    /// When the wager was last updated at.
    pub updated_at: DateTime<Utc>,
}
