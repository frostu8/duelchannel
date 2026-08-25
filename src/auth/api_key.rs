//! API key for subscribed servers.

use axum::extract::{FromRef, FromRequestParts};

use http::{header::HeaderName, request::Parts};
use sqlx::FromRow;

use crate::{
    app::AppState,
    error::{Error, ErrorKind},
};

use sha2::{Digest as _, Sha256};

use base16::encode_upper;

use rand::{
    Rng,
    distr::{Alphanumeric, SampleString},
};

pub const X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");

pub const API_KEY_LENGTH: usize = 64;

/// API key authentication.
#[derive(Clone, Debug)]
pub struct ServerAuthentication {
    /// The id of the server in the database.
    pub id: i32,
    /// The canonical name of the server.
    pub server_name: String,
}

#[derive(FromRow)]
struct ServerQuery {
    pub id: i32,
    pub server_name: String,
}

impl<S> FromRequestParts<S> for ServerAuthentication
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // if the result was cached, simply return the cached value
        if let Some(auth) = parts.extensions.get::<ServerAuthentication>() {
            return Ok(auth.clone());
        }

        let key = parts
            .headers
            .get(X_API_KEY)
            .and_then(|s| s.to_str().ok())
            .map(|s| s.trim());

        if let Some(key) = key {
            let state = AppState::from_ref(state);

            // hash token
            let hash = hash_api_key(key);
            let server = sqlx::query_as::<_, ServerQuery>(
                r#"
                SELECT
                    s.id,
                    s.server_name
                FROM
                    server s
                WHERE
                    key_hash = $1
                "#,
            )
            .bind(hash)
            .fetch_optional(&state.db)
            .await?;

            match server {
                Some(ServerQuery { id, server_name }) => {
                    let auth = ServerAuthentication { id, server_name };

                    // cache toe xtensions
                    parts.extensions.insert(auth.clone());

                    Ok(auth)
                }
                // api key matches nothing
                None => Err(ErrorKind::ApiKeyBadCredentials.into()),
            }
        } else {
            Err(ErrorKind::ApiKeyUnauthenticated.into())
        }
    }
}

/// Generates a new API key.
pub fn generate_api_key() -> String {
    generate_api_key_with(&mut rand::rng())
}

/// Generates a new API key.
pub fn generate_api_key_with<R>(rng: &mut R) -> String
where
    R: Rng,
{
    Alphanumeric::default().sample_string(rng, API_KEY_LENGTH)
}

/// Hashes an API key.
pub fn hash_api_key(key: impl AsRef<str>) -> String {
    let mut hasher = Sha256::new();

    hasher.update(key.as_ref());

    let result = hasher.finalize();

    encode_upper(&result)
}
