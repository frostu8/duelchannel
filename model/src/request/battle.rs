//! Match endpoint request bodies.

use serde::{Deserialize, Serialize};

use utoipa::ToSchema;

use crate::{
    battle::{BattleStatus, ItemUsage, PlayerTeam},
    profile::{Rrid, Skin},
};

/// Request to create a match.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateBattleRequest {
    /// The level the battle is taking place on.
    pub level_name: String,
    /// The level's internal identifier.
    ///
    /// This is the level's lumpname.
    pub level_id: String,
    /// The players to register for this battle.
    pub participants: Vec<CreateBattleParticipant>,
}

/// A participant in a [`CreateBattleRequest`].
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateBattleParticipant {
    /// The short ID of the participant.
    pub user_id: String,
    /// The public key of the participant.
    pub public_key: Rrid,
    /// The player's display name at the time of match creation.
    pub name: String,
    /// What team they are on.
    pub team: PlayerTeam,
    /// The player's skin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<Skin>,
    /// The name of the player's skin color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin_color: Option<String>,
}

/// Request to set the placement of a player.
///
/// This may be updated continuously until the match is ended.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdatePlayerPlacementRequest {
    /// The finishing time of the player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_time: Option<i32>,
    /// The player's score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i32>,
    /// Player roulette statistics.
    ///
    /// Statistics that share a stack and item will increment in-place instead
    /// of appending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roulette: Option<Vec<ItemUsage>>,
}

/// Request to update a match.
///
/// Concluded matches cannot be updated.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
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
