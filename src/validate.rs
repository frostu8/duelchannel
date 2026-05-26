//! Body validators.

use axum::{
    RequestExt as _, RequestPartsExt as _,
    extract::{FromRef, FromRequest, FromRequestParts, Request},
    response::{IntoResponse, Response},
};

use axum_valid::{GardeRejection, HasValidate};

use derive_more::Deref;

use garde::Validate;

use http::request::Parts;

use crate::error::{Error, ErrorKind};

/// App Garde extrarctor.
#[derive(Deref)]
pub struct Valid<T>(pub T);

impl<S, T> FromRequestParts<S> for Valid<T>
where
    S: Send + Sync,
    T: FromRequestParts<S> + HasValidate + 'static,
    Error: From<<T as FromRequestParts<S>>::Rejection>,
    <T as HasValidate>::Validate: Validate,
    <<T as HasValidate>::Validate as Validate>::Context: Send + Sync + FromRef<S>,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let valid = parts
            .extract_with_state::<axum_valid::Garde<T>, S>(state)
            .await;

        match valid {
            Ok(axum_valid::Garde(valid)) => Ok(Valid(valid)),
            Err(GardeRejection::Valid(garde)) => Err(ErrorKind::Garde(garde).into()),
            Err(GardeRejection::Inner(err)) => Err(err.into()),
        }
    }
}

impl<S, T> FromRequest<S> for Valid<T>
where
    S: Send + Sync,
    T: FromRequest<S> + HasValidate + 'static,
    Error: From<<T as FromRequest<S>>::Rejection>,
    <T as HasValidate>::Validate: Validate,
    <<T as HasValidate>::Validate as Validate>::Context: Send + Sync + FromRef<S>,
{
    type Rejection = Error;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let valid = request
            .extract_with_state::<axum_valid::Garde<T>, S, _>(state)
            .await;

        match valid {
            Ok(axum_valid::Garde(valid)) => Ok(Valid(valid)),
            Err(GardeRejection::Valid(garde)) => Err(ErrorKind::Garde(garde).into()),
            Err(GardeRejection::Inner(err)) => Err(err.into()),
        }
    }
}

impl<T> IntoResponse for Valid<T>
where
    T: IntoResponse,
{
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}
