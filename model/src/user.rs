//! User representations.

use derive_more::{Deref, DerefMut};

use serde::{Deserialize, Serialize};

use serde_with::{TryFromInto, serde_as};

use bytemuck::cast;

use crate::Profile;

/// The current user returned by `/users/~me`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, Deref, DerefMut)]
pub struct CurrentUser {
    #[serde(flatten)]
    #[deref]
    #[deref_mut]
    pub user: User,
}

/// A single user.
#[serde_as]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
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
    /// The user's MMR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mmr: Option<i32>,
    /// The user flags.
    #[serde_as(as = "TryFromInto<i32>")]
    pub flags: UserFlags,
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
        /// This user achieved 3000 MMR at some point.
        const CHALLENGER = 1 << 1;
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
