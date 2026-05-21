//! Match management routes.

pub mod player;
pub mod replay;
pub mod wager;

pub use replay::upload;

use axum::{
    Extension,
    extract::{Path, State},
};

use chrono::{DateTime, TimeDelta, Utc};

use garde::Validate;

use ring_channel_model::{
    Player,
    battle::{Battle, BattleStatus, Participant},
    request::battle::{CreateBattleRequest, UpdateBattleRequest},
};

use http::StatusCode;

use serde::Deserialize;

use sqlx::{FromRow, SqliteConnection};

use tracing::instrument;

use uuid::Uuid;

use std::fmt::Debug;

use crate::{
    app::{AppForm, AppGarde, AppJson, AppState, Model, Payload},
    auth::api_key::ServerAuthentication,
    battle::{
        BattleRow, calculate_winnings, get_replay_url, preload_participants,
        update_participant_ratings,
    },
    error::{Error, ErrorKind},
    player::mmr::{self, Rating, RawRating},
    room::BattleData,
};

/// A query for [`list`].
#[derive(Deserialize, Debug, Validate)]
#[garde(context(AppState as state))]
pub struct ListBattlesQuery {
    #[garde(range(min = 1, max = 50))]
    #[serde(default = "list_battle_count_default")]
    pub count: i32,
    #[garde(skip)]
    pub before: Option<DateTime<Utc>>,
    #[garde(skip)]
    pub after: Option<DateTime<Utc>>,
}

fn list_battle_count_default() -> i32 {
    50
}

/// Lists all matches.
#[instrument(skip(state, model))]
pub async fn list<T>(
    Extension(model): Extension<Model<T>>,
    State(state): State<AppState>,
    AppGarde(AppForm(query)): AppGarde<AppForm<ListBattlesQuery>>,
) -> Result<AppJson<Vec<Battle>>, Error>
where
    T: mmr::Model + 'static,
{
    let mut conn = state.db.acquire().await?;

    let rows = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT
            id, uuid, level_name, status, margin_score, replay_hash,
            replay_filename, inserted_at, closed_at
        FROM
            battle
        WHERE
            ($1 IS NULL OR inserted_at < $1)
            AND ($2 IS NULL OR inserted_at > $2)
        ORDER BY
            inserted_at DESC
        LIMIT $3
        "#,
    )
    .bind(query.before)
    .bind(query.after)
    .bind(query.count)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect::<Vec<_>>();

    // Preload all battles
    let mut battles = Vec::with_capacity(rows.len());
    for row in rows {
        let mut battle = Battle::from(&row);
        battle.replay_url = get_replay_url(&row, &state.config);
        preload_participants(&mut battle, &model, &mut *conn).await?;
        battles.push(battle);
    }

    Ok(AppJson(battles))
}

/// Shows an existing match.
#[instrument(skip(state, model))]
pub async fn show<T>(
    Path((uuid,)): Path<(Uuid,)>,
    Extension(model): Extension<Model<T>>,
    State(state): State<AppState>,
) -> Result<AppJson<Battle>, Error>
where
    T: mmr::Model + 'static,
{
    let mut conn = state.db.acquire().await?;

    let row = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT
            id, uuid, level_name, status, margin_score, replay_hash,
            replay_filename, inserted_at, closed_at
        FROM battle
        WHERE uuid = $1
        "#,
    )
    .bind(uuid.hyphenated().to_string())
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = row else {
        return Err(Error::not_found(format!("Match {} not found", uuid)));
    };

    // Create battle struct
    let mut battle = Battle::from(&row);

    battle.replay_url = get_replay_url(&row, &state.config);
    preload_participants(&mut battle, &model, &mut *conn).await?;

    Ok(AppJson(battle))
}

/// Creates a match.
#[instrument(skip(state, model))]
pub async fn create<T>(
    _auth_guard: ServerAuthentication,
    Extension(model): Extension<Model<T>>,
    State(state): State<AppState>,
    Payload(request): Payload<CreateBattleRequest>,
) -> Result<(StatusCode, AppJson<Battle>), Error>
where
    T: mmr::Model + 'static,
{
    #[derive(FromRow)]
    struct PlayerQuery {
        #[sqlx(rename = "player_id")]
        id: i32,
        short_id: String,
        display_name: String,
        rating: Option<f32>,
        deviation: Option<f32>,
        #[sqlx(rename = "rating_extra")]
        extra: Option<String>,
    }

    let uuid = Uuid::new_v4();
    let now = Utc::now();

    let closes_in = TimeDelta::seconds(request.bet_time.unwrap_or(20));
    let closed_at = now + closes_in;

    let mut tx = state.db.begin().await?;

    // Create the battle
    let (match_id,) = sqlx::query_as::<_, (i32,)>(
        r#"
        INSERT INTO battle (uuid, level_name, inserted_at, closed_at, status)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(uuid.hyphenated().to_string())
    .bind(&request.level_name)
    .bind(now)
    .bind(closed_at)
    .bind(u8::from(BattleStatus::Ongoing))
    .fetch_one(&mut *tx)
    .await?;

    // register players
    let mut participants = Vec::with_capacity(request.participants.len());
    for input_player in request.participants.into_iter() {
        // find player
        let player = sqlx::query_as::<_, PlayerQuery>(
            r#"
            SELECT
                p.id AS player_id,
                p.short_id,
                p.display_name,
                p.rating,
                p.deviation,
                p.rating_extra
            FROM player p
            WHERE short_id = $1
            "#,
        )
        .bind(&input_player.id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(mut player) = player {
            let rating = if !model.ratings_enabled() {
                None
            } else if let Some((rating, deviation)) = player.rating.zip(player.deviation) {
                let rating = RawRating {
                    player_id: player.id,
                    rating,
                    deviation,
                    extra: player.extra.take(),
                };

                Some(Rating::<T::Data>::try_from(rating).map_err(Error::new)?)
            } else {
                None
            };

            // add player to match
            sqlx::query(
                r#"
                INSERT INTO participant
                    (match_id, player_id, team, no_contest, skin, kart_speed, kart_weight)
                VALUES ($1, $2, $3, FALSE, $4, $5, $6)
                "#,
            )
            .bind(match_id)
            .bind(player.id)
            .bind(u8::from(input_player.team))
            .bind(&input_player.skin)
            .bind(input_player.kart_speed)
            .bind(input_player.kart_weight)
            .execute(&mut *tx)
            .await?;

            // insert players to vec
            participants.push(Participant {
                player: Player {
                    id: player.short_id,
                    mmr: rating.map(|r| r.ordinal() as i32),
                    public_key: None,
                    display_name: player.display_name,
                },
                team: input_player.team,
                finish_time: None,
                no_contest: false,
                skin: Some(input_player.skin),
                kart_speed: Some(input_player.kart_speed),
                kart_weight: Some(input_player.kart_weight),
            })
        } else {
            tx.rollback().await?;
            return Err(ErrorKind::MissingParticipant(input_player.id.clone()).into());
        }
    }

    tx.commit().await?;

    // Create battle model
    let schema = BattleRow {
        id: match_id,
        uuid: uuid.hyphenated().to_string(),
        level_name: request.level_name,
        status: BattleStatus::Ongoing,
        replay_hash: None,
        replay_filename: None,
        margin_score: 0,
        inserted_at: now,
        closed_at: closed_at,
    };

    let mut battle = Battle::from(&schema);
    battle.participants = participants.clone();
    battle.accepting_bets = true;
    battle.closes_in = Some(closes_in.num_milliseconds());

    // Send the notice of the new battle to all connected clients
    state
        .room
        .update_battle(BattleData {
            schema,
            participants,
        })
        .await;

    Ok((StatusCode::CREATED, AppJson(battle)))
}

/// Updates a match.
#[instrument(skip(state, model))]
pub async fn update<T>(
    _auth_guard: ServerAuthentication,
    Path((uuid,)): Path<(Uuid,)>,
    Extension(model): Extension<Model<T>>,
    State(state): State<AppState>,
    Payload(request): Payload<UpdateBattleRequest>,
) -> Result<AppJson<Battle>, Error>
where
    T: Debug + mmr::Model + 'static,
    T::Data: Debug,
{
    let now = Utc::now();

    let mut tx = state.db.begin().await?;

    let battle_query = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT
            id, uuid, level_name, status, margin_score, replay_hash,
            replay_filename, inserted_at, closed_at
        FROM
            battle
        WHERE
            uuid = $1
        "#,
    )
    .bind(uuid.hyphenated().to_string())
    .fetch_optional(&mut *tx)
    .await?;

    let Some(mut battle_query) = battle_query else {
        return Err(Error::not_found(format!("Match {} not found", uuid)));
    };

    // Verify changes
    let is_status_changed = request
        .status
        .map(|s| s != battle_query.status)
        .unwrap_or(false);
    if battle_query.status != BattleStatus::Ongoing {
        return Err(ErrorKind::AlreadyConcluded(uuid).into());
    }

    let mut set_concluded = None::<DateTime<Utc>>;

    // CHECK! We may need to process the end of a match here.
    if is_status_changed {
        // is_status_changed conditional gaurantees this is `Some`
        let new_status = request.status.unwrap();

        tracing::debug!("setting {} match status to {:?}", uuid, new_status);

        // Set all participants without a clear time to NO CONTEST
        sqlx::query(
            r#"
            UPDATE
                participant
            SET
                no_contest = TRUE
            WHERE
                finish_time IS NULL
                AND match_id = $1
            "#,
        )
        .bind(battle_query.id)
        .execute(&mut *tx)
        .await?;

        set_concluded = Some(now);

        // if this cancels the betting session, we need to stop accepting bets
        if now < battle_query.closed_at {
            battle_query.closed_at = now;
        }

        // Update base schema value
        battle_query.status = new_status;
    }

    // Update margin score if it is changed
    if let Some(margin_score) = request.margin_score {
        battle_query.margin_score = margin_score;
    }

    // Update match details
    sqlx::query(
        r#"
        UPDATE
            battle
        SET
            status = IFNULL($2, status),
            closed_at = $3,
            concluded_at = IFNULL($4, concluded_at),
            margin_score = IFNULL($5, margin_score)
        WHERE
            id = $1
        "#,
    )
    .bind(battle_query.id)
    .bind(request.status.map(|s| u8::from(s)))
    .bind(battle_query.closed_at)
    .bind(set_concluded)
    .bind(request.margin_score)
    .execute(&mut *tx)
    .await?;

    if request.status == Some(BattleStatus::Concluded)
        || request.status == Some(BattleStatus::Cancelled)
    {
        update_participant_ratings(battle_query.id, &model, &mut *tx).await?;
    }

    // Create battle struct
    let mut battle = Battle::from(&battle_query);

    battle.replay_url = get_replay_url(&battle_query, &state.config);
    preload_participants(&mut battle, &model, &mut *tx).await?;

    // Update websocket listeners
    state
        .room
        .update_battle(BattleData {
            schema: battle_query.clone(),
            participants: battle.participants.clone(),
        })
        .await;

    if request.status == Some(BattleStatus::Concluded) {
        // distribute pots!
        calculate_winnings(battle_query.id, &state.room, &mut *tx).await?;
    }

    tx.commit().await?;

    Ok(AppJson(battle))
}

async fn get_battle_id(match_id: Uuid, conn: &mut SqliteConnection) -> Result<i32, Error> {
    #[derive(FromRow)]
    struct BattleQuery {
        id: i32,
    }

    let battle = sqlx::query_as::<_, BattleQuery>(
        r#"
        SELECT id FROM battle WHERE uuid = $1
        "#,
    )
    .bind(match_id.hyphenated().to_string())
    .fetch_optional(&mut *conn)
    .await?;

    let Some(battle) = battle else {
        return Err(Error::not_found(format!("Match {} not found", match_id)));
    };

    Ok(battle.id)
}
