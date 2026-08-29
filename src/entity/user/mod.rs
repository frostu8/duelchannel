//! User structs and utilities.

pub mod mmr;

use std::cmp::max;

use chrono::{DateTime, Utc};

use derive_more::{Display, From};
use duelchannel_model::{
    CurrentUser, Profile, Rrid, User,
    user::{Rank, UnknownRank, UserFlags},
};
use sea_query::{Expr, ExprTrait as _, Iden, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder as _;

use crate::{config::Config, entity::MissingData, error::Error, mmr::RatingModel, short_id};

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
    pub ordinal: Option<f32>,
    pub hide_rating: bool,
    pub matches_until_rated: i32,
    pub rank: Option<String>,
    pub inserted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[sqlx(skip)]
    pub profiles: Option<Vec<ProfileEntity>>,
    #[sqlx(skip)]
    pub statistics: Option<UserStatistics>,
}

impl UserEntity {
    /// Fetches a user's statistics, and attaches it to the entity.
    pub async fn preload_statistics(
        &mut self,
        conn: &mut SqliteConnection,
    ) -> Result<&UserStatistics, Error> {
        self.statistics = Some(get_user_statistics(self.id, conn).await?);
        Ok(self.statistics.as_ref().unwrap())
    }

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

impl TryFrom<UserEntity> for CurrentUser {
    type Error = NormalizeError;

    fn try_from(value: UserEntity) -> Result<Self, Self::Error> {
        Ok(CurrentUser {
            user: value.try_into()?,
        })
    }
}

impl TryFrom<UserEntity> for User {
    type Error = NormalizeError;

    fn try_from(value: UserEntity) -> Result<Self, Self::Error> {
        // DR and rank are hidden while the player is still calibrating
        let dr = match value.ordinal {
            Some(dr) if value.matches_until_rated <= 0 => Some(Some(dr)),
            Some(_dr) => Some(None),
            None => None,
        };

        let rank = value.rank.map(|v| v.parse::<Rank>()).transpose()?;
        let rank = match value.ordinal {
            Some(_) if value.matches_until_rated <= 0 => Some(rank),
            Some(_) => Some(None),
            None => None,
        };

        let statistics = value.statistics.ok_or_else(|| MissingData {
            field_name: "statistics".into(),
        })?;
        let matches_played = statistics.wins + statistics.losses;

        Ok(User {
            id: value.short_id,
            display_name: value.display_name,
            avatar_url: value.avatar_url,
            dr,
            rank,
            matches_until_rated: match value.ordinal {
                Some(_) => Some(max(value.matches_until_rated, 0) as u32),
                None => None,
            },
            matches_played,
            win_ratio: if matches_played > 0 {
                statistics.wins as f32 / matches_played as f32
            } else {
                0.0
            },
            flags: value.flags,
            profiles: value
                .profiles
                .map(|list| list.into_iter().map(Profile::from).collect()),
        })
    }
}

/// User statistics.
#[derive(Clone, Debug, Default, FromRow)]
pub struct UserStatistics {
    /// The amount of wins a player has accrued.
    pub wins: i32,
    /// The amount of losses a player has accrued.
    pub losses: i32,
}

/// Gets statistics for a specific user.
pub async fn get_user_statistics(
    user_id: i32,
    conn: &mut SqliteConnection,
) -> Result<UserStatistics, Error> {
    sqlx::query_as::<_, UserStatistics>(
        r#"
        SELECT
            COUNT(p.id) FILTER (WHERE p.no_contest = false) AS wins,
            COUNT(p.id) FILTER (WHERE p.no_contest = true) AS losses
        FROM user u
        LEFT JOIN participant p ON p.user_id = u.id
        LEFT JOIN battle b ON p.match_id = b.id
        WHERE u.id = $1 AND b.status = 1
        GROUP BY u.id
        "#,
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await
    .map(|stats| stats.unwrap_or_default())
    .map_err(Error::from)
}

/// Creates a new [`UserBuilder`].
pub fn build_user(display_name: impl Into<String>) -> UserBuilder {
    UserBuilder::new(display_name)
}

/// A builder for a user.
#[derive(Debug)]
pub struct UserBuilder {
    display_name: String,
    avatar_url: Option<String>,
    flags: UserFlags,
    timestamp: DateTime<Utc>,
}

impl UserBuilder {
    /// Creates a new `UserBuilder`.
    pub fn new(display_name: impl Into<String>) -> UserBuilder {
        UserBuilder {
            display_name: display_name.into(),
            avatar_url: None,
            flags: UserFlags::empty(),
            timestamp: Utc::now(),
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

    /// Stamps the user at a specific time, instead of at builder creation.
    pub fn timestamp(self, timestamp: impl Into<DateTime<Utc>>) -> UserBuilder {
        UserBuilder {
            timestamp: timestamp.into(),
            ..self
        }
    }

    /// Creates the user.
    pub async fn create(self, conn: &mut SqliteConnection) -> Result<UserEntity, Error> {
        // get new allocator
        let mut allocator = short_id::allocate();

        let UserBuilder {
            display_name,
            avatar_url,
            flags,
            timestamp,
            ..
        } = self;

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
                        RETURNING id, short_id, display_name, avatar_url, flags, ordinal, hide_rating, matches_until_rated, rank, inserted_at, updated_at
                        "#,
                    )
                    .bind(timestamp)
                    .bind(short_id)
                    .bind(&display_name)
                    .bind(i32::from(flags))
                    .bind(&avatar_url)
                    .fetch_one(&mut *conn)
                    .await
                    .map(|u| UserEntity {
                            statistics: Some(UserStatistics::default()),
                            ..u
                        })
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

#[derive(Iden)]
enum Table {
    User,
}

/// Update ratings of all users passed to the function.
///
/// This *does* reassign a player's rank and grant them awards. Should be
/// called after a battle finishes. The caller is expected to know who won
/// each battle; outcomes feed rank promotions (on win) and demotions (on
/// loss).
pub async fn update_post_battle<T>(
    entries: &[(i32, bool)],
    model: &T,
    config: &Config,
    conn: &mut SqliteConnection,
) -> Result<(), Error>
where
    T: RatingModel,
{
    #[derive(Debug, FromRow)]
    struct UserRow {
        id: i32,
        #[sqlx(try_from = "i32")]
        flags: UserFlags,
        rank: Option<String>,
    }

    let user_ids = entries
        .iter()
        .map(|(user_id, _)| *user_id)
        .collect::<Vec<_>>();

    // Fetch players
    let (query, values) = Query::select()
        .column((Table::User, "id"))
        .column((Table::User, "flags"))
        .column((Table::User, "rank"))
        .from(Table::User)
        .and_where(Expr::col((Table::User, "id")).is_in(user_ids.iter().copied()))
        .build_sqlx(SqliteQueryBuilder);

    let players = sqlx::query_as_with::<_, UserRow, _>(sqlx::AssertSqlSafe(query), values)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .collect::<Vec<_>>();

    let ratings = mmr::update_ratings(&user_ids, model, &mut *conn).await?;

    // Grant awards, update rank
    for ((player, rating), (_, no_contest)) in players
        .into_iter()
        .zip(ratings)
        .zip(entries.iter().copied())
    {
        let mut flags = player.flags;
        let awards = config
            .awards
            .values()
            .filter(|award| award.threshold as f32 <= rating.ordinal())
            .filter(|award| !rating.is_provisional() || award.award_provisional);

        // Award these guys
        for award in awards {
            flags |= award.flag;
        }

        // Rank update; only move up on win or down on loss.
        // Players without a rank get a fresh one if they are ranked
        let old_rank = player
            .rank
            .map(|r| r.parse::<Rank>())
            .transpose()
            .map_err(|e| Error::new(e).with_message("failed to parse rank from db"))?;
        let new_rank = config.classify_rank(rating.ordinal());

        let rank_update = match (old_rank, new_rank) {
            (None, fresh) if rating.matches_until_rated == 0 => fresh,
            (None, _) => None,
            (Some(old), Some(new)) if new > old && !no_contest => Some(new),
            (Some(old), Some(new)) if new < old && no_contest => Some(new),
            (Some(_), _) => None,
        };

        let mut query = Query::update();
        let mut should_update = false;
        query
            .table(Table::User)
            .value("updated_at", Utc::now())
            .and_where(Expr::col((Table::User, "id")).eq(player.id));

        if let Some(rank) = rank_update {
            should_update = true;
            query.value("rank", rank.to_string());
        }

        // Only update if the player's flags actually changed
        if flags != player.flags {
            should_update = true;
            query.value("flags", i32::from(flags));
        }

        if should_update {
            let (query, values) = query.build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(sqlx::AssertSqlSafe(query), values)
                .execute(&mut *conn)
                .await?;
        }
    }

    Ok(())
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

/// An error for parsing a user as a model type.
#[derive(Debug, Display, From)]
pub enum NormalizeError {
    #[display("{_0}")]
    MissingData(MissingData),
    #[display("stored rank is malformed: {_0}")]
    UnknownRank(UnknownRank),
}

impl std::error::Error for NormalizeError {}

impl From<NormalizeError> for Error {
    fn from(value: NormalizeError) -> Self {
        Error::new(value)
    }
}
