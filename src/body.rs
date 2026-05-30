//! Provides body extractors.

use axum::{
    RequestExt as _,
    extract::{FromRequest, Request},
    response::{IntoResponse, Response},
};

use axum_valid::HasValidate;
use derive_more::Deref;

use http::header;
use serde::de::DeserializeOwned;

use crate::error::{Error, ErrorKind};

/// Selective body extractor.
///
/// The `duelchannel` API can accept both JSON and urlencoded bodies.
#[derive(Deref)]
pub struct Payload<T>(pub T);

impl<S, T> FromRequest<S> for Payload<T>
where
    T: DeserializeOwned + 'static,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        // switch on content type
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ErrorKind::MissingContentType)?;

        match content_type {
            "application/x-www-form-urlencoded" => {
                let Form(form) = req.extract_with_state::<Form<T>, _, _>(state).await?;
                Ok(Payload(form))
            }
            "application/json" => {
                let Json(json) = req.extract_with_state::<Json<T>, _, _>(state).await?;
                Ok(Payload(json))
            }
            mime => Err(ErrorKind::UnsupportedContentType(mime.to_owned()).into()),
        }
    }
}

impl<T> HasValidate for Payload<T> {
    type Validate = T;

    fn get_validate(&self) -> &Self::Validate {
        &self.0
    }
}

/// App Form extractor and responder.
#[derive(Deref, FromRequest)]
#[from_request(via(axum::Form), rejection(Error))]
pub struct Form<T>(pub T);

impl<T> HasValidate for Form<T> {
    type Validate = T;

    fn get_validate(&self) -> &Self::Validate {
        &self.0
    }
}

impl<T> IntoResponse for Form<T>
where
    axum::Form<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        axum::Form(self.0).into_response()
    }
}

/// App JSON extractor and responder.
#[derive(Deref, FromRequest)]
#[from_request(via(axum::Json), rejection(Error))]
pub struct Json<T>(pub T);

impl<T> HasValidate for Json<T> {
    type Validate = T;

    fn get_validate(&self) -> &Self::Validate {
        &self.0
    }
}

impl<T> IntoResponse for Json<T>
where
    axum::Json<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}
