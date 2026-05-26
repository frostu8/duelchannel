//! Chat crossposting module.

use serde::{Deserialize, Serialize};

use crate::Profile;

/// A chat message.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Message {
    /// The player that sent this message.
    pub profile: Profile,
    /// The name of the player that sent the message.
    pub name: String,
    /// The content of the player's message.
    pub content: String,
    /// When the message was created.
    pub created_at: String,
}
