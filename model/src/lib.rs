//! API model representations.

pub mod battle;
pub mod chat;
pub mod error;
pub mod profile;
pub mod request;
pub mod response;
pub mod server;
pub mod user;

pub use battle::Battle;
pub use error::ApiError;
pub use profile::{Profile, Rrid};
pub use user::{CurrentUser, User};
