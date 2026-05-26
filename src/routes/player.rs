//! Users endpoints.

use axum::{
    Extension,
    extract::{Path, State},
};
use chrono::Utc;
use duelchannel_model::{
    Profile, Rrid, User,
    request::user::CreateUser,
    user::{CurrentUser, UserFlags},
};
use garde::Validate;
use serde::Deserialize;

use crate::{
    app::{AppForm, AppGarde, AppJson, AppState, Model, Payload},
    auth::api_key::ServerAuthentication,
    error::{Error, ErrorKind},
    schema::user::{
        UserBuilder, UserRow, get_user_by_short_id,
        mmr::{self, init_rating},
        preload_profiles,
    },
    session::SessionUser,
};

/// A query for [`list`].
#[derive(Deserialize, Debug, Validate)]
#[serde(default)]
#[garde(context(AppState as state))]
pub struct ListUsersQuery {
    #[garde(range(min = 1, max = 50))]
    pub count: i32,
    #[garde(skip)]
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
pub async fn create<T>(
    _auth_guard: ServerAuthentication,
    State(state): State<AppState>,
    Extension(model): Extension<Model<T>>,
    Payload(request): Payload<CreateUser>,
) -> Result<AppJson<User>, Error>
where
    T: mmr::Model + Send + Sync + 'static,
{
    let now = Utc::now();

    let mut tx = state.db.begin().await?;

    // Create user based off specs
    let row = UserBuilder::new(request.display_name)
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

    // Initialize rating
    let rating = init_rating(row.id, &model, &mut *tx).await?;

    tx.commit().await?;

    Ok(AppJson(User {
        profiles: Some(profiles),
        mmr: Some(rating.ordinal() as i32),
        ..User::from(&row)
    }))
}

/// Lists all users.
pub async fn list(
    State(state): State<AppState>,
    AppGarde(AppForm(query)): AppGarde<AppForm<ListUsersQuery>>,
) -> Result<AppJson<Vec<User>>, Error> {
    let mut conn = state.db.acquire().await?;

    let users = sqlx::query_as::<_, UserRow>(
        r#"
        SELECT *
        FROM user u, profile p
        WHERE
            p.parent_id = u.id
            AND ($2 IS NULL OR p.public_key = $2)
        ORDER BY ordinal DESC
        LIMIT $1
        "#,
    )
    .bind(query.count)
    .bind(query.public_key.as_ref().map(|s| s.as_bytes()))
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|u| User::from(u))
    .collect::<Vec<_>>();

    Ok(AppJson(users))
}

/// Shows the currently authenticated user's details.
pub async fn show_self(
    mut user: SessionUser,
    State(state): State<AppState>,
) -> Result<AppJson<CurrentUser>, Error> {
    let mut conn = state.db.acquire().await?;

    // The authenticated user can see their profiles
    preload_profiles(&mut user, &mut *conn).await?;
    Ok(AppJson(user.into_inner()))
}

/// Shows information about a specific user.
pub async fn show(
    Path((short_id,)): Path<(String,)>,
    State(state): State<AppState>,
) -> Result<AppJson<User>, Error> {
    let mut conn = state.db.acquire().await?;
    match get_user_by_short_id(&short_id, &mut *conn).await? {
        Some(user) => Ok(AppJson(User::from(user))),
        None => Err(Error::not_found(format!(
            "user w/ id {} not found",
            short_id
        ))),
    }
}
