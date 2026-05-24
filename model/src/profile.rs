//! Player model.

pub use crate::rrid::Rrid;

use serde::{Deserialize, Serialize};

/// A profile on the Ring Racers server.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct Profile {
    /// The public rrid of the profile.
    ///
    /// The base16 encoded public key of the player, which is a 64-character
    /// string.
    pub public_key: Rrid,
}

/// A character a player has selected.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Skin {
    /// The internal name of the character.
    pub name: String,
    /// The human-readable name of the character.
    ///
    /// In Ring Racers, this is stored with underscores for spaces, but in the
    /// API these are printed as they appear in-game.
    pub real_name: String,
    /// The speed of the character.
    pub kart_speed: i32,
    /// The weight of the character.
    pub kart_weight: i32,
}
