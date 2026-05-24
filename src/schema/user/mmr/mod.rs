//! Skill-based placements.

pub mod glicko2;
pub mod openskill;

use std::any::Any;
use std::fmt::Debug;

use derive_more::{Deref, DerefMut};

use chrono::{DateTime, TimeDelta, Utc};

use duelchannel_model::battle::BattleStatus;

use serde::{
    Deserialize, Serialize,
    de::{DeserializeOwned, value::UnitDeserializer},
};

use sqlx::{FromRow, SqliteConnection};

use tracing::instrument;

use crate::error::Error;

/// A rating model.
pub trait Model: Send + Sync {
    /// The associated data type used to make the model function.
    type Data: ModelData + Serialize + DeserializeOwned + 'static;

    /// Initializes a new rating.
    fn create_rating(
        &self,
        player_id: i32,
    ) -> impl Future<Output = Result<Rating<Self::Data>, Error>> + Send + Sync;

    /// Rates a player's performance.
    ///
    /// This also passes a `period_elapsed` delta.
    fn rate(
        &self,
        rating: &RatingRecord<Self::Data>,
        matchups: &[Matchup<Self::Data>],
        period_elapsed: f32,
    ) -> impl Future<Output = Result<Rating<Self::Data>, Error>> + Send + Sync;

    /// The time between rating periods.
    fn period(&self) -> TimeDelta;
}

pub trait ModelData: Send + Sync + Sized + 'static {
    /// The ordinal of the rating.
    fn ordinal(rating: &Rating<Self>) -> f32 {
        rating.rating - rating.deviation * 2.0
    }
}

impl ModelData for () {}

/// The rating period.
#[derive(Clone, Debug, FromRow)]
pub struct RatingPeriod {
    pub id: i32,
    #[sqlx(rename = "inserted_at")]
    pub started_at: DateTime<Utc>,
    #[sqlx(skip)]
    pub period_elapsed: f32,
}

/// A matchup between two players.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Matchup<T = ()> {
    /// The opponent of the player.
    pub opponent: RatingRecord<T>,
    /// The status of the match that the player participated in.
    pub status: BattleStatus,
    /// The player's finish position.
    pub position: i32,
    /// The player's finish time.
    pub finish_time: i32,
    /// Whether the player NO CONTEST'd.
    pub no_contest: bool,
}

#[derive(Debug, FromRow)]
struct MatchupQuery {
    #[sqlx(flatten)]
    pub opponent: RatingRow,
    #[sqlx(try_from = "u8")]
    pub status: BattleStatus,
    pub position: i32,
    pub no_contest: bool,
    pub finish_time: i32,
}

impl<T> TryFrom<MatchupQuery> for Matchup<T>
where
    T: DeserializeOwned + 'static,
{
    type Error = ron::Error;

    fn try_from(value: MatchupQuery) -> Result<Self, Self::Error> {
        value.opponent.try_into().map(|opponent| Matchup {
            opponent,
            status: value.status,
            position: value.position,
            finish_time: value.finish_time,
            no_contest: value.no_contest,
        })
    }
}

/// A single player rating.
///
/// The rating may also contain arbitrary info `T` for the relevant MMR system
/// to query.
#[derive(Clone, Debug, Deref, DerefMut, Deserialize, Serialize)]
pub struct Rating<T = ()> {
    /// The id of the player this is for.
    pub user_id: i32,
    /// The player's actual rating.
    pub rating: f32,
    /// The rating deviation of the player.
    pub deviation: f32,
    /// Extra data for the rating system.
    #[deref]
    #[deref_mut]
    #[serde(flatten)]
    pub extra: T,
}

impl<T> Rating<T>
where
    T: ModelData,
{
    /// The player's ordinal.
    ///
    /// This is a number where the player's true skill rating is above with a
    /// 95% chance.
    pub fn ordinal(&self) -> f32 {
        T::ordinal(self)
    }
}

/// A historic player rating.
///
/// These are fetched from the database and are associated with a rating
/// period.
#[derive(Clone, Debug, Deref, DerefMut, Deserialize, Serialize)]
pub struct RatingRecord<T = ()> {
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

impl<T> From<RatingRecord<T>> for Rating<T> {
    fn from(value: RatingRecord<T>) -> Self {
        Rating {
            user_id: value.user_id,
            rating: value.rating,
            deviation: value.deviation,
            extra: value.extra,
        }
    }
}

/// Inner struct for querying the database.
#[derive(Clone, Debug, FromRow)]
pub struct RatingRow {
    /// The period this rating belongs to.
    pub period_id: i32,
    /// The id of the player this is for.
    pub user_id: i32,
    /// The player's actual rating.
    pub rating: f32,
    /// The rating deviation of the player.
    pub deviation: f32,
    /// Serialized extra data.
    pub extra: Option<String>,
    /// When the record was inserted.
    pub inserted_at: DateTime<Utc>,
    /// When the record was updated.
    pub updated_at: DateTime<Utc>,
}

impl<T> TryFrom<RatingRow> for RatingRecord<T>
where
    T: DeserializeOwned + 'static,
{
    type Error = ron::Error;

    fn try_from(value: RatingRow) -> Result<Self, Self::Error> {
        // Deserialize extra
        let extra = deserialize_extra(value.extra.as_deref())?;

        Ok(RatingRecord {
            user_id: value.user_id,
            period_id: value.period_id,
            rating: value.rating,
            deviation: value.deviation,
            extra,
            inserted_at: value.inserted_at,
            updated_at: value.updated_at,
        })
    }
}

/// Initializes a user's rating, and inserts it into the database.
pub async fn init_rating<T>(
    user_id: i32,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<Rating<T::Data>, Error>
where
    T: Model,
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
        let period = sqlx::query_as::<_, RatingPeriod>(
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
    period: &RatingPeriod,
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
            (user_id, period_id, rating, deviation, extra, inserted_at)
        VALUES
            ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(rating.user_id)
    .bind(period.id)
    .bind(rating.rating)
    .bind(rating.deviation)
    .bind(extra)
    .bind(now)
    //.bind(period.started_at)
    .execute(&mut *conn)
    .await
    .map(|_| ())
    .map_err(Error::from)
}

/// Updates a player's current rating.
///
/// Should be called when a match is finished.
///
/// Ensure both player's ratings exist (by calling [`get_rating`] for each of
/// them) before calling this!
#[instrument(skip(conn))]
pub async fn update_rating<T>(
    rating: &RatingRecord<T::Data>,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<Rating<T::Data>, Error>
where
    T: Model + Debug,
    T::Data: Debug,
{
    let now = Utc::now();

    // Get the current period start
    let period = next_rating_period_at(rating.user_id, model, now, &mut *conn).await?;
    let ends_at = period.started_at + model.period();

    let matchups = fetch_matchups(rating.user_id, period.started_at, ends_at, &mut *conn).await?;

    // Get the player's new rating
    let new_rating = model.rate(rating, &matchups, period.period_elapsed).await?;

    // Cap deviation at certain value
    // TODO: move this into the glicko2 mod
    //new_rating.deviation = f32::min(new_rating.deviation, config.defaults.deviation);

    tracing::debug!(?new_rating, "updating rating for");

    // Update the cached ordinal
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
) -> Result<RatingPeriod, Error>
where
    T: Model,
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
    now: DateTime<Utc>,
    conn: &mut SqliteConnection,
) -> Result<RatingPeriod, Error>
where
    T: Model,
{
    // Get last period the player participated in
    let period = sqlx::query_as::<_, RatingPeriod>(
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
        let period = sqlx::query_as::<_, RatingPeriod>(
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

        return Ok(period);
    };

    // Fetch logged periods
    let mut next_periods = sqlx::query_as::<_, RatingPeriod>(
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

    // Close any pending periods
    let delta = now - period.started_at;
    let mut elapsed_periods = delta.as_seconds_f32() / model.period().as_seconds_f32();

    period.period_elapsed = f32::min(elapsed_periods, 1.0);

    while elapsed_periods >= 1.0 {
        let ended_at = period.started_at + model.period();
        let mut new_period = match next_periods.pop() {
            Some(period) => period,
            None => {
                // No more periods, insert a new one.
                tracing::debug!(
                    ?period,
                    "closing rating period {} - {}",
                    period.started_at,
                    ended_at
                );

                // Insert a new period into the database
                sqlx::query_as::<_, RatingPeriod>(
                    r#"
                    INSERT INTO rating_period (inserted_at)
                    VALUES ($1)
                    RETURNING id, inserted_at
                    "#,
                )
                .bind(ended_at)
                .fetch_one(&mut *conn)
                .await?
            }
        };
        new_period.period_elapsed = f32::min(elapsed_periods, 1.0);

        // Get player rating
        let player = sqlx::query_as::<_, RatingRow>(
            r#"
            SELECT r.*
            FROM rating r
            WHERE r.user_id = $1
            ORDER BY inserted_at DESC
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        let player = RatingRecord::<T::Data>::try_from(player).map_err(Error::new)?;

        // All players get their rating rolled over if they had one.
        // Fetch the player's matchups
        let matchups =
            fetch_matchups(player.user_id, period.started_at, ended_at, &mut *conn).await?;

        // Get the player's new rating
        let new_rating = model
            .rate(&player, &matchups, period.period_elapsed)
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
        catalog_rating(&new_period, &new_rating, &mut *conn).await?;

        // Continue to next period
        period = new_period;
        elapsed_periods -= 1.0;
    }

    Ok(period)
}

#[instrument(skip(conn))]
async fn fetch_matchups<T>(
    user_id: i32,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    conn: &mut SqliteConnection,
) -> Result<Vec<Matchup<T>>, Error>
where
    T: DeserializeOwned + 'static,
{
    sqlx::query_as::<_, MatchupQuery>(include_str!("find_matchups.sql"))
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
    T: Model,
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
        let rating = sqlx::query_as::<_, RatingRow>(
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

        let rating = RatingRecord::<T::Data>::try_from(rating)?;

        let matchups = fetch_matchups::<T::Data>(user_id, from, now, &mut *conn).await?;

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

#[cfg(test)]
mod tests {
    use duelchannel_model::Rrid;
    use sqlx::sqlite::SqlitePoolOptions;
    use uuid::Uuid;

    use crate::{
        battle::update_participant_ratings,
        schema::user::{create_player, get_player, mmr::openskill::OpenSkillData},
    };

    use super::*;

    #[tokio::test]
    async fn test_rating_period1() {
        let db = SqlitePoolOptions::new().connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let mut conn = db.acquire().await.unwrap();

        // Create openskill model
        let model = openskill::OpenSkillConfig::default()
            .connect()
            .await
            .expect("valid openskill model");

        // Create players
        let player1 = create_player(
            &Rrid::new("26ABFC4C5960182E8FE20203A1634E9ECB42BBFCCF8CE2965306213E5C75E921").unwrap(),
            "Metal Sonic",
            &mut *conn,
        )
        .await
        .unwrap();
        let player2 = create_player(
            &Rrid::new("384F5460E7C95047245E92E7249AF019FB5215A7ABED748CF25FB1EA24B39443").unwrap(),
            "Phil's Pills",
            &mut *conn,
        )
        .await
        .unwrap();

        // Create ratings
        init_rating(player1.id, &model, &mut *conn)
            .await
            .expect("valid rating initialization");
        init_rating(player2.id, &model, &mut *conn)
            .await
            .expect("valid rating initialization");

        // Register battle
        let now = Utc::now();
        let uuid = Uuid::new_v4();
        let (battle_id,) = sqlx::query_as::<_, (i32,)>(
            r#"
            INSERT INTO battle (uuid, level_name, inserted_at, concluded_at, closed_at, status)
            VALUES ($1, $2, $3, $3, $3, $4)
            RETURNING id
            "#,
        )
        .bind(uuid.hyphenated().to_string())
        .bind("Withering Chateau Zone")
        .bind(now)
        .bind(u8::from(BattleStatus::Concluded))
        .fetch_one(&mut *conn)
        .await
        .unwrap();

        for (i, player) in [&player1, &player2].into_iter().enumerate() {
            let no_contest = i == 1;
            let finish_time = if no_contest { None } else { Some(3050) };

            // add player to match
            sqlx::query(
                r#"
                INSERT INTO participant
                    (match_id, player_id, team, skin, kart_speed, kart_weight, no_contest, finish_time)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                "#,
            )
            .bind(battle_id)
            .bind(player.id)
            .bind(i as u8)
            .bind("aigis")
            .bind(6)
            .bind(7)
            .bind(no_contest)
            .bind(finish_time)
            .execute(&mut *conn)
            .await
            .unwrap();
        }

        // update ratings for all players
        update_participant_ratings(battle_id, &model, &mut *conn)
            .await
            .unwrap();

        async fn get_rating(
            short_id: &str,
            conn: &mut SqliteConnection,
        ) -> eyre::Result<Rating<OpenSkillData>> {
            get_player(&short_id, &mut *conn)
                .await?
                .and_then(|r| match r.rating.zip(r.deviation) {
                    Some((rating, deviation)) => Some(RawRating {
                        player_id: r.id,
                        rating,
                        deviation,
                        extra: r.extra,
                    }),
                    None => None,
                })
                .ok_or_else(|| eyre::eyre!("player doesn't exist"))?
                .try_into()
                .map_err(From::from)
        }

        let rating1before = get_rating(&player1.short_id, &mut *conn).await.unwrap();
        let rating2before = get_rating(&player2.short_id, &mut *conn).await.unwrap();

        let later = now + model.period() * 2;

        for player in [player1, player2] {
            next_rating_period_at(player.id, &model, later, &mut *conn)
                .await
                .unwrap();
        }

        let rating1after = get_rating(&player1.short_id, &mut *conn).await.unwrap();
        let rating2after = get_rating(&player2.short_id, &mut *conn).await.unwrap();

        assert_eq!(
            rating1before.ordinal(),
            rating1after.ordinal(),
            "rating 1 neq"
        );
        assert_eq!(
            rating2before.ordinal(),
            rating2after.ordinal(),
            "rating 2 neq"
        );
    }
}
