//! Battle functions and utilities.

use std::fmt::Debug;

use chrono::{DateTime, Utc};

use duelchannel_model::{
    battle::{Battle, BattleStatus, Participant, PlayerTeam},
    profile::Skin,
    user::{User, UserFlags},
};

use sqlx::{FromRow, SqliteConnection};

use crate::{
    config::Config,
    error::Error,
    schema::user::mmr::{Model, update_ratings},
};

/// A schema for battles stored in database.
///
/// Used primarily to construct [`Battle`]s.
#[derive(Clone, Debug, FromRow)]
pub struct BattleRow {
    pub id: i32,
    pub server_id: i32,
    pub uuid: String,
    pub level_name: String,
    #[sqlx(try_from = "u8")]
    pub status: BattleStatus,
    pub margin_score: i32,
    pub replay_hash: Option<String>,
    pub replay_filename: Option<String>,
    pub concluded_at: Option<DateTime<Utc>>,
    pub inserted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<BattleRow> for Battle {
    fn from(value: BattleRow) -> Self {
        (&value).into()
    }
}

impl From<&BattleRow> for Battle {
    fn from(value: &BattleRow) -> Self {
        Battle {
            id: value.uuid.clone(),
            level_name: value.level_name.clone(),
            participants: vec![],
            status: value.status,
            margin_score: value.margin_score,
            replay_url: None,
            started_at: value.inserted_at,
        }
    }
}

/// Update ratings of all participants in a match.
pub async fn update_participant_ratings<T>(
    battle_id: i32,
    model: &T,
    conn: &mut SqliteConnection,
) -> Result<(), Error>
where
    T: Model + Debug,
    T::Data: Debug,
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

            // Only update if the player didn't already have the medal
            if !player.flags.contains(CHALLENGER_MEDAL) && rating.ordinal().ceil() >= 2000.0 {
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
pub fn get_replay_url(battle: &BattleRow, config: &Config) -> Option<String> {
    battle
        .replay_hash
        .as_ref()
        .zip(battle.replay_filename.as_ref())
        .map(|(hash, filename)| format!("{}/{}/{}", config.cdn.base_url, hash, filename))
}

/// Represents a possibly failed left join.
#[derive(Clone, FromRow)]
pub struct MaybeSkin {
    #[sqlx(rename = "skin")]
    name: Option<String>,
    realname: Option<String>,
    kartspeed: Option<i32>,
    kartweight: Option<i32>,
}

impl From<MaybeSkin> for Option<Skin> {
    fn from(value: MaybeSkin) -> Option<Skin> {
        Some(Skin {
            name: value.name?,
            real_name: value.realname?,
            kart_speed: value.kartspeed?,
            kart_weight: value.kartweight?,
        })
    }
}

/// A single participant.
#[derive(Clone, FromRow)]
pub struct ParticipantRow {
    // from participants
    pub id: i32,
    pub name: String,
    #[sqlx(try_from = "u8")]
    pub team: PlayerTeam,
    pub finish_time: Option<i32>,
    pub no_contest: bool,
    // from player table
    pub short_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    #[sqlx(try_from = "i32")]
    pub flags: UserFlags,
    pub ordinal: Option<i32>,
    // from skin table (on good join)
    #[sqlx(flatten)]
    pub skin: MaybeSkin,
}

impl From<ParticipantRow> for Participant {
    fn from(value: ParticipantRow) -> Participant {
        Participant {
            user: User {
                id: value.short_id,
                mmr: value.ordinal,
                display_name: value.display_name,
                avatar_url: value.avatar_url,
                flags: value.flags,
                profiles: None,
            },
            name: value.name,
            team: value.team,
            finish_time: value.finish_time,
            no_contest: value.no_contest,
            skin: value.skin.into(),
        }
    }
}

impl From<&ParticipantRow> for Participant {
    fn from(value: &ParticipantRow) -> Participant {
        value.clone().into()
    }
}

/// Fetches a single participant by their short_id.
pub async fn get_participant_by_short_id(
    battle_id: i32,
    short_id: &str,
    conn: &mut SqliteConnection,
) -> Result<Option<ParticipantRow>, Error> {
    sqlx::query_as::<_, ParticipantRow>(
        r#"
        SELECT
            pt.*,
            u.short_id,
            u.display_name,
            u.avatar_url,
            u.flags,
            u.ordinal,
            s.realname,
            s.kartspeed,
            s.kartweight
        FROM
            participant pt, profile pr, user u
        LEFT OUTER JOIN
            skin s ON pt.skin = s.name
        WHERE
            u.short_id = $1
            AND pt.profile_id = pr.id
            AND u.id = pr.parent_id
            AND pt.match_id = $2
        "#,
    )
    .bind(short_id)
    .bind(&battle_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(Error::from)
}

/// Preloads the `participants` field of a [`Battle`].
///
/// If this function fails, `battle` will not be modified.
pub async fn preload_participants(
    battle: &mut Battle,
    conn: &mut SqliteConnection,
) -> Result<(), Error> {
    let participants = sqlx::query_as::<_, ParticipantRow>(
        r#"
        SELECT
            pt.*,
            u.short_id,
            u.display_name,
            u.avatar_url,
            u.flags,
            u.ordinal,
            s.realname,
            s.kartspeed,
            s.kartweight
        FROM
            participant pt, battle b, profile pr, user u
        LEFT OUTER JOIN
            skin s ON pt.skin = s.name
        WHERE
            pt.match_id = b.id
            AND pt.profile_id = pr.id
            AND u.id = pr.parent_id
            AND b.uuid = $1
        "#,
    )
    .bind(&battle.id)
    .fetch_all(&mut *conn)
    .await?;

    battle.participants = participants
        .into_iter()
        .map(Participant::from)
        .collect::<Vec<_>>();

    Ok(())
}
