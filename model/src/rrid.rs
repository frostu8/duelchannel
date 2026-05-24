//! Ring Racers profile ID.

use std::{
    fmt::{self, Display, Formatter, LowerHex, UpperHex},
    str::FromStr,
};

use derive_more::{Deref, Display, Error};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// The length of an RRID public key.
pub const PUBKEYLENGTH: usize = 32;

/// The length of an encoded RRID public key.
pub const PUBKEYLENGTH_ENCODED: usize = PUBKEYLENGTH * 2;

/// Ring Racers profile ID.
///
/// This type has the property that its [`ToString`] implementation produces a
/// string that can recreate the original `Rrid` using [`FromStr`].
#[derive(Clone, Debug, Deref, PartialEq, Eq, Hash)]
pub struct Rrid([u8; PUBKEYLENGTH]);

impl Rrid {
    /// Creates a new `Rrid` from a binary public key.
    ///
    /// # Panics
    /// Panics if length of `buf` is not [`PUBKEYLENGTH`].
    pub fn new(bytes: impl AsRef<[u8]>) -> Rrid {
        Rrid::new_checked(bytes).unwrap()
    }

    /// Creates a new `Rrid` from a binary public key.
    pub fn new_checked(bytes: impl AsRef<[u8]>) -> Result<Rrid, InvalidPubkeyLength> {
        let bytes = bytes.as_ref();

        if bytes.len() == PUBKEYLENGTH {
            let mut buf = [0u8; PUBKEYLENGTH];
            (&mut buf).copy_from_slice(bytes);

            Ok(Rrid(buf))
        } else {
            Err(InvalidPubkeyLength(bytes.len()))
        }
    }

    /// Returns the inner bytes of the type.
    pub fn as_bytes(&self) -> &[u8] {
        self.as_ref()
    }

    fn write_base16(&self, f: &mut Formatter<'_>, config: base16::EncConfig) -> fmt::Result {
        let mut buf = [0u8; PUBKEYLENGTH_ENCODED];
        let len = base16::encode_config_slice(&self.0, config, &mut buf);

        // This should always write 32 ASCII characters. If not, something has
        // gone terribly wrong.
        let output = std::str::from_utf8(&buf[..len]).expect("valid utf8");
        assert_eq!(output.len(), PUBKEYLENGTH_ENCODED);

        f.write_str(output)
    }
}

impl Display for Rrid {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // Upper hex representation is default
        <Rrid as UpperHex>::fmt(self, f)
    }
}

impl UpperHex for Rrid {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.write_base16(f, base16::EncodeUpper)
    }
}

impl LowerHex for Rrid {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.write_base16(f, base16::EncodeLower)
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
        let mut buf = [0u8; 32];

        // Check string len
        if s.len() != PUBKEYLENGTH_ENCODED {
            return Err(RridParseError::InvalidLength { len: s.len() });
        }

        match base16::decode_slice(s, &mut buf) {
            Ok(_len) => {
                // Valid Rrid
                Ok(Rrid(buf))
            }
            Err(base16::DecodeError::InvalidByte { index, .. }) => {
                // Bad character
                Err(RridParseError::InvalidChar { valid_up_to: index })
            }
            // We already checked for bad length
            Err(_) => unreachable!(),
        }
    }
}

impl<'de> Deserialize<'de> for Rrid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let id = <&str>::deserialize(deserializer)?;
        id.parse::<Rrid>().map_err(D::Error::custom)
    }
}

impl AsRef<[u8]> for Rrid {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Serialize for Rrid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// An error for invalid pub key length.
#[derive(Debug, Error)]
pub struct InvalidPubkeyLength(#[error(not(source))] pub usize);

impl Display for InvalidPubkeyLength {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "buf len is {}, expected {}", self.0, PUBKEYLENGTH)
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
