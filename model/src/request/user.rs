//! User request bodies.

use serde::{Deserialize, Serialize};

use crate::Rrid;

/// A request to create a user with some profiles.
///
/// This also initializes the user's rating.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateUser {
    /// The display name of the user.
    ///
    /// This is typically the profile name of the user when they first log in.
    pub display_name: String,
    pub profiles: Vec<CreateUserProfile>,
}

/// A profile for [`CreateUser`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateUserProfile {
    /// The profile's public key.
    pub public_key: Rrid,
}
