//! Skill-based rating service.

use std::fmt::Debug;
use std::{any::Any, future::ready};

use derive_more::{Deref, DerefMut};

use chrono::{DateTime, Utc};

use duelchannel_model::battle::BattleStatus;

use serde::{
    Deserialize, Serialize,
    de::{DeserializeOwned, value::UnitDeserializer},
};

use sqlx::{FromRow, Row as _, SqliteConnection, sqlite::SqliteRow};

use tracing::instrument;

use crate::{
    entity::battle::update_participant_ratings,
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
    /// This returns the user's ordinal, or `None` if there is no model in-use.
    fn create_rating(
        &self,
        user_id: i32,
        conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<Option<i32>, Error>> + Send;

    /// Updates the ratings of participants in a battle.
    ///
    /// If the participants are not already preloaded, this will preload them.
    fn update_ratings(
        &self,
        battle_id: i32,
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
}

impl<T> RatingService for T
where
    T: RatingModel + Send + Sync + 'static,
{
    type Model = Self;

    async fn create_rating(
        &self,
        user_id: i32,
        conn: &mut SqliteConnection,
    ) -> Result<Option<i32>, Error> {
        init_rating::<Self::Model>(user_id, self, conn)
            .await
            .map(|r| r.ordinal() as i32)
            .map(Some)
    }

    async fn update_ratings(
        &self,
        battle_id: i32,
        conn: &mut SqliteConnection,
    ) -> Result<(), Error> {
        update_participant_ratings(battle_id, self, conn).await
    }

    async fn quality_1v1(
        &self,
        ratings: &[Rating<<Self::Model as RatingModel>::Data>],
    ) -> Result<Option<f32>, Error> {
        assert!(ratings.len() == 2);
        self.quality(ratings).await.map(Some)
    }

    async fn reset<I>(&self, players: I, conn: &mut SqliteConnection) -> Result<(), Error>
    where
        I: IntoIterator<Item = i32> + Send,
        I::IntoIter: Send,
    {
        for id in players.into_iter() {
            init_rating(id, self, conn).await?;
        }
        Ok(())
    }

    async fn dump<W>(&self, writer: W, conn: &mut SqliteConnection) -> eyre::Result<()>
    where
        W: std::io::Write + Send,
    {
        dump_rating(writer, self, conn).await
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
        _conn: &mut SqliteConnection,
    ) -> impl Future<Output = Result<Option<i32>, Error>> + Send {
        ready(Ok(None))
    }

    fn update_ratings(
        &self,
        _battle_id: i32,
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
#[derive(Clone, Debug, Deref, DerefMut, Deserialize, Serialize)]
pub struct RatingEntity<T = ()> {
    /// The id of the player this is for.
    pub user_id: i32,
    /// The period this rating belongs to.
    pub period_id: i32,
    /// The player's actual rating.
    pub rating: f32,
    /// The rating deviation of the player.
    pub deviation: f32,
    /// Extra data for the rating system.
    #[deref]
    #[deref_mut]
    #[serde(flatten)]
    pub extra: T,
    /// When the record was inserted.
    pub inserted_at: DateTime<Utc>,
    /// When the record was updated.
    pub updated_at: DateTime<Utc>,
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

        Ok(RatingEntity {
            user_id: row.try_get("user_id")?,
            period_id: row.try_get("period_id")?,
            rating: row.try_get("rating")?,
            deviation: row.try_get("deviation")?,
            inserted_at: row.try_get("inserted_at")?,
            updated_at: row.try_get("updated_at")?,
            extra: extra?,
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
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<Rating<T::Data>, Error>
where
    T: RatingModel,
{
    let now = Utc::now();

    let rating = model.create_rating(user_id).await?;

    // serialize extra data
    let extra = serialize_extra(&rating.extra).map_err(Error::new)?;

    let result = sqlx::query(
        r#"
        INSERT INTO rating
            (period_id, inserted_at, updated_at, user_id, rating, deviation, extra)
        SELECT p.id, $1, $1, $2, $3, $4, $5
        FROM rating_period p
        ORDER BY p.inserted_at DESC
        LIMIT 1
        RETURNING id
        "#,
    )
    .bind(now)
    .bind(user_id)
    .bind(rating.rating)
    .bind(rating.deviation)
    .bind(&extra)
    .execute(&mut *conn)
    .await?;

    // Update the cached ordinal
    sqlx::query(
        r#"
        UPDATE user
        SET ordinal = $3, updated_at = $1
        WHERE id = $2
        "#,
    )
    .bind(now)
    .bind(rating.user_id)
    .bind(rating.ordinal() as i32)
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
        .bind(now)
        .fetch_one(&mut *conn)
        .await?;

        tracing::info!(?period, "no mmr logged! creating a new period now...!");

        sqlx::query(
            r#"
            INSERT INTO rating
                (inserted_at, updated_at, period_id, user_id, rating, deviation, extra)
            VALUES
                ($1, $1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(now)
        .bind(period.id)
        .bind(user_id)
        .bind(rating.rating)
        .bind(rating.deviation)
        .bind(&extra)
        .execute(&mut *conn)
        .await?;

        Ok(rating)
    }
}

/// Catalogs a player rating.
async fn catalog_rating<T>(
    period: &RatingPeriodEntity,
    rating: &Rating<T>,
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
            (inserted_at, updated_at, user_id, period_id, rating, deviation, extra)
        VALUES
            ($1, $1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(now)
    .bind(rating.user_id)
    .bind(period.id)
    .bind(rating.rating)
    .bind(rating.deviation)
    .bind(extra)
    //.bind(period.started_at)
    .execute(&mut *conn)
    .await
    .map(|_| ())
    .map_err(Error::from)
}

pub async fn update_ratings<T>(
    user_ids: &[i32],
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<Vec<Rating<T::Data>>, Error>
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
) -> Result<Vec<Rating<T::Data>>, Error>
where
    T: RatingModel,
{
    let mut ratings = Vec::with_capacity(user_ids.len());
    let mut period: Option<RatingPeriodEntity> = None;

    for user_id in user_ids.iter().copied() {
        // Update players' rating
        let current_period = next_rating_period_at(user_id, model, time, &mut *conn).await?;

        // Get player's current rating
        ratings.push(get_rating::<T>(user_id, &mut *conn).await?);
        period = Some(current_period);
    }

    let Some(period) = period else {
        // There are no ratings to process
        assert!(ratings.len() == 0);
        return Ok(vec![]);
    };

    let mut out = Vec::with_capacity(ratings.len());
    for rating in ratings {
        out.push(update_one_rating(&rating, &period, model, &mut *conn).await?);
    }

    Ok(out)
}

async fn update_one_rating<T>(
    rating: &RatingEntity<T::Data>,
    period: &RatingPeriodEntity,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<Rating<T::Data>, Error>
where
    T: RatingModel,
{
    let ends_at = period.started_at + model.period();

    let matchups = fetch_matchups(rating.user_id, period.started_at, ends_at, &mut *conn)
        .await?
        .into_iter()
        .map(mmr::Matchup::<T::Data>::from)
        .collect::<Vec<_>>();

    // Get the player's new rating
    let rating = Rating::<T::Data>::from(rating.clone());
    let new_rating = model
        .rate(&rating, matchups.as_slice(), period.period_elapsed)
        .await?;

    // Cap deviation at certain value
    // TODO: move this into the glicko2 mod, as deviation capping is only a
    // glicko2 thing; openskill doesn't need this
    //new_rating.deviation = f32::min(new_rating.deviation, config.defaults.deviation);

    tracing::debug!(?rating, ?new_rating, "updating rating for");

    // Update the cached ordinal
    let now = Utc::now();
    sqlx::query(
        r#"
        UPDATE user
        SET ordinal = $3, updated_at = $1
        WHERE id = $2
        "#,
    )
    .bind(now)
    .bind(new_rating.user_id)
    .bind(new_rating.ordinal() as i32)
    .execute(&mut *conn)
    .await?;

    Ok(new_rating)
}

/// Fetches the last start of the rating period for a given user.
///
/// If there are no rating periods, this initializes a rating period and
/// returns it. If there is one, but it has expired, this closes rating
/// periods until falling on a single rating period.
pub async fn next_rating_period<T>(
    user_id: i32,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<RatingPeriodEntity, Error>
where
    T: RatingModel,
{
    let now = Utc::now();
    next_rating_period_at(user_id, model, now, conn).await
}

/// Fetches the last start of the rating period at the given time.
///
/// If there are no rating periods, this initializes a rating period and
/// returns it. If there is one, but it has expired, this closes rating
/// periods until falling on a single rating period.
pub async fn next_rating_period_at<T>(
    user_id: i32,
    model: &T,
    time: DateTime<Utc>,
    conn: &mut SqliteConnection,
) -> Result<RatingPeriodEntity, Error>
where
    T: RatingModel,
{
    // Get last period the player participated in
    let period = sqlx::query_as::<_, RatingPeriodEntity>(
        r#"
        SELECT p.*
        FROM rating_period p, rating r
        WHERE
            r.period_id = p.id
            AND r.user_id = $1
        ORDER BY inserted_at DESC
        LIMIT 1
        "#,
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(mut period) = period else {
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

        return Ok(period);
    };

    // Fast-forward logged periods
    let ff = sqlx::query_as::<_, RatingPeriodEntity>(
        r#"
        SELECT *
        FROM rating_period
        WHERE inserted_at > $1
        ORDER BY inserted_at ASC
        "#,
    )
    .bind(period.started_at)
    .fetch_all(&mut *conn)
    .await?;

    let grace = model.decay_grace();

    let mut idle_periods = 0.0f32;
    for next_period in ff {
        let started_at = period.started_at;
        let ended_at = next_period.started_at;

        // Get player rating
        let player = get_rating::<T>(user_id, &mut *conn).await?;
        let player = Rating::from(player);

        // All players get their rating rolled over if they had one.
        // Fetch the player's matchups
        let matchups = fetch_matchups(player.user_id, started_at, ended_at, &mut *conn)
            .await?
            .into_iter()
            .map(mmr::Matchup::from)
            .collect::<Vec<_>>();

        // Idle periods accumulate, but don't start actually eating at your
        // deviation until after it passes over the grace period.
        let period_elapsed = if matchups.is_empty() {
            idle_periods += 1.0;
            (idle_periods - grace).clamp(0.0, 1.0)
        } else {
            idle_periods = 0.0;
            1.0
        };

        // Get the player's new rating
        let new_rating = model
            .rate(&player, matchups.as_slice(), period_elapsed)
            .await?;

        let now = Utc::now();

        // Update the player's existing rating
        sqlx::query(
            r#"
            UPDATE user
            SET ordinal = $3, updated_at = $2
            WHERE id = $1
            "#,
        )
        .bind(now)
        .bind(player.user_id)
        .bind(new_rating.ordinal() as i32)
        .execute(&mut *conn)
        .await?;

        // Insert it into the rating period
        catalog_rating(&next_period, &new_rating, &mut *conn).await?;

        // Update old period
        period = next_period;
    }

    // Now, period is the most recent in the database, but check if we need to
    // close future periods.

    // Close any pending periods
    let delta = time - period.started_at;
    let mut elapsed_periods = delta.as_seconds_f32() / model.period().as_seconds_f32();

    period.period_elapsed = f32::min(elapsed_periods, 1.0);

    while elapsed_periods >= 1.0 {
        let ended_at = period.started_at + model.period();

        tracing::debug!(
            ?period,
            "closing rating period {} - {}",
            period.started_at,
            ended_at
        );

        // Insert a new period into the database
        let mut new_period = sqlx::query_as::<_, RatingPeriodEntity>(
            r#"
            INSERT INTO rating_period (inserted_at)
            VALUES ($1)
            RETURNING id, inserted_at
            "#,
        )
        .bind(ended_at)
        .fetch_one(&mut *conn)
        .await?;
        new_period.period_elapsed = f32::min(elapsed_periods, 1.0);

        // Get player rating
        let player = get_rating::<T>(user_id, &mut *conn).await?;
        let player = Rating::from(player);

        // All players get their rating rolled over if they had one.
        // Fetch the player's matchups
        let matchups = fetch_matchups(player.user_id, period.started_at, ended_at, &mut *conn)
            .await?
            .into_iter()
            .map(mmr::Matchup::from)
            .collect::<Vec<_>>();

        // Idle periods accumulate, but don't start actually eating at your
        // deviation until after it passes over the grace period.
        let period_elapsed = if matchups.is_empty() {
            idle_periods += 1.0;
            (idle_periods - grace).clamp(0.0, 1.0)
        } else {
            idle_periods = 0.0;
            1.0
        };

        // Get the player's new rating
        let new_rating = model.rate(&player, &matchups, period_elapsed).await?;

        let now = Utc::now();

        // Update the player's existing rating
        sqlx::query(
            r#"
            UPDATE user
            SET ordinal = $3, updated_at = $2
            WHERE id = $1
            "#,
        )
        .bind(now)
        .bind(player.user_id)
        .bind(new_rating.ordinal() as i32)
        .execute(&mut *conn)
        .await?;

        // Insert it into the rating period
        catalog_rating(&new_period, &new_rating, &mut *conn).await?;

        // Continue to next period
        period = new_period;
        elapsed_periods -= 1.0;
    }

    Ok(period)
}

/// Gets a player's last historical record
pub async fn get_rating<T>(
    user_id: i32,
    conn: &mut SqliteConnection,
) -> Result<RatingEntity<T::Data>, Error>
where
    T: RatingModel,
{
    let rating = sqlx::query_as::<_, RatingEntity<T::Data>>(
        r#"
        SELECT *
        FROM rating
        WHERE user_id = $1
        ORDER BY inserted_at DESC
        LIMIT 1
        "#,
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    RatingEntity::<T::Data>::try_from(rating).map_err(Error::new)
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
