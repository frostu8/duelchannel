//! Match endpoint request bodies.

use serde::{Deserialize, Serialize};

use crate::battle::{BattleStatus, PlayerTeam};

/// Request to create a match.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateBattleRequest {
    /// The level the battle is taking place on.
    pub level_name: String,
    /// The players to register for this battle.
    pub participants: Vec<CreateBattleParticipant>,
    /// How long bets should last for, in seconds.
    ///
    /// Uses `20` seconds as the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bet_time: Option<i64>,
}

/// A participant in a [`CreateBattleRequest`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateBattleParticipant {
    /// The ID of the participant.
    pub id: String,
    /// What team they are on.
    pub team: PlayerTeam,
    /// The player's kartspeed.
    pub kart_speed: i32,
    /// The player's kartweight.
    pub kart_weight: i32,
    /// The skin the player is running.
    pub skin: String,
}

/// Request to set the placement of a player.
///
/// This may be updated continuously until the match is ended.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdatePlayerPlacementRequest {
    /// The finishing time of the player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_time: Option<i32>,
}

/// Request to update a match.
///
/// Concluded matches cannot be updated.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateBattleRequest {
    /// Match status.
    ///
    /// If this flag is set to [`BattleStatus::Concluded`] or
    /// [`BattleStatus::Cancelled`], the match ends, and processing is done for
    /// it. All players without finish times have their NO CONTEST values set
    /// to `true` if it hasn't been done already.
    ///
    /// If the match was not cancelled, the match is then evaluated.
    ///
    /// If the match's current status is [`BattleStatus::Ongoing`], and this
    /// request sets it to `BattleStatus::Ongoing`, nothing happens.
    ///
    /// **This action is irreversible.** Be careful!
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<BattleStatus>,
    /// Updates the margin score of the battle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin_score: Option<i32>,
}

/// Request to update a wager.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateWager {
    /// The mobiums the user bets.
    ///
    /// This can only be between 0 and the mobiums the user has.
    ///
    /// If this is 0, this removes the wager.
    pub mobiums: i64,
    /// The victor the user is betting on.
    ///
    /// If this team wins, they will be paid out.
    pub victor: PlayerTeam,
    /// The [CSRF token].
    ///
    /// [CSRF token]: crate::session::Session::shuffle_csrf
    pub csrf: String,
}
