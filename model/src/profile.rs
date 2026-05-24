//! Player model.

use std::str::FromStr;

use derive_more::{Deref, Display, Error};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, Unexpected},
};

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
    pub realname: String,
    /// The speed of the character.
    #[serde(rename = "s")]
    pub kartspeed: i32,
    /// The weight of the character.
    #[serde(rename = "w")]
    pub kartweight: i32,
}

/// Ring Racers ID.
#[derive(Clone, Debug, Deref, Display, PartialEq, Eq, Hash)]
pub struct Rrid(String);

impl Rrid {
    /// Creates a new, checked `Rrid`.
    pub fn new(s: impl AsRef<str>) -> Result<Rrid, RridParseError> {
        s.as_ref().parse()
    }

    /// Represents the Rrid as a string.
    pub fn as_str(&self) -> &str {
        <Self as AsRef<str>>::as_ref(self)
    }
}

impl TryFrom<String> for Rrid {
    type Error = RridParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<&str> for Rrid {
    type Error = RridParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl FromStr for Rrid {
    type Err = RridParseError;

    /// Creates a new Ring Racers ID from a checked string.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const ACCEPTED_CHARS: &[u8] = b"0123456789ABCDEFabcdef";

        if s.len() == 64 {
            let idx = s
                .as_bytes()
                .iter()
                .position(|ch| !ACCEPTED_CHARS.contains(ch));
            if let Some(idx) = idx {
                Err(RridParseError::InvalidChar { valid_up_to: idx })
            } else {
                Ok(Rrid(s.to_uppercase()))
            }
        } else {
            Err(RridParseError::InvalidLength { len: s.len() })
        }
    }
}

impl AsRef<str> for Rrid {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Rrid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let id = String::deserialize(deserializer)?;

        if id.len() == 64 {
            Ok(Rrid(id))
        } else {
            Err(D::Error::invalid_value(
                Unexpected::Str(&id),
                &"an rrid of length 64",
            ))
        }
    }
}

impl Serialize for Rrid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

/// An error for parsing RRIDs.
#[derive(Debug, Display, Error)]
pub enum RridParseError {
    /// The RRID was of invalid length.
    #[display("string was len {len}, expected len 64")]
    InvalidLength {
        #[error(not(source))]
        len: usize,
    },
    /// The RRID contained an invalid character.
    #[display("string contains invalid characters")]
    InvalidChar {
        #[error(not(source))]
        valid_up_to: usize,
    },
}
