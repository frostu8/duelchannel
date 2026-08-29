//! Battle functions and utilities.

pub mod analytics;

use std::fmt::Debug;

use chrono::{DateTime, Utc};

use duelchannel_model::{
    battle::{Battle, BattleStatus, ItemUsage, KartItem, Participant, PlayerTeam},
    profile::Skin,
    user::User,
};

use rand::{RngExt as _, SeedableRng as _, rngs::StdRng};
use sea_query::{
    Asterisk, Expr, ExprTrait, Iden, JoinType, Query, SelectStatement, SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::{FromRow, Row as _, SqliteConnection, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    config::Config,
    entity::{MissingData, user::UserEntity},
    error::Error,
    short_id::IdsExhausted,
};

const MAX_INSERT_ATTEMPTS: usize = 5;

/// A schema for battles stored in database.
#[derive(Clone, Debug, FromRow)]
pub struct BattleEntity {
    pub id: i32,
    pub server_id: Option<i32>,
    #[sqlx(try_from = "String")]
    pub uuid: Uuid,
    pub level_name: String,
    #[sqlx(try_from = "u8")]
    pub status: BattleStatus,
    pub margin_score: i32,
    pub replay_hash: Option<String>,
    pub replay_filename: Option<String>,
    pub concluded_at: Option<DateTime<Utc>>,
    pub inserted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[sqlx(skip)]
    pub participants: Option<Vec<ParticipantEntity>>,
}

impl BattleEntity {
    /// Preloads the `participants` field of a [`Battle`].
    pub async fn preload_participants(
        &mut self,
        conn: &mut SqliteConnection,
    ) -> Result<&[ParticipantEntity], Error> {
        let (query, values) = select_participants()
            .and_where(Expr::col((Table::Participant, "match_id")).eq(self.id))
            .build_sqlx(SqliteQueryBuilder);
        let mut participants = sqlx::query_with(sqlx::AssertSqlSafe(query), values)
            .try_map(unpack_participant)
            .fetch_all(&mut *conn)
            .await
            .map_err(Error::from)?;

        for p in participants.iter_mut() {
            p.preload_roulette(&mut *conn).await?;

            if let Some(user) = p.user.as_mut() {
                user.preload_statistics(&mut *conn).await?;
            }
        }

        self.participants = Some(participants);
        Ok(self.participants.as_ref().unwrap())
    }
}

impl TryFrom<BattleEntity> for Battle {
    type Error = MissingData;

    fn try_from(value: BattleEntity) -> Result<Self, Self::Error> {
        Ok(Battle {
            id: value.uuid.hyphenated().to_string(),
            level_name: value.level_name,
            participants: value
                .participants
                .ok_or_else(|| MissingData::new("participants"))?
                .into_iter()
                .map(Participant::try_from)
                .collect::<Result<Vec<_>, MissingData>>()?,
            status: value.status,
            margin_score: value.margin_score,
            replay_url: None,
            started_at: value.inserted_at,
        })
    }
}

/// Creates a new [`BattleBuilder`].
pub fn build_battle(level_name: impl Into<String>) -> BattleBuilder {
    BattleBuilder::new(level_name)
}

/// A battle builder.
#[derive(Debug)]
pub struct BattleBuilder {
    server_id: Option<i32>,
    timestamp: DateTime<Utc>,
    level_name: String,
    status: BattleStatus,
}

impl BattleBuilder {
    /// Creates a new `BattleBuilder`.
    pub fn new(level_name: impl Into<String>) -> BattleBuilder {
        BattleBuilder {
            server_id: None,
            timestamp: Utc::now(),
            level_name: level_name.into(),
            status: BattleStatus::Ongoing,
        }
    }

    /// Sets the server id of the battle it took place on.
    pub fn server_id(self, server_id: i32) -> BattleBuilder {
        BattleBuilder {
            server_id: Some(server_id),
            ..self
        }
    }

    /// Sets the battle's status.
    pub fn status(self, status: BattleStatus) -> BattleBuilder {
        BattleBuilder { status, ..self }
    }

    /// Sets the battle's timestamp.
    pub fn timestamp(self, timestamp: impl Into<DateTime<Utc>>) -> BattleBuilder {
        BattleBuilder {
            timestamp: timestamp.into(),
            ..self
        }
    }

    /// Creates the battle builder.
    pub async fn create(self, conn: &mut SqliteConnection) -> Result<BattleEntity, Error> {
        let BattleBuilder {
            timestamp,
            server_id,
            level_name,
            status,
            ..
        } = self;

        let mut rng = StdRng::from_seed(rand::random());

        let mut inserted_id = None::<(i32, Uuid)>;
        for _ in 0..MAX_INSERT_ATTEMPTS {
            let mut bytes = [0u8; 16];
            rng.fill(&mut bytes);

            let uuid = uuid::Builder::from_random_bytes(bytes).into_uuid();

            // Create the battle
            let res = sqlx::query(
                r#"
                INSERT INTO battle (inserted_at, updated_at, server_id, uuid, level_name, status)
                VALUES ($1, $1, $2, $3, $4, $5)
                "#,
            )
            .bind(timestamp)
            .bind(server_id)
            .bind(uuid.hyphenated().to_string())
            .bind(&level_name)
            .bind(u8::from(status))
            .execute(&mut *conn)
            .await;

            match res {
                Ok(res) => {
                    inserted_id = Some((res.last_insert_rowid() as i32, uuid));
                    break;
                }
                // regenerate ID
                Err(sqlx::Error::Database(err)) if err.is_unique_violation() => (),
                Err(err) => return Err(err.into()),
            }
        }

        let Some((id, uuid)) = inserted_id else {
            return Err(IdsExhausted.into());
        };

        Ok(BattleEntity {
            id,
            server_id,
            uuid,
            level_name,
            status,
            margin_score: 0,
            replay_hash: None,
            replay_filename: None,
            concluded_at: None,
            inserted_at: timestamp,
            updated_at: timestamp,
            participants: Some(Vec::new()),
        })
    }
}

/// Gets the replay url of a battle.
pub fn get_replay_url(battle: &BattleEntity, config: &Config) -> Option<String> {
    battle
        .replay_hash
        .as_ref()
        .zip(battle.replay_filename.as_ref())
        .map(|(hash, filename)| format!("{}/{}/{}", config.cdn.base_url, hash, filename))
}

/// Represents a possibly failed left join.
#[derive(Clone, Debug, FromRow)]
pub struct MaybeSkinEntity {
    #[sqlx(rename = "skin")]
    name: Option<String>,
    realname: Option<String>,
    kartspeed: Option<i32>,
    kartweight: Option<i32>,
}

impl From<MaybeSkinEntity> for Option<Skin> {
    fn from(value: MaybeSkinEntity) -> Option<Skin> {
        Some(Skin {
            name: value.name?,
            real_name: value.realname?,
            kart_speed: value.kartspeed?,
            kart_weight: value.kartweight?,
        })
    }
}

/// A single participant.
#[derive(Clone, Debug, FromRow)]
pub struct ParticipantEntity {
    // from participants
    pub id: i32,
    pub user_id: i32,
    pub name: String,
    #[sqlx(try_from = "u8")]
    pub team: PlayerTeam,
    pub finish_time: Option<i32>,
    pub no_contest: bool,
    pub skin_color: Option<String>,

    #[sqlx(skip)]
    pub user: Option<UserEntity>,
    #[sqlx(skip)]
    pub roulette: Option<Vec<RouletteEntity>>,
    // from skin table (on good join)
    #[sqlx(flatten)]
    pub skin: MaybeSkinEntity,
}

impl ParticipantEntity {
    /// Adds some lines to a participant's roulette stats.
    pub async fn extend_roulette<I>(
        &mut self,
        extend: I,
        conn: &mut SqliteConnection,
    ) -> Result<&[RouletteEntity], Error>
    where
        I: IntoIterator<Item = ItemUsage>,
    {
        for item in extend {
            let ItemUsage { item, stack, count } = item;

            sqlx::query(
                r#"
                INSERT INTO roulette (participant_id, item, multiplicity, count)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (participant_id, item, multiplicity)
                DO UPDATE SET count = count + $4
                "#,
            )
            .bind(self.id)
            .bind(item.name())
            .bind(stack as i32)
            .bind(count as i32)
            .execute(&mut *conn)
            .await?;
        }

        // Refetch roulette stats, using the database to serialize it all
        self.preload_roulette(conn).await
    }

    /// Loads the participant's roulette stats.
    pub async fn preload_roulette(
        &mut self,
        conn: &mut SqliteConnection,
    ) -> Result<&[RouletteEntity], Error> {
        let roulette = sqlx::query_as::<_, RouletteEntity>(
            r#"
            SELECT *
            FROM roulette
            WHERE participant_id = $1
            "#,
        )
        .bind(self.id)
        .fetch_all(conn)
        .await?;

        self.roulette = Some(roulette);
        Ok(self.roulette.as_ref().unwrap().as_slice())
    }
}

impl TryFrom<ParticipantEntity> for Participant {
    type Error = MissingData;

    fn try_from(value: ParticipantEntity) -> Result<Self, Self::Error> {
        Ok(Participant {
            user: value
                .user
                .ok_or_else(|| MissingData {
                    field_name: String::from("user"),
                })
                .and_then(User::try_from)?,
            roulette: value
                .roulette
                .ok_or_else(|| MissingData {
                    field_name: String::from("roulette"),
                })
                .map(|v| v.into_iter().map(ItemUsage::from).collect())?,
            name: value.name,
            team: value.team,
            finish_time: value.finish_time,
            no_contest: value.no_contest,
            skin: value.skin.into(),
            skin_color: value.skin_color,
        })
    }
}

/// A row in the roulette table.
#[derive(Clone, Debug, FromRow)]
pub struct RouletteEntity {
    pub id: i32,
    pub participant_id: i32,
    #[sqlx(try_from = "String")]
    pub item: KartItem,
    pub multiplicity: i32,
    pub count: i32,
}

impl From<RouletteEntity> for ItemUsage {
    fn from(value: RouletteEntity) -> Self {
        ItemUsage {
            item: value.item,
            stack: value.multiplicity as usize,
            count: value.count as usize,
        }
    }
}

/// Fetches a single participant by their short_id.
pub async fn get_participant_by_short_id(
    battle_id: i32,
    short_id: &str,
    conn: &mut SqliteConnection,
) -> Result<Option<ParticipantEntity>, Error> {
    use Table::*;

    let (query, values) = select_participants()
        .and_where(Expr::col((User, "short_id")).eq(short_id))
        .and_where(Expr::col((Participant, "match_id")).eq(battle_id))
        .build_sqlx(SqliteQueryBuilder);
    let participant = sqlx::query_with(sqlx::AssertSqlSafe(query), values)
        .fetch_optional(&mut *conn)
        .await
        .and_then(|row| row.map(unpack_participant).transpose())
        .map_err(Error::from)?;

    let Some(mut participant) = participant else {
        return Ok(None);
    };

    if let Some(user) = participant.user.as_mut() {
        user.preload_statistics(conn).await?;
    }

    Ok(Some(participant))
}

#[derive(Iden)]
enum Table {
    User,
    Participant,
    Skin,
}

fn select_participants() -> SelectStatement {
    use Table::*;

    Query::select()
        .column((Participant, Asterisk))
        .column((Skin, "realname"))
        .column((Skin, "kartspeed"))
        .column((Skin, "kartweight"))
        .expr_as(Expr::col((User, "short_id")), "user_short_id")
        .expr_as(Expr::col((User, "display_name")), "user_display_name")
        .expr_as(Expr::col((User, "avatar_url")), "user_avatar_url")
        .expr_as(Expr::col((User, "flags")), "user_flags")
        .expr_as(Expr::col((User, "ordinal")), "user_ordinal")
        .expr_as(Expr::col((User, "hide_rating")), "user_hide_rating")
        .expr_as(
            Expr::col((User, "matches_until_rated")),
            "user_matches_until_rated",
        )
        .expr_as(Expr::col((User, "inserted_at")), "user_inserted_at")
        .expr_as(Expr::col((User, "updated_at")), "user_updated_at")
        .from(Participant)
        .join(
            JoinType::Join,
            User,
            Expr::col((Participant, "user_id")).equals((User, "id")),
        )
        .join(
            JoinType::LeftJoin,
            Skin,
            Expr::col((Participant, "skin")).equals((Skin, "name")),
        )
        .take()
}

fn unpack_participant(row: SqliteRow) -> Result<ParticipantEntity, sqlx::Error> {
    let participant = ParticipantEntity::from_row(&row)?;

    Ok(ParticipantEntity {
        user: Some(UserEntity {
            id: row.try_get("user_id")?,
            short_id: row.try_get("user_short_id")?,
            display_name: row.try_get("user_display_name")?,
            avatar_url: row.try_get("user_avatar_url")?,
            flags: row
                .try_get::<i32, _>("user_flags")?
                .try_into()
                .map_err(|err| sqlx::Error::ColumnDecode {
                    index: "user_flags".into(),
                    source: Box::new(err),
                })?,
            ordinal: row.try_get("user_ordinal")?,
            hide_rating: row.try_get("user_hide_rating")?,
            matches_until_rated: row.try_get("user_matches_until_rated")?,
            inserted_at: row.try_get("user_inserted_at")?,
            updated_at: row.try_get("user_updated_at")?,
            statistics: None,
            profiles: None,
        }),
        ..participant
    })
}
