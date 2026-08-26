//! Battle functions and utilities.

pub mod analytics;

use std::fmt::Debug;

use chrono::{DateTime, Utc};

use duelchannel_model::{
    battle::{Battle, BattleStatus, Participant, PlayerTeam},
    profile::Skin,
    user::{User, UserFlags},
};

use sea_query::{
    Asterisk, Expr, ExprTrait, Iden, JoinType, Query, SelectStatement, SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::{FromRow, Row as _, SqliteConnection, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    config::Config,
    entity::{
        MissingData,
        user::{UserEntity, mmr::update_ratings},
    },
    error::Error,
    mmr::RatingModel,
};

/// A schema for battles stored in database.
#[derive(Clone, Debug, FromRow)]
pub struct BattleEntity {
    pub id: i32,
    pub server_id: i32,
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
            if let Some(user) = p.user.as_mut() {
                user.preload_statistics(conn).await?;
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

/// Update ratings of all participants in a match.
pub async fn update_participant_ratings<T>(
    battle_id: i32,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<(), Error>
where
    T: RatingModel,
{
    #[derive(FromRow)]
    struct Query {
        id: i32,
        #[sqlx(try_from = "i32")]
        flags: UserFlags,
    }

    // Fetch players
    let players = sqlx::query_as::<_, Query>(
        r#"
        SELECT u.id, u.flags
        FROM participant p, user u
        WHERE
            p.match_id = $1
            AND p.user_id = u.id
        "#,
    )
    .bind(battle_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect::<Vec<_>>();

    // Only update if there was more than 1 participant
    if players.len() > 1 {
        let ids = players.iter().map(|s| s.id).collect::<Vec<_>>();
        let ratings = update_ratings(&ids, model, &mut *conn).await?;

        // Grant certain awards
        for (player, rating) in players.into_iter().zip(ratings) {
            // CHALLENGER MEDAL for the season
            const CHALLENGER_MEDAL: UserFlags = UserFlags::BETA_CHALLENGER;

            // Do not give awards if the player's ordinal isn't even fucking
            // visible.
            if rating.is_provisional() {
                continue;
            }

            // Only update if the player didn't already have the medal
            if !player.flags.contains(CHALLENGER_MEDAL) && rating.ordinal().ceil() >= 18000.0 {
                sqlx::query(
                    r#"
                        UPDATE user
                        SET flags = $2
                        WHERE id = $1
                        "#,
                )
                .bind(player.id)
                .bind(i32::from(player.flags | CHALLENGER_MEDAL))
                .execute(&mut *conn)
                .await?;
            }
        }
    }

    Ok(())
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
    pub name: String,
    #[sqlx(try_from = "u8")]
    pub team: PlayerTeam,
    pub finish_time: Option<i32>,
    pub no_contest: bool,
    pub skin_color: Option<String>,

    #[sqlx(skip)]
    pub user: Option<UserEntity>,
    // from skin table (on good join)
    #[sqlx(flatten)]
    pub skin: MaybeSkinEntity,
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
            name: value.name,
            team: value.team,
            finish_time: value.finish_time,
            no_contest: value.no_contest,
            skin: value.skin.into(),
            skin_color: value.skin_color,
        })
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
            inserted_at: row.try_get("user_inserted_at")?,
            updated_at: row.try_get("user_updated_at")?,
            statistics: None,
            profiles: None,
        }),
        ..participant
    })
}
