//! Battle data representations.

use std::{
    fmt::{self, Display, Formatter},
    str::FromStr,
};

use derive_more::{Deref, Display};
use num_enum::{IntoPrimitive, TryFromPrimitive};

use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use serde::{Deserializer, Serializer, de::Error as _};

use serde_repr::{Deserialize_repr, Serialize_repr};

use utoipa::ToSchema;

use crate::{profile::Skin, user::User};

/// A single match.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct Battle {
    /// The unique identifier of the match.
    pub id: String,
    /// The level name the match played on.
    pub level_name: String,
    /// The level's internal identifier (the map lumpname, e.g. `RR_AUTUMNRING`).
    pub level_id: String,
    /// Whether the match contributes to player ratings.
    ///
    /// A match is rated if it concluded normally, or if it was cancelled
    /// after at least 30 seconds of play.
    ///
    /// Disconnecting from a match after 30 seconds will mean you take full
    /// penalties for losing the match.
    pub rated: bool,
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
#[derive(Clone, Debug, Deref, Deserialize, Serialize, ToSchema)]
pub struct Participant {
    /// The name of the player.
    pub name: String,
    /// The team they are on.
    pub team: PlayerTeam,
    /// The player's finish time, if they finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_time: Option<i32>,
    /// The player's score.
    ///
    /// For duels, this is how many checkpoints the player crossed.
    pub score: i32,
    /// If the player no contest'd.
    #[serde(default)]
    pub no_contest: bool,
    /// The player's skin.
    pub skin: Option<Skin>,
    /// The internal name of the player's skin color.
    pub skin_color: Option<String>,
    /// The player's DR at the time of match creation.
    ///
    /// If the player is provisional, this will be `null`. Will also be `null`
    /// for pre-season matches.
    pub dr: Option<Option<f32>>,
    /// The change in the player's DR once the match concluded.
    ///
    /// If the player is provisonal, this will be `null`.
    pub dr_delta: Option<Option<f32>>,
    /// The item usage of the player in the match.
    pub roulette: Vec<ItemUsage>,
    /// The user participating.
    #[deref]
    pub user: User,
}

/// A description for a single item pull.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ItemUsage {
    /// The base item this struct represents.
    pub item: KartItem,
    /// The size of the stack when this was pulled from the roulette.
    pub stack: usize,
    /// How many times the item was pulled from a roulette.
    pub count: usize,
}

/// A kart item.
#[derive(Clone, Debug, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KartItem {
    Sneaker,
    RocketSneaker,
    Invincibility,
    Banana,
    Eggman,
    Orbinaut,
    Jawz,
    /// Misleadingly listed as `KITEM_MINE` in the source code.
    ProximityMine,
    Landmine,
    Ballhog,
    Spb,
    Grow,
    Shrink,
    LightningShield,
    BubbleShield,
    FlameShield,
    Hyudoro,
    PogoSpring,
    SuperRing,
    KitchenSink,
    DropTarget,
    GardenTop,
    Gachabom,
    StoneShoe,
    Toxomister,
}

impl KartItem {
    /// The name of the item, for storing in the database.
    pub fn name(&self) -> &'static str {
        match self {
            KartItem::Sneaker => "sneaker",
            KartItem::RocketSneaker => "rocket_sneaker",
            KartItem::Invincibility => "invincibility",
            KartItem::Banana => "banana",
            KartItem::Eggman => "eggman",
            KartItem::Orbinaut => "orbinaut",
            KartItem::Jawz => "jawz",
            KartItem::ProximityMine => "proximity_mine",
            KartItem::Landmine => "landmine",
            KartItem::Ballhog => "ballhog",
            KartItem::Spb => "spb",
            KartItem::Grow => "grow",
            KartItem::Shrink => "shrink",
            KartItem::LightningShield => "lightning_shield",
            KartItem::BubbleShield => "bubble_shield",
            KartItem::FlameShield => "flame_shield",
            KartItem::Hyudoro => "hyudoro",
            KartItem::PogoSpring => "pogo_spring",
            KartItem::SuperRing => "super_ring",
            KartItem::KitchenSink => "kitchen_sink",
            KartItem::DropTarget => "drop_target",
            KartItem::GardenTop => "garden_top",
            KartItem::Gachabom => "gachabom",
            KartItem::StoneShoe => "stone_shoe",
            KartItem::Toxomister => "toxomister",
        }
    }
}

impl Display for KartItem {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for KartItem {
    type Err = InvalidKartItem;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sneaker" => Ok(KartItem::Sneaker),
            "rocket_sneaker" => Ok(KartItem::RocketSneaker),
            "invincibility" => Ok(KartItem::Invincibility),
            "banana" => Ok(KartItem::Banana),
            "eggman" => Ok(KartItem::Eggman),
            "orbinaut" => Ok(KartItem::Orbinaut),
            "jawz" => Ok(KartItem::Jawz),
            "proximity_mine" => Ok(KartItem::ProximityMine),
            "landmine" => Ok(KartItem::Landmine),
            "ballhog" => Ok(KartItem::Ballhog),
            "spb" => Ok(KartItem::Spb),
            "grow" => Ok(KartItem::Grow),
            "shrink" => Ok(KartItem::Shrink),
            "lightning_shield" => Ok(KartItem::LightningShield),
            "bubble_shield" => Ok(KartItem::BubbleShield),
            "flame_shield" => Ok(KartItem::FlameShield),
            "hyudoro" => Ok(KartItem::Hyudoro),
            "pogo_spring" => Ok(KartItem::PogoSpring),
            "super_ring" => Ok(KartItem::SuperRing),
            "kitchen_sink" => Ok(KartItem::KitchenSink),
            "drop_target" => Ok(KartItem::DropTarget),
            "garden_top" => Ok(KartItem::GardenTop),
            "gachabom" => Ok(KartItem::Gachabom),
            "stone_shoe" => Ok(KartItem::StoneShoe),
            "toxomister" => Ok(KartItem::Toxomister),
            _ => Err(InvalidKartItem(s.to_string())),
        }
    }
}

impl TryFrom<String> for KartItem {
    type Error = InvalidKartItem;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.as_str().parse()
    }
}

impl<'de> Deserialize<'de> for KartItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)
            .and_then(|s| s.parse::<KartItem>().map_err(D::Error::custom))
    }
}

impl Serialize for KartItem {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.name().serialize(serializer)
    }
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
    ToSchema,
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
    ToSchema,
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
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct BattlePoint {
    /// The ID of the battle.
    pub id: String,
    /// The name of the level the battle took place on.
    pub level_name: String,
    /// The level's internal identifier.
    pub level_id: String,
    /// The margin score of the battle.
    pub margin_score: Option<i32>,
    /// The statistics of the battle.
    #[serde(flatten)]
    #[schema(inline)]
    pub statistics: BattleStatistics,
}

/// The statistics of a battle.
///
/// A single battle can be represented as a single point in n-dimensional
/// space.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct BattleStatistics {
    /// The average MMR of the match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_mmr: Option<f32>,
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

/// A failure when converting a `str` to [`KartItem`].
#[derive(Debug, Display)]
#[display("unknown item: {_0}")]
pub struct InvalidKartItem(pub String);

impl std::error::Error for InvalidKartItem {}
