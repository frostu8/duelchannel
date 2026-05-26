//! User structs and utilities.

pub mod mmr;

use chrono::{DateTime, Utc};

use duelchannel_model::{CurrentUser, Profile, Rrid, User, user::UserFlags};
use rand::{Rng, SeedableRng, distr::Alphanumeric};

use crate::error::{Error, ErrorKind};

use sqlx::{FromRow, SqliteConnection};

const MAX_INSERT_ATTEMPTS: usize = 5;

/// A user schema.
#[derive(Clone, FromRow)]
pub struct UserRow {
    pub id: i32,
    pub short_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    #[sqlx(try_from = "i32")]
    pub flags: UserFlags,
    pub ordinal: Option<i32>,
    pub inserted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<UserRow> for CurrentUser {
    fn from(value: UserRow) -> Self {
        CurrentUser { user: value.into() }
    }
}

impl From<&UserRow> for CurrentUser {
    fn from(value: &UserRow) -> Self {
        value.clone().into()
    }
}

impl From<UserRow> for User {
    fn from(value: UserRow) -> Self {
        User {
            id: value.short_id,
            display_name: value.display_name,
            avatar_url: value.avatar_url,
            mmr: value.ordinal,
            flags: value.flags,
            profiles: None,
        }
    }
}

impl From<&UserRow> for User {
    fn from(value: &UserRow) -> Self {
        value.clone().into()
    }
}

/// A builder for a user.
#[derive(Debug)]
pub struct UserBuilder {
    display_name: String,
    avatar_url: Option<String>,
    flags: UserFlags,
}

impl UserBuilder {
    /// Creates a new `UserBuilder`.
    pub fn new(display_name: impl Into<String>) -> UserBuilder {
        UserBuilder {
            display_name: display_name.into(),
            avatar_url: None,
            flags: UserFlags::empty(),
        }
    }

    /// Sets the avatar url.
    pub fn avatar_url(self, avatar_url: impl Into<Option<String>>) -> UserBuilder {
        UserBuilder {
            avatar_url: avatar_url.into(),
            ..self
        }
    }

    /// Sets the new user's flags.
    pub fn flags(self, flags: UserFlags) -> UserBuilder {
        UserBuilder { flags, ..self }
    }

    /// Creates the user.
    pub async fn create(self, conn: &mut SqliteConnection) -> Result<UserRow, Error> {
        let mut rng = rand::rngs::StdRng::from_os_rng();
        self.create_with(conn, &mut rng).await
    }

    /// Creates the user with a given PRNG.
    pub async fn create_with<R>(
        self,
        conn: &mut SqliteConnection,
        rng: &mut R,
    ) -> Result<UserRow, Error>
    where
        R: Rng,
    {
        let now = Utc::now();

        // this is a new player
        let mut inserted_user = None::<UserRow>;

        for _ in 0..MAX_INSERT_ATTEMPTS {
            // generate a short id
            let short_id = rng
                .sample_iter(Alphanumeric)
                .take(6)
                .map(char::from)
                .map(|c| char::to_ascii_uppercase(&c))
                .collect::<String>();

            // try to insert with short_id
            let result = sqlx::query_as::<_, UserRow>(
                r#"
                INSERT INTO user
                    (
                        inserted_at,
                        updated_at,
                        short_id,
                        display_name,
                        flags,
                        avatar_url
                    )
                VALUES ($1, $1, $2, $3, $4, $5)
                RETURNING id, short_id, display_name, avatar_url, flags, ordinal, inserted_at, updated_at
                "#,
            )
            .bind(now)
            .bind(&short_id)
            .bind(&self.display_name)
            .bind(i32::from(self.flags))
            .bind(&self.avatar_url)
            .fetch_one(&mut *conn)
            .await;

            match result {
                Ok(user) => {
                    inserted_user = Some(user);
                    break;
                }
                Err(err) => {
                    if let Some(db_err) = err.as_database_error() {
                        // if this is a unique violation, simply try again
                        if db_err.is_unique_violation() {
                            tracing::debug!("unique key {} failed, regenerating", short_id);
                        } else {
                            return Err(err.into());
                        }
                    } else {
                        return Err(err.into());
                    }
                }
            }
        }

        inserted_user.ok_or_else(|| ErrorKind::OutOfIds.into())
    }
}

/// Gets a user from the database by their ID.
pub async fn get_user(id: i32, conn: &mut SqliteConnection) -> Result<Option<UserRow>, Error> {
    sqlx::query_as::<_, UserRow>(
        r#"
        SELECT *
        FROM user
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(Error::new)
}

/// Gets a user from the database by their short ID.
pub async fn get_user_by_short_id(
    short_id: &str,
    conn: &mut SqliteConnection,
) -> Result<Option<UserRow>, Error> {
    sqlx::query_as::<_, UserRow>(
        r#"
        SELECT *
        FROM user
        WHERE short_id = $1
        "#,
    )
    .bind(short_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(Error::new)
}

/// Gets a user from the database by a profile public key
pub async fn get_user_by_public_key(
    public_key: &Rrid,
    conn: &mut SqliteConnection,
) -> Result<Option<UserRow>, Error> {
    sqlx::query_as::<_, UserRow>(
        r#"
        SELECT u.*
        FROM user u, profile pr
        WHERE
            pr.public_key = $1
            AND u.id = pr.parent_id
        "#,
    )
    .bind(public_key.as_bytes())
    .fetch_optional(&mut *conn)
    .await
    .map_err(Error::new)
}

#[derive(FromRow)]
struct ProfileRow {
    #[sqlx(try_from = "String")]
    pub public_key: Rrid,
}

/// Preloads a user with their profiles.
pub async fn preload_profiles(user: &mut User, conn: &mut SqliteConnection) -> Result<(), Error> {
    let profiles = sqlx::query_as::<_, ProfileRow>(
        r#"
        SELECT p.*
        FROM profile p, user u
        WHERE
            p.parent_id = u.id
            AND u.short_id = $1
        "#,
    )
    .bind(&user.id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|p| Profile {
        public_key: p.public_key,
    })
    .collect::<Vec<_>>();

    user.profiles = Some(profiles);
    Ok(())
}
