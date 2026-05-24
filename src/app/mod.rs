//! Application interface and state.

use std::{any::Any, sync::Arc};

use axum_valid::{Garde, GardeRejection, HasValidate};

use axum::{
    Form, Json, RequestExt as _, RequestPartsExt as _,
    extract::{FromRef, FromRequest, FromRequestParts, Request},
    response::{IntoResponse, Response},
};

use derive_more::{AsRef, Deref};

use garde::Validate;
use http::{header, request::Parts};

use opendal::Operator;
use serde::de::DeserializeOwned;

use sqlx::SqlitePool;

use crate::{config::Config, schema::user::mmr};

use crate::error::{Error, ErrorKind};

/// Shared app state.
///
/// Cheaply cloneable.
#[derive(Clone, Debug)]
pub struct AppState {
    /// The database connection pool.
    pub db: SqlitePool,
    /// The object storage.
    pub object_storage: Operator,
    /// Server config.
    ///
    /// May be missing secrets as they are taken at initialization.
    pub config: Arc<Config>,
}

/// Rating model.
#[derive(Clone, Debug, Deref, AsRef)]
pub struct Model<T> {
    #[deref]
    inner: T,
}

impl<T> mmr::Model for Model<T>
where
    T: mmr::Model,
{
    type Data = T::Data;

    async fn create_rating(&self, player_id: i32) -> Result<mmr::Rating<Self::Data>, Error> {
        self.inner.create_rating(player_id).await
    }

    async fn rate(
        &self,
        rating: &mmr::RatingRecord<Self::Data>,
        matchups: &[mmr::Matchup<Self::Data>],
        period_elapsed: f32,
    ) -> Result<mmr::Rating<Self::Data>, Error> {
        self.inner.rate(rating, matchups, period_elapsed).await
    }

    fn period(&self) -> chrono::TimeDelta {
        self.inner.period()
    }
}

impl<T> Model<T> {
    /// Creates a new rating model.
    pub fn new(model: T) -> Model<T> {
        Model { inner: model }
    }
}

impl<T> Model<T>
where
    T: 'static,
{
    /// Returns `true` if ratings are enabled.
    pub fn ratings_enabled(&self) -> bool {
        !(&self.inner as &dyn Any).is::<Unrated>()
    }
}

impl<T> From<T> for Model<T>
where
    T: mmr::Model,
{
    fn from(value: T) -> Self {
        Model { inner: value }
    }
}

/// Indicator for "no rating model."
///
/// This shouldn't be used as a [`Model`], as all methods panic, and is only
/// meant to be used in a handler extension.
///
/// [`Model`]: mmr::Model
#[derive(Clone, Copy, Debug)]
pub struct Unrated;

impl mmr::Model for Unrated {
    type Data = ();

    async fn create_rating(&self, _player_id: i32) -> Result<mmr::Rating<Self::Data>, Error> {
        unimplemented!()
    }

    async fn rate(
        &self,
        _rating: &mmr::RatingRecord<Self::Data>,
        _matchups: &[mmr::Matchup<Self::Data>],
        _period_elapsed: f32,
    ) -> Result<mmr::Rating<Self::Data>, Error> {
        unimplemented!()
    }

    fn period(&self) -> chrono::TimeDelta {
        unimplemented!()
    }
}

/// Selective body extractor.
///
/// The duel-channel API can accept both JSON and urlencoded bodies.
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
                let AppForm(form) = req.extract_with_state::<AppForm<T>, _, _>(state).await?;
                Ok(Payload(form))
            }
            "application/json" => {
                let AppJson(json) = req.extract_with_state::<AppJson<T>, _, _>(state).await?;
                Ok(Payload(json))
            }
            mime => Err(ErrorKind::UnsupportedContentType(mime.to_owned()).into()),
        }
    }
}

/// App Garde extrarctor.
#[derive(Deref)]
pub struct AppGarde<T>(pub T);

impl<S, T> FromRequestParts<S> for AppGarde<T>
where
    S: Send + Sync,
    T: FromRequestParts<S> + HasValidate + 'static,
    Error: From<<T as FromRequestParts<S>>::Rejection>,
    <T as HasValidate>::Validate: Validate,
    <<T as HasValidate>::Validate as Validate>::Context: Send + Sync + FromRef<S>,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let valid = parts.extract_with_state::<Garde<T>, S>(state).await;

        match valid {
            Ok(Garde(valid)) => Ok(AppGarde(valid)),
            Err(GardeRejection::Valid(garde)) => Err(ErrorKind::Garde(garde).into()),
            Err(GardeRejection::Inner(err)) => Err(err.into()),
        }
    }
}

impl<S, T> FromRequest<S> for AppGarde<T>
where
    S: Send + Sync,
    T: FromRequest<S> + HasValidate + 'static,
    Error: From<<T as FromRequest<S>>::Rejection>,
    <T as HasValidate>::Validate: Validate,
    <<T as HasValidate>::Validate as Validate>::Context: Send + Sync + FromRef<S>,
{
    type Rejection = Error;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let valid = request.extract_with_state::<Garde<T>, S, _>(state).await;

        match valid {
            Ok(Garde(valid)) => Ok(AppGarde(valid)),
            Err(GardeRejection::Valid(garde)) => Err(ErrorKind::Garde(garde).into()),
            Err(GardeRejection::Inner(err)) => Err(err.into()),
        }
    }
}

impl<T> IntoResponse for AppGarde<T>
where
    T: IntoResponse,
{
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

/// App Form extractor and responder.
#[derive(Deref, FromRequest)]
#[from_request(via(Form), rejection(Error))]
pub struct AppForm<T>(pub T);

impl<T> HasValidate for AppForm<T> {
    type Validate = T;

    fn get_validate(&self) -> &Self::Validate {
        &self.0
    }
}

impl<T> IntoResponse for AppForm<T>
where
    Form<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        Form(self.0).into_response()
    }
}

/// App JSON extractor and responder.
#[derive(Deref, FromRequest)]
#[from_request(via(Json), rejection(Error))]
pub struct AppJson<T>(pub T);

impl<T> HasValidate for AppJson<T> {
    type Validate = T;

    fn get_validate(&self) -> &Self::Validate {
        &self.0
    }
}

impl<T> IntoResponse for AppJson<T>
where
    Json<T>: IntoResponse,
{
    fn into_response(self) -> Response {
        Json(self.0).into_response()
    }
}
