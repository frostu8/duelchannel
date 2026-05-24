//! Client error types.

use derive_more::{Display, Error, From};

use duelchannel_model::ApiError;
use reqwest::header::InvalidHeaderValue;

/// An error.
#[derive(Debug, Display, Error, From)]
pub enum Error {
    /// An error occured in the reqwest client.
    #[display("{_0}")]
    Reqwest(reqwest::Error),
    /// An error for headers.
    #[display("{_0}")]
    Header(InvalidHeaderValue),
    /// An API error occured.
    #[display("{_0}")]
    Api(ApiError),
}
