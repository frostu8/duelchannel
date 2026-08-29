//! User representations.

use std::{
    fmt::{self, Display, Formatter},
    str::FromStr,
};

use derive_more::{Deref, DerefMut, Display};

use serde::{Deserialize, Serialize};

use serde_with::{DeserializeFromStr, SerializeDisplay, TryFromInto, serde_as};

use utoipa::ToSchema;

use bytemuck::cast;

use crate::Profile;

/// The current user returned by `/users/~me`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Deref, DerefMut, ToSchema)]
pub struct CurrentUser {
    #[serde(flatten)]
    #[schema(inline)]
    #[deref]
    #[deref_mut]
    pub user: User,
}

/// A single user.
#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, ToSchema)]
pub struct User {
    /// The ID of the user.
    ///
    /// This is a 6-digit alphanumeric string that uniquely identifies the
    /// user.
    pub id: String,
    /// The display name of the user.
    pub display_name: String,
    /// The URL of the user's avatar.
    pub avatar_url: Option<String>,
    /// The user's DR.
    ///
    /// If this field is absent, skill ratings have been disabled on the
    /// server. If this field is present but `null`, that means the user's DR
    /// is currently being calibrated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable)]
    pub dr: Option<Option<f32>>,
    /// The user's rank.
    ///
    /// See [`User::dr`] for rules on visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(nullable)]
    pub rank: Option<Option<Rank>>,
    /// How many more matches until the user's DR is displayed.
    ///
    /// If this field is absent, skill ratings have been disabled on the
    /// server. Otherwise, always present, even when zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches_until_rated: Option<u32>,
    /// The user flags.
    #[serde_as(as = "TryFromInto<i32>")]
    #[schema(value_type = i32)]
    pub flags: UserFlags,
    /// How many matches the user has played.
    pub matches_played: i32,
    /// The win/loss ratio of the user.
    ///
    /// If the user has not played any matches, this is `0.0`.
    pub win_ratio: f32,
    /// The user's profiles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profiles: Option<Vec<Profile>>,
}

bitflags::bitflags! {
    /// User flags.
    #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
    pub struct UserFlags: u32 {
        /// The user is an administrator.
        const ADMINISTRATOR = 1;
        /// This user helped beta test. Thanks!
        const BETA_TESTER = 1 << 1;
        /// This user achieved 3000 MMR at some point during the period.
        const BETA_CHALLENGER = 1 << 2;
    }
}

impl FromStr for UserFlags {
    type Err = UnknownFlag;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ADMINISTRATOR" => Ok(UserFlags::ADMINISTRATOR),
            "BETA_TESTER" => Ok(UserFlags::BETA_TESTER),
            "BETA_CHALLENGER" => Ok(UserFlags::BETA_CHALLENGER),
            _ => Err(UnknownFlag(s.to_string())),
        }
    }
}

impl From<i32> for UserFlags {
    fn from(value: i32) -> Self {
        let value: u32 = cast(value);
        UserFlags::from_bits_truncate(value)
    }
}

impl From<UserFlags> for i32 {
    fn from(value: UserFlags) -> Self {
        cast(value.bits())
    }
}

/// A user's rank.
///
/// The duelchannel API supports a select number of ranks.
///
/// Below are the target percentiles of each rank.
#[derive(
    Clone,
    Copy,
    Debug,
    DeserializeFromStr,
    SerializeDisplay,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    ToSchema,
)]
pub enum Rank {
    /// C rank - 60-100%
    C,
    /// B rank - 35-60%
    B,
    /// A rank - 20-35%
    A,
    /// AA rank - 10-20%
    AA,
    /// S rank - 5-10%
    S,
    /// SS rank - 1-5%
    SS,
    /// X rank - top 1%
    X,
}

impl Rank {
    /// The name of each rank, as it appears over the wire.
    pub fn name(&self) -> &'static str {
        match self {
            Rank::X => "x",
            Rank::SS => "ss",
            Rank::S => "s",
            Rank::AA => "aa",
            Rank::A => "a",
            Rank::B => "b",
            Rank::C => "c",
        }
    }
}

impl Display for Rank {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Rank {
    type Err = UnknownRank;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "x" => Ok(Rank::X),
            "ss" => Ok(Rank::SS),
            "s" => Ok(Rank::S),
            "aa" => Ok(Rank::AA),
            "a" => Ok(Rank::A),
            "b" => Ok(Rank::B),
            "c" => Ok(Rank::C),
            s => Err(UnknownRank(s.to_string())),
        }
    }
}

impl TryFrom<String> for Rank {
    type Error = UnknownRank;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse::<Rank>()
    }
}

/// An error for unknown flags.
#[derive(Debug, Display)]
#[display("unknown flag: \"{_0}\"")]
pub struct UnknownFlag(String);

impl std::error::Error for UnknownFlag {}

/// A rank name was not recognized.
#[derive(Debug, Display)]
#[display("unknown rank: \"{_0}\"")]
pub struct UnknownRank(String);

impl std::error::Error for UnknownRank {}
