//! User structs and utilities.

pub mod mmr;

use chrono::{DateTime, Utc};

use duelchannel_model::{CurrentUser, Profile, Rrid, User, user::UserFlags};

use crate::{error::Error, short_id};

use sqlx::{FromRow, SqliteConnection};

/// A user entity.
#[derive(Clone, Debug, FromRow)]
pub struct UserEntity {
    pub id: i32,
    pub short_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    #[sqlx(try_from = "i32")]
    pub flags: UserFlags,
    pub ordinal: Option<i32>,
    pub hide_rating: bool,
    pub inserted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[sqlx(skip)]
    pub profiles: Option<Vec<ProfileEntity>>,
}

impl UserEntity {
    /// Preloads a user with their profiles.
    pub async fn preload_profiles(
        &mut self,
        conn: &mut SqliteConnection,
    ) -> Result<&[ProfileEntity], Error> {
        let profiles = sqlx::query_as::<_, ProfileEntity>(
            r#"
            SELECT *
            FROM profile
            WHERE p.parent_id = $1
            "#,
        )
        .bind(self.id)
        .fetch_all(&mut *conn)
        .await?;

        self.profiles = Some(profiles);
        Ok(self.profiles.as_ref().unwrap())
    }
}

impl From<UserEntity> for CurrentUser {
    fn from(value: UserEntity) -> Self {
        CurrentUser { user: value.into() }
    }
}

impl From<UserEntity> for User {
    fn from(value: UserEntity) -> Self {
        let dr = match value.ordinal {
            Some(dr) if !value.hide_rating => Some(Some(dr)),
            Some(_dr) => Some(None),
            None => None,
        };

        User {
            id: value.short_id,
            display_name: value.display_name,
            avatar_url: value.avatar_url,
            dr,
            flags: value.flags,
            profiles: value
                .profiles
                .map(|list| list.into_iter().map(Profile::from).collect()),
        }
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
    pub async fn create(self, conn: &mut SqliteConnection) -> Result<UserEntity, Error> {
        // get new allocator
        let mut allocator = short_id::allocate();

        let now = Utc::now();
        let display_name = self.display_name;
        let avatar_url = self.avatar_url;
        let flags = self.flags;

        allocator
            .insert(conn, |short_id, conn| {
                let display_name = display_name.clone();
                let avatar_url = avatar_url.clone();
                Box::pin(async move {
                    sqlx::query_as::<_, UserEntity>(
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
                        RETURNING id, short_id, display_name, avatar_url, flags, ordinal, hide_rating, inserted_at, updated_at
                        "#,
                    )
                    .bind(now)
                    .bind(short_id)
                    .bind(&display_name)
                    .bind(i32::from(flags))
                    .bind(&avatar_url)
                    .fetch_one(&mut *conn)
                    .await
                })
            })
            .await
    }
}

/// Gets a user from the database by their ID.
pub async fn get_user(id: i32, conn: &mut SqliteConnection) -> Result<Option<UserEntity>, Error> {
    sqlx::query_as::<_, UserEntity>(
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
) -> Result<Option<UserEntity>, Error> {
    sqlx::query_as::<_, UserEntity>(
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

/// Gets a user from the database by a profile public key.
pub async fn get_user_by_public_key(
    public_key: &Rrid,
    conn: &mut SqliteConnection,
) -> Result<Option<UserEntity>, Error> {
    sqlx::query_as::<_, UserEntity>(
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

/// A raw entity for profiles.
#[derive(Clone, Debug, FromRow)]
pub struct ProfileEntity {
    pub id: i32,
    pub parent_id: i32,
    #[sqlx(try_from = "String")]
    pub public_key: Rrid,
}

impl From<ProfileEntity> for Profile {
    fn from(value: ProfileEntity) -> Self {
        Profile {
            public_key: value.public_key,
        }
    }
}
