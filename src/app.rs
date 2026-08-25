//! Application interface and state.

use std::sync::Arc;

use opendal::Operator;

use sqlx::SqlitePool;

use crate::config::Config;

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
