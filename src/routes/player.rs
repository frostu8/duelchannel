//! Users endpoints.

use axum::{
    Extension,
    extract::{Path, State},
};
use chrono::Utc;
use duelchannel_model::{
    ApiError, Profile, Rrid, User,
    request::user::CreateUser,
    user::{CurrentUser, UserFlags},
};
use garde::Validate;
use serde::Deserialize;

use utoipa::IntoParams;

use crate::{
    app::AppState,
    auth::api_key::ServerAuthentication,
    body::{Form, Json, Payload},
    entity::user::{build_user, get_user, mmr::RatingService},
    error::{Error, ErrorKind},
    session::SessionUser,
    validate::Valid,
};

/// A query for [`list`].
#[derive(Deserialize, Debug, Validate, IntoParams)]
#[serde(default)]
#[garde(context(AppState as state))]
pub struct ListUsersQuery {
    /// The maximum number of users to return.
    #[garde(range(min = 1, max = 50))]
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub count: i32,
    /// Filter users by profile public key.
    #[garde(skip)]
    #[param(value_type = String)]
    pub public_key: Option<Rrid>,
}

impl Default for ListUsersQuery {
    fn default() -> Self {
        ListUsersQuery {
            count: 20,
            public_key: None,
        }
    }
}

/// Creates a new user.
#[utoipa::path(
    post,
    path = "/players",
    tag = "player",
    request_body = CreateUser,
    responses(
        (status = 200, description = "The created user", body = User),
        (status = 400, description = "Invalid request body or profile already in use", body = ApiError),
        (status = 401, description = "Missing or invalid API key", body = ApiError),
        (status = 415, description = "Missing or unsupported request content type", body = ApiError),
    ),
    security(("apiKey" = [])),
)]
pub async fn create<T>(
    _auth_guard: ServerAuthentication,
    State(state): State<AppState>,
    Extension(model): Extension<T>,
    Payload(request): Payload<CreateUser>,
) -> Result<Json<User>, Error>
where
    T: RatingService,
{
    let now = Utc::now();

    let mut tx = state.db.begin().await?;

    // Create user based off specs
    let row = build_user(request.display_name)
        .flags(UserFlags::BETA_TESTER)
        .create(&mut *tx)
        .await?;

    // Add profiles
    let mut profiles = Vec::with_capacity(request.profiles.len());
    for profile in request.profiles {
        let res = sqlx::query(
            r#"
            INSERT INTO profile (inserted_at, updated_at, parent_id, public_key)
            VALUES ($1, $1, $2, $3)
            "#,
        )
        .bind(now)
        .bind(row.id)
        .bind(profile.public_key.as_bytes())
        .execute(&mut *tx)
        .await;

        match res {
            Ok(_) => {
                profiles.push(Profile {
                    public_key: profile.public_key,
                });
            }
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
                // The profile already exists!
                return Err(ErrorKind::ProfileInUse(profile.public_key).into());
            }
            Err(err) => return Err(err.into()),
        }
    }

    // Initialize rating if it's enabled
    let dr = match model.create_rating(row.id, &state.config, &mut *tx).await? {
        // (ordinal, matches_until_rated)
        Some((dr, 0)) => Some(Some(dr)),
        Some((_dr, _)) => Some(None),
        None => None,
    };

    tx.commit().await?;

    Ok(Json(User {
        profiles: Some(profiles),
        dr,
        ..User::try_from(row)?
    }))
}

/// Lists all users.
#[utoipa::path(
    get,
    path = "/players",
    tag = "player",
    params(ListUsersQuery),
    responses(
        (status = 200, description = "A list of users", body = Vec<User>),
        (status = 400, description = "Invalid query parameters", body = ApiError),
    ),
)]
pub async fn list<T>(
    State(state): State<AppState>,
    Extension(model): Extension<T>,
    Valid(Form(query)): Valid<Form<ListUsersQuery>>,
) -> Result<Json<Vec<User>>, Error>
where
    T: RatingService,
{
    let mut tx = state.db.begin().await?;

    let user_ids = sqlx::query_as::<_, (i32,)>(
        r#"
        SELECT u.id
        FROM user u, profile p
        WHERE
            p.parent_id = u.id
            AND ($2 IS NULL OR p.public_key = $2)
        ORDER BY
            matches_until_rated ASC,
            ordinal DESC
        LIMIT $1
        "#,
    )
    .bind(query.count)
    .bind(query.public_key.as_ref().map(|s| s.as_bytes()))
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(id,)| id)
    .collect::<Vec<_>>();

    model
        .update_cached_ratings(user_ids.as_slice(), &mut *tx)
        .await?;

    let mut out = Vec::with_capacity(user_ids.len());
    for user_id in user_ids {
        let mut user = get_user(user_id, &mut *tx).await?.expect("valid user");
        user.preload_statistics(&mut *tx).await?;

        out.push(User::try_from(user)?);
    }

    tx.commit().await?;

    Ok(Json(out))
}

/// Shows the currently authenticated user's details.
#[utoipa::path(
    get,
    path = "/players/~me",
    tag = "player",
    responses(
        (status = 200, description = "The authenticated user", body = CurrentUser),
        (status = 401, description = "Not authenticated", body = ApiError),
    ),
    security(("cookie" = [])),
)]
pub async fn show_self<T>(
    user: SessionUser,
    Extension(model): Extension<T>,
    State(state): State<AppState>,
) -> Result<Json<CurrentUser>, Error>
where
    T: RatingService,
{
    let mut tx = state.db.begin().await?;

    model.update_cached_ratings(&[user.id], &mut *tx).await?;

    let mut user = get_user(user.id, &mut *tx).await?.expect("valid_user");
    user.preload_statistics(&mut *tx).await?;
    // The authenticated user can see their profiles
    user.preload_profiles(&mut *tx).await?;

    tx.commit().await?;

    Ok(Json(user.try_into()?))
}

/// Shows information about a specific user.
#[utoipa::path(
    get,
    path = "/players/{short_id}",
    tag = "player",
    params(
        ("short_id" = String, Path, description = "The short ID of the user"),
    ),
    responses(
        (status = 200, description = "The user", body = User),
        (status = 404, description = "User not found", body = ApiError),
    ),
)]
pub async fn show<T>(
    Path((short_id,)): Path<(String,)>,
    Extension(model): Extension<T>,
    State(state): State<AppState>,
) -> Result<Json<User>, Error>
where
    T: RatingService,
{
    let mut tx = state.db.begin().await?;

    let user_id = sqlx::query_as::<_, (i32,)>(
        r#"
        SELECT id
        FROM user
        WHERE short_id = $1
        "#,
    )
    .bind(&short_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((user_id,)) = user_id else {
        return Err(Error::not_found(format!(
            "user w/ id {} not found",
            short_id
        )));
    };

    model.update_cached_ratings(&[user_id], &mut *tx).await?;

    let mut user = get_user(user_id, &mut *tx).await?.expect("valid_user");
    user.preload_statistics(&mut *tx).await?;
    // The authenticated user can see their profiles
    user.preload_profiles(&mut *tx).await?;

    tx.commit().await?;

    Ok(Json(user.try_into()?))
}
