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
    entity::user::{UserQuery, build_user, get_user, mmr::RatingService},
    error::{Error, ErrorKind},
    session::SessionUser,
    validate::Valid,
};

/// A query for [`list`].
#[derive(Deserialize, Debug, Validate, IntoParams)]
#[serde(default)]
#[garde(context(AppState as state))]
#[into_params(parameter_in = Query)]
pub struct ListUsersFilters {
    /// The maximum number of users to return.
    #[garde(range(min = 1, max = 50))]
    #[param(minimum = 1, maximum = 50, default = 20)]
    pub count: i32,
    /// Filter users by profile public key.
    #[garde(skip)]
    #[param(value_type = String)]
    pub public_key: Option<Rrid>,
    /// Filter users that have a lower ordinal than `before`.
    #[garde(skip)]
    pub before: Option<f32>,
    /// Filter users that have a higher ordinal than `after`.
    #[garde(skip)]
    pub after: Option<f32>,
    /// A search term for users.
    #[garde(length(min = 1))]
    #[param(min_length = 1)]
    pub search: Option<String>,
}

impl Default for ListUsersFilters {
    fn default() -> Self {
        ListUsersFilters {
            count: 20,
            public_key: None,
            before: None,
            after: None,
            search: None,
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
    params(ListUsersFilters),
    responses(
        (status = 200, description = "A list of users", body = Vec<User>),
        (status = 400, description = "Invalid query parameters", body = ApiError),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    Valid(Form(filters)): Valid<Form<ListUsersFilters>>,
) -> Result<Json<Vec<User>>, Error> {
    let mut conn = state.db.acquire().await?;

    let mut query = UserQuery::new();

    if let Some(before) = filters.before {
        query.before(before);
    }
    if let Some(after) = filters.after {
        query.after(after);
    }
    if let Some(search) = filters.search {
        query.search(search);
    }
    if let Some(public_key) = filters.public_key {
        query.public_key(public_key);
    }

    // This is a bit too aggressive for updating ratings. This route should be
    // idempotent.
    // model
    //     .update_cached_ratings(user_ids.as_slice(), &mut *tx)
    //     .await?;

    let mut users = query.fetch(&mut conn).await?;
    for user in users.iter_mut() {
        user.preload_statistics(&mut conn).await?;
    }

    Ok(Json(
        users
            .into_iter()
            .map(|u| User::try_from(u).map_err(Error::from))
            .collect::<Result<Vec<_>, Error>>()?,
    ))
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
