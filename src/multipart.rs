//! Multipart form extractors.

use axum::{
    RequestExt,
    extract::{FromRequest, Request},
};

use derive_more::{Deref, DerefMut};

use crate::error::{Error, ErrorKind};

/// Multipart extractor and responder.
#[derive(Deref, DerefMut)]
pub struct Multipart(pub axum::extract::Multipart);

impl<S> FromRequest<S> for Multipart
where
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        req.extract_with_state::<axum::extract::Multipart, S, _>(state)
            .await
            .map(|m| Multipart(m))
            .map_err(ErrorKind::Multipart)
            .map_err(Error::from)
    }
}
