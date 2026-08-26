//! Skill rating MMR models.
//!
//! Currently, this module provides two models:
//! * [`glicko2`]  
//!   A more traditional model for 1v1 formats.
//! * [`openskill`]  
//!   An alternative model.

pub mod glicko2;
pub mod openskill;

use std::fmt::Debug;

use derive_more::{Deref, DerefMut};

use chrono::TimeDelta;

use duelchannel_model::battle::BattleStatus;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::error::Error;

/// A rating model.
///
/// Defines the actual mechanics of a rating model. Implementing this trait
/// means to service consumers that the type provides a model for calculating
/// ratings.
///
/// Do *not* implement this trait with `unimplemented!()` stubs to communicate
/// that there is no model in use.
pub trait RatingModel: Send + Sync {
    /// The associated data type used to make the model function.
    ///
    /// For convenience, the data must be cloneable (though it doesn't have to
    /// be cheap).
    type Data: RatingModelData + Serialize + DeserializeOwned + Clone + Debug + Unpin + 'static;

    /// Initializes a new rating.
    fn create_rating(
        &self,
        player_id: i32,
    ) -> impl Future<Output = Result<Rating<Self::Data>, Error>> + Send + Sync;

    /// Rates a player's performance.
    ///
    /// This also passes a `period_elapsed` delta.
    fn rate(
        &self,
        rating: &Rating<Self::Data>,
        matchups: &[Matchup<Self::Data>],
        period_elapsed: f32,
    ) -> impl Future<Output = Result<Rating<Self::Data>, Error>> + Send + Sync;

    /// Gets the quality of a match, assuming each player is on their own team.
    fn quality(
        &self,
        players: &[Rating<Self::Data>],
    ) -> impl Future<Output = Result<f32, Error>> + Send + Sync;

    /// The time between rating periods.
    fn period(&self) -> TimeDelta;

    /// The number of consecutive idle rating periods a player may accrue
    /// before rating decay begins.
    fn decay_grace(&self) -> f32 {
        0.0
    }
}

impl RatingModel for ! {
    type Data = ();

    async fn create_rating(&self, _player_id: i32) -> Result<Rating<Self::Data>, Error> {
        *self
    }

    async fn rate(
        &self,
        _rating: &Rating<Self::Data>,
        _matchups: &[Matchup<Self::Data>],
        _period_elapsed: f32,
    ) -> Result<Rating<Self::Data>, Error> {
        *self
    }

    async fn quality(&self, _players: &[Rating<Self::Data>]) -> Result<f32, Error> {
        *self
    }

    fn period(&self) -> TimeDelta {
        *self
    }
}

pub trait RatingModelData: Send + Sync + Sized + 'static {
    /// The ordinal of the rating.
    fn ordinal(rating: &Rating<Self>) -> f32 {
        rating.rating - rating.deviation * 2.0
    }

    /// Whether or not the rating is provisional.
    ///
    /// Provisional ratings are hidden from users.
    fn is_provisional(_rating: &Rating<Self>) -> bool {
        false
    }
}

impl RatingModelData for () {}

/// A matchup between two players.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Matchup<T = ()> {
    /// The opponent of the player.
    pub opponent: Rating<T>,
    /// The status of the match that the player participated in.
    pub status: BattleStatus,
    /// The player's finish position.
    pub position: i32,
    /// The player's finish time.
    pub finish_time: i32,
    /// Whether the player NO CONTEST'd.
    pub no_contest: bool,
}

/// A single player rating.
///
/// The rating may also contain arbitrary info `T` for the relevant MMR system
/// to query.
#[derive(Clone, Debug, Deref, DerefMut, Deserialize, Serialize)]
pub struct Rating<T = ()> {
    /// The id of the player this is for.
    ///
    /// This should be a unique identifer across all users.
    pub user_id: i32,
    /// The player's actual rating.
    pub rating: f32,
    /// The rating deviation of the player.
    pub deviation: f32,
    /// Extra data for the rating system.
    #[deref]
    #[deref_mut]
    #[serde(flatten)]
    pub extra: T,
}

impl Rating<()> {
    /// Creates a new rating.
    pub fn new(user_id: i32, rating: f32, deviation: f32) -> Rating<()> {
        Rating {
            user_id,
            rating,
            deviation,
            extra: (),
        }
    }
}

impl<T> Rating<T> {
    /// Packs additional data from a RON-encoded string.
    pub fn decode<U>(self, ron_str: impl AsRef<str>) -> Result<Rating<U>, Error>
    where
        U: RatingModelData + DeserializeOwned + 'static,
    {
        Ok(Rating {
            user_id: self.user_id,
            rating: self.rating,
            deviation: self.deviation,
            extra: ron::from_str(ron_str.as_ref())
                .map_err(|error| error.code)
                .map_err(Error::new)?,
        })
    }
}

impl<T> Rating<T>
where
    T: Serialize,
{
    /// Extracts the additional data as a RON-encoded string.
    pub fn encode(&self) -> Result<String, Error> {
        ron::to_string(&self.extra).map_err(Error::new)
    }
}

impl<T> Rating<T>
where
    T: RatingModelData,
{
    /// The player's ordinal.
    ///
    /// This is what is actually displayed as their DR.
    pub fn ordinal(&self) -> f32 {
        T::ordinal(self)
    }

    /// Whether or not the rating is provisional.
    ///
    /// Provisional ratings are hidden from users.
    pub fn is_provisional(&self) -> bool {
        T::is_provisional(self)
    }
}
