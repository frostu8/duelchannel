//! Application interface and state.

use std::sync::Arc;

use derive_more::{AsRef, Deref};

use opendal::Operator;

use sqlx::SqlitePool;

use crate::{config::Config, schema::user::mmr};

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

/// The rating model.
#[derive(Clone, Debug, Deref, AsRef)]
pub struct Model<T> {
    #[deref]
    inner: T,
}

impl<T> Model<T> {
    /// Creates a new `Model`.
    pub fn new(inner: T) -> Model<T> {
        Model { inner }
    }
}

pub trait ModelOrUnrated: 'static {
    type Model: mmr::Model + Send + Sync + 'static;

    /// Gets a MMR model.
    fn model<'a>(&'a self) -> Option<&'a Self::Model>;
}

impl<T> ModelOrUnrated for T
where
    T: mmr::Model + Send + Sync + 'static,
{
    type Model = T;

    fn model<'a>(&'a self) -> Option<&'a <Self as ModelOrUnrated>::Model> {
        Some(self)
    }
}

/// The unrated model.
///
/// This actually represents the absence of a model.
#[derive(Clone, Debug, Default)]
pub struct Unrated;

impl ModelOrUnrated for Unrated {
    type Model = !;

    fn model<'a>(&'a self) -> Option<&'a Self::Model> {
        None
    }
}
