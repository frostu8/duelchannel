//! Client authentication.
//!
//! There are two methods to authenticating to the API:
//! * [`oauth2`]  
//!   This is the flow for humans through the main portal. This allows a user
//!   to edit their settings and view personal information about themselves,
//!   but otherwise does not allow mutations.
//! * [`api_key`]  
//!   This flow is meant for game servers to submit results of matches that
//!   play out on the game server.

pub mod api_key;
pub mod oauth2;
