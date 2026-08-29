//! Skill-based rating service.

use std::cmp::{max, min};
use std::fmt::Debug;
use std::{any::Any, future::ready};

use derive_more::{Deref, DerefMut};

use chrono::{DateTime, Utc};

use duelchannel_model::battle::BattleStatus;

use serde::{
    Serialize,
    de::{DeserializeOwned, value::UnitDeserializer},
};

use sqlx::{FromRow, Row as _, SqliteConnection, sqlite::SqliteRow};

use tracing::instrument;

use crate::{
    config::Config,
    error::Error,
    mmr::{self, Rating, RatingModel},
};

/// A rating service.
///
/// Unlike [`RatingModel`], this can mean "one or zero models." Thus, the
/// methods on this struct are specialized.
///
/// [`RatingModel`]: crate::mmr::RatingModel
pub trait RatingService: Send + Sync {
    /// The model included in the service.
    ///
    /// This may be the never type ([`!`]), in that case there is no
    /// associated model, and code paths going there are unreachable.
    type Model: RatingModel + Send + Sync + 'static;

    /// Creates a user's rating.
    ///
    /// Returns the user's ordinal plus their provisional counter, or `None`
    /// if there is no model in use.
    fn create_rating(
        &self,
        user_id: i32,
        config: &Config,
        conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<Option<(f32, u32)>, Error>> + Send;

    /// Updates the ratings of a list of users.
    fn update_cached_ratings(
        &self,
        user_ids: &[i32],
        conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Updates the ratings of a list of users after the completion of a
    /// battle.
    ///
    /// rank promotions and award grants.
    fn update_post_battle(
        &self,
        entries: &[(i32, bool)],
        config: &Config,
        conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Fetches the quality of a match by its ratings.
    fn quality_1v1(
        &self,
        ratings: &[Rating<<Self::Model as RatingModel>::Data>],
    ) -> impl Future<Output = Result<Option<f32>, Error>> + Send;

    /// Resets all MMR of a list of players.
    fn reset<I>(
        &self,
        players: I,
        config: &Config,
        conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<(), Error>> + Send
    where
        I: IntoIterator<Item = i32> + Send,
        I::IntoIter: Send;

    /// Dumps the MMR list to a writerr.
    fn dump<W>(
        &self,
        writer: W,
        conn: &mut SqliteConnection,
    ) -> impl Future<Output = eyre::Result<()>> + Send
    where
        W: std::io::Write + Send;

    /// Gets the inner model, or `None` if the service does not provide one.
    fn model(&self) -> Option<&Self::Model>;
}

impl<T> RatingService for T
where
    T: RatingModel + Send + Sync + 'static,
{
    type Model = Self;

    async fn create_rating(
        &self,
        user_id: i32,
        config: &Config,
        conn: &mut SqliteConnection,
    ) -> Result<Option<(f32, u32)>, Error> {
        init_rating::<Self::Model>(user_id, config, self, conn)
            .await
            .map(|r| (r.ordinal(), config.mmr.matches_until_rated))
            .map(Some)
    }

    async fn update_cached_ratings(
        &self,
        user_ids: &[i32],
        conn: &mut SqliteConnection,
    ) -> Result<(), Error> {
        update_ratings(user_ids, self, conn).await.map(|_| ())
    }

    async fn update_post_battle(
        &self,
        user_ids: &[(i32, bool)],
        config: &Config,
        conn: &mut SqliteConnection,
    ) -> Result<(), Error> {
        super::update_post_battle(user_ids, self, config, conn).await
    }

    async fn quality_1v1(
        &self,
        ratings: &[Rating<<Self::Model as RatingModel>::Data>],
    ) -> Result<Option<f32>, Error> {
        assert!(ratings.len() == 2);
        self.quality(ratings).await.map(Some)
    }

    async fn reset<I>(
        &self,
        players: I,
        config: &Config,
        conn: &mut SqliteConnection,
    ) -> Result<(), Error>
    where
        I: IntoIterator<Item = i32> + Send,
        I::IntoIter: Send,
    {
        for id in players.into_iter() {
            init_rating(id, config, self, conn).await?;
        }
        Ok(())
    }

    async fn dump<W>(&self, writer: W, conn: &mut SqliteConnection) -> eyre::Result<()>
    where
        W: std::io::Write + Send,
    {
        dump_rating(writer, self, conn).await
    }

    fn model(&self) -> Option<&Self::Model> {
        Some(self)
    }
}

/// Indicates that there is no rating model being used.
#[derive(Clone, Debug)]
pub struct Unrated;

impl RatingService for Unrated {
    type Model = !;

    fn create_rating(
        &self,
        _user_id: i32,
        _config: &Config,
        _conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<Option<(f32, u32)>, Error>> + Send {
        ready(Ok(None))
    }

    fn update_cached_ratings(
        &self,
        _user_ids: &[i32],
        _conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        ready(Ok(()))
    }

    fn update_post_battle(
        &self,
        _user_ids: &[(i32, bool)],
        _config: &Config,
        _conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        ready(Ok(()))
    }

    fn quality_1v1(
        &self,
        _ratings: &[Rating<<Self::Model as RatingModel>::Data>],
    ) -> impl Future<Output = Result<Option<f32>, Error>> + Send {
        ready(Ok(None))
    }

    fn reset<I>(
        &self,
        _players: I,
        _config: &Config,
        _conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<(), Error>> + Send
    where
        I: IntoIterator<Item = i32> + Send,
        I::IntoIter: Send,
    {
        ready(Ok(()))
    }

    fn dump<W>(
        &self,
        _writer: W,
        _conn: &mut SqliteConnection,
    ) -> impl Future<Output = eyre::Result<()>> + Send
    where
        W: std::io::Write + Send,
    {
        ready(Ok(()))
    }

    fn model(&self) -> Option<&Self::Model> {
        None
    }
}

/// A rating period.
#[derive(Clone, Debug, FromRow)]
pub struct RatingPeriodEntity {
    pub id: i32,
    #[sqlx(rename = "inserted_at")]
    pub started_at: DateTime<Utc>,
    #[sqlx(skip)]
    pub period_elapsed: f32,
}

/// A historic player rating.
///
/// These are fetched from the database and are associated with a rating
/// period.
#[derive(Clone, Debug, Deref, DerefMut)]
pub struct RatingEntity<T = ()> {
    /// The id of the player this is for.
    pub user_id: i32,
    /// The period this rating belongs to.
    pub period_id: i32,
    /// The player's actual rating.
    pub rating: f32,
    /// The rating deviation of the player.
    pub deviation: f32,
    /// Periods this user has been idle for.
    pub idle_periods: f32,
    /// Extra data for the rating system.
    #[deref]
    #[deref_mut]
    pub extra: T,
    /// When the record was inserted.
    pub inserted_at: DateTime<Utc>,
    /// When the record was updated.
    pub updated_at: DateTime<Utc>,
    pub matches_until_rated: i32,
    pub period: RatingPeriodEntity,
}

impl<T> From<RatingEntity<T>> for Rating<T> {
    fn from(value: RatingEntity<T>) -> Self {
        Rating {
            user_id: value.user_id,
            rating: value.rating,
            deviation: value.deviation,
            extra: value.extra,
        }
    }
}

impl<T> FromRow<'_, SqliteRow> for RatingEntity<T>
where
    T: DeserializeOwned + 'static,
{
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        // Fetch extra data
        let extra = match row.try_get::<Option<String>, _>("extra")? {
            Some(ron_str) => ron::from_str(&ron_str).map_err(|error| sqlx::Error::ColumnDecode {
                index: "extra".into(),
                source: Box::new(error),
            }),
            None => T::deserialize(UnitDeserializer::<ron::Error>::new()).map_err(|error| {
                sqlx::Error::ColumnDecode {
                    index: "extra".into(),
                    source: Box::new(error),
                }
            }),
        };

        let period_id: i32 = row.try_get("period_id")?;
        Ok(RatingEntity {
            user_id: row.try_get("user_id")?,
            period_id,
            rating: row.try_get("rating")?,
            deviation: row.try_get("deviation")?,
            idle_periods: row.try_get("idle_periods")?,
            inserted_at: row.try_get("inserted_at")?,
            updated_at: row.try_get("updated_at")?,
            matches_until_rated: row.try_get("matches_until_rated")?,
            extra: extra?,
            period: RatingPeriodEntity {
                id: period_id,
                started_at: row.try_get("period_inserted_at")?,
                period_elapsed: 0.0,
            },
        })
    }
}

#[derive(Debug, FromRow)]
struct Matchup<T> {
    #[sqlx(flatten)]
    pub opponent: RatingEntity<T>,
    #[sqlx(try_from = "u8")]
    pub status: BattleStatus,
    pub position: i32,
    pub no_contest: bool,
    pub finish_time: i32,
}

impl<T> From<Matchup<T>> for mmr::Matchup<T> {
    fn from(value: Matchup<T>) -> Self {
        mmr::Matchup {
            opponent: value.opponent.into(),
            status: value.status,
            position: value.position,
            finish_time: value.finish_time,
            no_contest: value.no_contest,
        }
    }
}

/// Initializes a user's rating, and inserts it into the database.
pub async fn init_rating<T>(
    user_id: i32,
    config: &Config,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<Rating<T::Data>, Error>
where
    T: RatingModel,
{
    init_rating_at(user_id, Utc::now(), config, model, conn).await
}

/// Initializes a user's rating, and inserts it into the database.
pub async fn init_rating_at<T>(
    user_id: i32,
    time: impl Into<DateTime<Utc>>,
    config: &Config,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<Rating<T::Data>, Error>
where
    T: RatingModel,
{
    let time = time.into();

    let matches_until_rated = config.mmr.matches_until_rated;
    let rating = model.create_rating(user_id).await?;

    // serialize extra data
    let extra = serialize_extra(&rating.extra).map_err(Error::new)?;

    let result = sqlx::query(
        r#"
        INSERT INTO rating
            (period_id, inserted_at, updated_at, user_id, rating, deviation, matches_until_rated, extra)
        SELECT p.id, $1, $1, $2, $3, $4, $5, $6
        FROM rating_period p
        ORDER BY p.inserted_at DESC
        LIMIT 1
        RETURNING id
        "#,
    )
    .bind(time)
    .bind(user_id)
    .bind(rating.rating)
    .bind(rating.deviation)
    .bind(matches_until_rated as i32)
    .bind(&extra)
    .execute(&mut *conn)
    .await?;

    // Update the cached ordinal
    sqlx::query(
        r#"
        UPDATE user
        SET ordinal = $3, hide_rating = $4, matches_until_rated = $5, updated_at = $1
        WHERE id = $2
        "#,
    )
    .bind(time)
    .bind(rating.user_id)
    .bind(rating.ordinal())
    .bind(matches_until_rated > 0)
    .bind(matches_until_rated as i32)
    .execute(&mut *conn)
    .await?;

    if result.rows_affected() > 0 {
        Ok(rating)
    } else {
        // make a new rating period and use that id instead
        let period = sqlx::query_as::<_, RatingPeriodEntity>(
            r#"
            INSERT INTO rating_period (inserted_at)
            VALUES ($1)
            RETURNING id, inserted_at
            "#,
        )
        .bind(time)
        .fetch_one(&mut *conn)
        .await?;

        tracing::info!(?period, "no mmr logged! creating a new period now...!");

        sqlx::query(
            r#"
            INSERT INTO rating
                (inserted_at, updated_at, period_id, user_id, rating, deviation, matches_until_rated, extra)
            VALUES
                ($1, $1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(time)
        .bind(period.id)
        .bind(user_id)
        .bind(rating.rating)
        .bind(rating.deviation)
        .bind(matches_until_rated as i32)
        .bind(&extra)
        .execute(&mut *conn)
        .await?;

        Ok(rating)
    }
}

/// Catalogs a player rating.
async fn catalog_rating<T>(
    rating: &RatingEntity<T>,
    conn: &mut SqliteConnection,
) -> Result<(), Error>
where
    T: Serialize + 'static,
{
    let now = Utc::now();

    // serialize extra data
    let extra = serialize_extra(&rating.extra).map_err(Error::new)?;

    sqlx::query(
        r#"
        INSERT INTO rating
            (inserted_at, updated_at, user_id, period_id, rating, deviation, idle_periods, extra, matches_until_rated)
        VALUES
            ($1, $1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(now)
    .bind(rating.user_id)
    .bind(rating.period_id)
    .bind(rating.rating)
    .bind(rating.deviation)
    .bind(rating.idle_periods)
    .bind(extra)
    .bind(rating.matches_until_rated)
    //.bind(period.started_at)
    .execute(&mut *conn)
    .await
    .map(|_| ())
    .map_err(Error::from)
}

/// A result returned by [`update_ratings_at`].
#[derive(Clone, Debug, Deref, DerefMut)]
pub struct UpdateRatingResult<T> {
    /// The updated rating.
    #[deref]
    #[deref_mut]
    pub rating: Rating<T>,
    /// How many more matches need to be played by the player until they are
    /// rated.
    pub matches_until_rated: u32,
}

impl<T> UpdateRatingResult<T> {
    pub fn into_inner(self) -> Rating<T> {
        self.rating
    }
}

/// Updates player's ratings.
///
/// See [`update_ratings_at`] for more information.
pub async fn update_ratings<T>(
    user_ids: &[i32],
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<Vec<UpdateRatingResult<T::Data>>, Error>
where
    T: RatingModel,
{
    let now = Utc::now();
    update_ratings_at(user_ids, model, now, conn).await
}

/// Updates player's ratings.
///
/// Should be called when a match is finished. This first makes sure that all
/// the players are at the current rating period.
///
/// Ensure both player's ratings exist (by calling [`get_rating`] for each of
/// them) before calling this!
#[instrument(skip(conn, model))]
pub async fn update_ratings_at<T>(
    user_ids: &[i32],
    model: &T,
    time: DateTime<Utc>,
    conn: &mut SqliteConnection,
) -> Result<Vec<UpdateRatingResult<T::Data>>, Error>
where
    T: RatingModel,
{
    // Do nothing if there are no user ids
    if user_ids.len() == 0 {
        return Ok(Vec::new());
    }

    let mut ratings = Vec::with_capacity(user_ids.len());

    let mut min_period: Option<RatingPeriodEntity> = None;
    for user_id in user_ids.iter().copied() {
        // We need to update all participant periods one by one.
        // Get each player's rating
        let rating = get_rating(user_id, time, model, conn)
            .await
            .map_err(|err| {
                Error::from(err).with_message(format!("failed to get rating for user {}", user_id))
            })?;

        if let Some(mp) = min_period.as_ref() {
            if rating.period.started_at < mp.started_at {
                min_period = Some(rating.period.clone());
            }
        } else {
            min_period = Some(rating.period.clone());
        }

        ratings.push(rating);
    }

    let Some(mut period) = min_period else {
        // There are no ratings to process
        assert!(ratings.len() == 0);
        return Ok(vec![]);
    };

    // We need to fast forward through existing periods, and add any new ones
    let mut ff = sqlx::query_as::<_, RatingPeriodEntity>(
        r#"
        SELECT *
        FROM rating_period
        WHERE
            inserted_at > $1
            AND id <> $2
        ORDER BY inserted_at ASC
        "#,
    )
    .bind(period.started_at)
    .bind(period.id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|mut period| {
        let delta = time - period.started_at;
        period.period_elapsed =
            (delta.as_seconds_f32() / model.period().as_seconds_f32()).clamp(0.0, 1.0);
        period
    })
    .collect::<Vec<_>>();

    let grace = model.decay_grace();

    loop {
        let ended_at = period.started_at + model.period();

        // Get next period for next iteration
        let next_period = match ff.pop() {
            // keep fast forwarding
            Some(period) if period.started_at <= time => Some(period),
            Some(_) => None,
            None if ended_at <= time => {
                // Create new period
                let res = sqlx::query(
                    r#"
                    INSERT INTO rating_period (inserted_at)
                    VALUES ($1)
                    "#,
                )
                .bind(ended_at)
                .execute(&mut *conn)
                .await?;

                let delta = time - ended_at;
                let period = RatingPeriodEntity {
                    id: res.last_insert_rowid() as i32,
                    started_at: ended_at,
                    period_elapsed: (delta.as_seconds_f32() / model.period().as_seconds_f32())
                        .clamp(0.0, 1.0),
                };

                tracing::info!(?period, "creating new rating period");

                Some(period)
            }
            None => None,
        };

        for rating in ratings.iter_mut() {
            // Update ratings if the periods are old
            if rating.period.started_at > period.started_at {
                continue;
            }
            let player = Rating::from(rating.clone());

            // Fetch the player's matchups
            let time_to = min(time, ended_at);
            let matchups = fetch_matchups(player.user_id, period.started_at, time_to, &mut *conn)
                .await?
                .into_iter()
                .map(mmr::Matchup::from)
                .collect::<Vec<_>>();

            // Idle periods accumulate, but don't start actually eating at your
            // deviation until after it passes over the grace period.
            let period_elapsed = if matchups.is_empty() {
                rating.idle_periods += period.period_elapsed;
                (rating.idle_periods - grace).clamp(0.0, period.period_elapsed)
            } else {
                rating.idle_periods = 0.0;
                0.0
            };

            // Find matches until rated; difference between last catalog and
            // num of matchups
            let matches_until_rated = max(rating.matches_until_rated - matchups.len() as i32, 0);

            // Get the player's new rating
            let new_rating = model
                .rate(&player, matchups.as_slice(), period_elapsed)
                .await?;

            // Update the player's existing rating
            sqlx::query(
                r#"
                UPDATE user
                SET ordinal = $3, hide_rating = $4, matches_until_rated = $5, updated_at = $1
                WHERE id = $2
                "#,
            )
            .bind(Utc::now())
            .bind(new_rating.user_id)
            .bind(new_rating.ordinal())
            .bind(matches_until_rated > 0)
            .bind(matches_until_rated)
            .execute(&mut *conn)
            .await?;

            rating.rating = new_rating.rating;
            rating.deviation = new_rating.deviation;
            rating.extra = new_rating.extra;

            rating.matches_until_rated = matches_until_rated;

            if let Some(next_period) = next_period.as_ref() {
                // Catalog it into the rating period
                rating.period = next_period.clone();
                rating.period_id = next_period.id;

                catalog_rating(&rating, &mut *conn).await?;
            }
        }

        if let Some(next_period) = next_period {
            period = next_period;
        } else {
            break;
        }
    }

    Ok(ratings
        .into_iter()
        .map(|r| UpdateRatingResult {
            rating: Rating {
                user_id: r.user_id,
                rating: r.rating,
                deviation: r.deviation,
                extra: r.extra,
            },
            matches_until_rated: r.matches_until_rated as u32,
        })
        .collect())
}

/// Gets a player's last historical record along with the rating period the
/// record is from.
pub async fn get_rating<T>(
    user_id: i32,
    time: DateTime<Utc>,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<RatingEntity<T::Data>, Error>
where
    T: RatingModel,
{
    let mut rating = sqlx::query_as::<_, RatingEntity<T::Data>>(
        r#"
        SELECT
            r.*,
            rp.inserted_at AS period_inserted_at
        FROM rating r, rating_period rp
        WHERE
            r.period_id = rp.id
            AND user_id = $1
            AND rp.inserted_at <= $2
        ORDER BY inserted_at DESC
        LIMIT 1
        "#,
    )
    .bind(user_id)
    .bind(time)
    .fetch_one(&mut *conn)
    .await?;

    // Calculate elapsed time
    let delta = time - rating.period.started_at;
    rating.period.period_elapsed =
        (delta.as_seconds_f32() / model.period().as_seconds_f32()).clamp(0.0, 1.0);
    assert!(rating.period.period_elapsed >= 0.0f32);

    Ok(rating)
}

#[instrument(skip(conn))]
async fn fetch_matchups<T>(
    user_id: i32,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    conn: &mut SqliteConnection,
) -> Result<Vec<Matchup<T>>, Error>
where
    T: DeserializeOwned + Send + Unpin + 'static,
{
    sqlx::query_as::<_, Matchup<T>>(include_str!("find_matchups.sql"))
        .bind(user_id)
        .bind(from)
        .bind(to)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        // Filter short matches if they were cancelled
        .filter(|matchup| match matchup.status {
            BattleStatus::Concluded => true,
            BattleStatus::Cancelled => matchup.finish_time > 35 * 30,
            BattleStatus::Ongoing => false,
        })
        .map(|matchup| Matchup::<T>::try_from(matchup))
        .collect::<Result<Vec<_>, _>>()
        .map_err(Error::new)
}

/// Calculates the MMR for all players in the last rating period.
pub async fn dump_rating<T, W: std::io::Write>(
    mut writer: W,
    model: &T,
    conn: &mut SqliteConnection,
) -> eyre::Result<()>
where
    T: RatingModel,
{
    let now = Utc::now();
    let from = now - model.period();

    // Write header
    writer.write(b"ID,Player Name,Total Matches,Win/Loss Rate,MMR,Deviation\n")?;

    let users = sqlx::query_as::<_, (i32, String, String)>(
        r#"
        SELECT id, short_id, display_name FROM user
        "#,
    )
    .fetch_all(&mut *conn)
    .await?;

    for (user_id, short_id, display_name) in users {
        // Get the player's record, or insert it if it doesn't exist.
        let rating = sqlx::query_as::<_, RatingEntity<T::Data>>(
            r#"
            SELECT r.*
            FROM user u, rating r
            WHERE
                p.id = $1
                AND r.id IN (
                    SELECT id
                    FROM rating r
                    WHERE r.user_id = u.id
                    ORDER BY inserted_at DESC
                    LIMIT 1
                )
            "#,
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;

        let rating = RatingEntity::<T::Data>::try_from(rating)?;
        let rating = Rating::from(rating);

        let matchups = fetch_matchups::<T::Data>(user_id, from, now, &mut *conn)
            .await?
            .into_iter()
            .map(mmr::Matchup::from)
            .collect::<Vec<_>>();

        if matchups.len() > 0 {
            // Get the player's new rating
            let new_rating = model.rate(&rating, &matchups, 1.0).await?;

            let csv_name = display_name.replace("\"", "\"\"");

            let total = matchups.len() as f32;
            let wl_rate = matchups
                .iter()
                .filter(|m| !m.no_contest)
                .map(|_| 1.0)
                .sum::<f32>()
                / total;
            let wl_rate = wl_rate.abs(); // fucked up -0 insanity

            write!(
                writer,
                "{},\"{}\",{},{:.2}%,{},{}\n",
                short_id,
                csv_name,
                matchups.len(),
                wl_rate * 100.0,
                new_rating.rating,
                new_rating.deviation,
            )?;
        }
    }

    Ok(())
}

pub fn serialize_extra<S>(data: &S) -> Result<Option<String>, ron::Error>
where
    S: Any + Serialize,
{
    if (data as &dyn Any).is::<()>() {
        // No extra data needs to be serialized if type is empty.
        Ok(None)
    } else {
        ron::to_string(data).map(Some)
    }
}

pub fn deserialize_extra<D>(extra: Option<&str>) -> Result<D, ron::Error>
where
    D: Any + DeserializeOwned,
{
    match extra {
        Some(data) => ron::from_str(data).map_err(|error| error.code),
        // No extra data should have been serialized
        None => D::deserialize(UnitDeserializer::new()),
    }
}
