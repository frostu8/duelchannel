//! Analytics for battles.

use chrono::{DateTime, Utc};
use duelchannel_model::battle::BattlePoint;
use futures_core::Stream;
use futures_util::StreamExt as _;
use sqlx::{FromRow, SqliteConnection};

use crate::{
    app::{Model, ModelOrUnrated},
    error::Error,
    schema::user::mmr::{self, Model as _, Rating, RatingRecord, RatingRow},
};

/// A set of analytics for a battle.
#[derive(Clone, Debug, FromRow)]
pub struct BattleStatistics {
    pub match_id: i32,
    pub avg_mmr: Option<i32>,
    pub quality: Option<f32>,
    pub finish_time: Option<i32>,
    pub inserted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<BattleStatistics> for duelchannel_model::battle::BattleStatistics {
    fn from(value: BattleStatistics) -> Self {
        duelchannel_model::battle::BattleStatistics {
            avg_mmr: value.avg_mmr,
            quality: value.quality,
            finish_time: value.finish_time,
        }
    }
}

#[derive(FromRow)]
struct BattlePointRow {
    pub uuid: String,
    pub level_name: String,
    pub margin_score: Option<i32>,
    #[sqlx(flatten)]
    pub statistics: BattleStatistics,
}

impl From<BattlePointRow> for BattlePoint {
    fn from(value: BattlePointRow) -> Self {
        BattlePoint {
            id: value.uuid,
            level_name: value.level_name,
            margin_score: value.margin_score,
            statistics: value.statistics.into(),
        }
    }
}

/// Streams analytics.
pub fn stream_analytics(
    conn: &mut SqliteConnection,
) -> impl Stream<Item = Result<BattlePoint, Error>> {
    sqlx::query_as::<_, BattlePointRow>(
        r#"
        SELECT bs.*, b.uuid, b.level_name, b.margin_score
        FROM battle_statistics bs, battle b
        WHERE b.id = bs.match_id
        "#,
    )
    .fetch(&mut *conn)
    .map(|s| s.map(BattlePoint::from).map_err(Error::from))
}

/// Gets analytics of a battle.
///
/// If they haven't been calculated, or if they're not up-to-date, this
/// calculates them.
pub async fn get_analytics<T>(
    battle_id: i32,
    model: &Model<T>,
    conn: &mut SqliteConnection,
) -> Result<BattleStatistics, Error>
where
    T: ModelOrUnrated,
    <T::Model as mmr::Model>::Data: Clone,
{
    #[derive(FromRow)]
    struct BattleRow {
        pub id: i32,
        #[sqlx(rename = "battle_updated_at")]
        pub updated_at: DateTime<Utc>,
        #[sqlx(flatten)]
        pub statistics: BattleStatistics,
    }

    let row = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT bs.*, b.updated_at AS battle_updated_at
        FROM battle_statistics bs, battle b
        WHERE
            bs.match_id = $1
            AND bs.match_id = b.id
        "#,
    )
    .bind(battle_id)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(row) = row {
        if row.statistics.updated_at >= row.updated_at {
            // Statistics are still up-to-date.
            Ok(row.statistics)
        } else {
            // Recalculate statistics
            let mut statistics = calculate_analytics(battle_id, model, conn).await?;
            statistics.inserted_at = row.statistics.inserted_at;

            sqlx::query(
                r#"
                UPDATE battle_statistics
                SET
                    avg_mmr = $2,
                    quality = $3,
                    finish_time = $4,
                    updated_at = $5
                WHERE
                    id = $1
                "#,
            )
            .bind(row.id)
            .bind(statistics.avg_mmr)
            .bind(statistics.quality)
            .bind(statistics.finish_time)
            .bind(statistics.updated_at)
            .execute(&mut *conn)
            .await?;

            Ok(statistics)
        }
    } else {
        let statistics = calculate_analytics(battle_id, model, conn).await?;

        sqlx::query(
            r#"
            INSERT INTO battle_statistics (match_id, avg_mmr, quality, finish_time, inserted_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(battle_id)
        .bind(statistics.avg_mmr)
        .bind(statistics.quality)
        .bind(statistics.finish_time)
        .bind(statistics.inserted_at)
        .bind(statistics.updated_at)
        .execute(&mut *conn)
        .await?;

        Ok(statistics)
    }
}

/// Calculates a set of analytics for a battle.
pub async fn calculate_analytics<T>(
    battle_id: i32,
    model: &Model<T>,
    conn: &mut SqliteConnection,
) -> Result<BattleStatistics, Error>
where
    T: ModelOrUnrated,
    <T::Model as mmr::Model>::Data: Clone,
{
    #[derive(FromRow)]
    struct ParticipantRow {
        pub finish_time: Option<i32>,
        #[sqlx(flatten)]
        pub rating: RatingRow,
    }

    let participants = sqlx::query_as::<_, ParticipantRow>(
        r#"
        SELECT r1.*, p.finish_time
        FROM rating r1, rating r2, participant p, battle b
        WHERE
            p.match_id = $1
            AND p.match_id = b.id
            AND p.user_id = r1.user_id
            AND p.user_id = r2.user_id
            AND r1.inserted_at <= b.inserted_at
            AND r2.inserted_at <= b.inserted_at
        GROUP BY
            r1.period_id, r1.user_id, r1.rating, r1.deviation, r1.extra,
            r1.inserted_at, r1.updated_at, p.finish_time
        HAVING r1.inserted_at = MAX(r2.inserted_at)
        "#,
    )
    .bind(battle_id)
    .fetch_all(&mut *conn)
    .await?;

    let ratings = participants
        .iter()
        .map(|p| &p.rating)
        .cloned()
        .map(RatingRecord::<<T::Model as mmr::Model>::Data>::try_from)
        .map(|r| r.map_err(Error::new))
        .collect::<Result<Vec<_>, Error>>()?;

    let ordinal_sum: i32 = ratings
        .iter()
        .cloned()
        .map(|r| Rating::from(r).ordinal() as i32)
        .sum();
    let avg_mmr = if ratings.len() > 0 {
        Some(ordinal_sum / ratings.len() as i32)
    } else {
        None
    };

    let quality = match model.model() {
        Some(model) if ratings.len() > 0 => model.quality(&ratings).await.map(Some)?,
        Some(_) => None,
        None => None,
    };

    let finish_time = participants.iter().filter_map(|s| s.finish_time).max();

    let now = Utc::now();

    Ok(BattleStatistics {
        match_id: battle_id,
        avg_mmr,
        quality,
        finish_time,
        inserted_at: now,
        updated_at: now,
    })
}
