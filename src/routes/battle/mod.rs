//! Match management routes.

pub mod analytics;
pub mod player;
pub mod replay;

pub use replay::upload;

use axum::{
    Extension,
    extract::{Path, State},
};

use chrono::{DateTime, Utc};

use garde::Validate;

use duelchannel_model::{
    User,
    battle::{Battle, BattleStatus, Participant},
    profile::Skin,
    request::battle::{CreateBattleRequest, UpdateBattleRequest},
};

use http::StatusCode;

use serde::Deserialize;

use sqlx::{SqliteConnection, SqlitePool};

use tracing::instrument;

use uuid::Uuid;

use std::{collections::HashSet, fmt::Debug};

use crate::{
    app::{AppState, Model, ModelOrUnrated},
    auth::api_key::ServerAuthentication,
    body::{Form, Json, Payload},
    error::{Error, ErrorKind},
    schema::{
        battle::{
            BattleRow, analytics::get_analytics, get_replay_url, preload_participants,
            update_participant_ratings,
        },
        user::{get_user_by_public_key, mmr},
    },
    validate::Valid,
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
#[instrument(skip(state))]
pub async fn list(
    State(state): State<AppState>,
    Valid(Form(query)): Valid<Form<ListBattlesQuery>>,
) -> Result<Json<Vec<Battle>>, Error> {
    let mut conn = state.db.acquire().await?;

    let rows = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT b.*
        FROM battle b
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
        preload_participants(&mut battle, &mut *conn).await?;
        battles.push(battle);
    }

    Ok(Json(battles))
}

/// Shows an existing match.
#[instrument(skip(state))]
pub async fn show(
    Path((uuid,)): Path<(Uuid,)>,
    State(state): State<AppState>,
) -> Result<Json<Battle>, Error> {
    let mut conn = state.db.acquire().await?;

    let row = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT b.*
        FROM battle b
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
    preload_participants(&mut battle, &mut *conn).await?;

    Ok(Json(battle))
}

async fn upsert_skin(skin: &Skin, conn: &mut SqliteConnection) -> Result<(), Error> {
    sqlx::query(
        r#"
        INSERT INTO skin (name, realname, kartspeed, kartweight)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT DO UPDATE
        SET
            realname = $2,
            kartspeed = $3,
            kartweight = $4
        "#,
    )
    .bind(&skin.name)
    .bind(&skin.real_name)
    .bind(&skin.kart_speed)
    .bind(&skin.kart_weight)
    .execute(&mut *conn)
    .await
    .map(|_| ())
    .map_err(Error::from)
}

/// Creates a match.
#[instrument(skip(state, model))]
pub async fn create<T>(
    server_auth: ServerAuthentication,
    Extension(model): Extension<Model<T>>,
    State(state): State<AppState>,
    Payload(request): Payload<CreateBattleRequest>,
) -> Result<(StatusCode, Json<Battle>), Error>
where
    T: ModelOrUnrated + Clone + Send + Sync,
    <T as ModelOrUnrated>::Model: mmr::Model + Debug,
    <<T as ModelOrUnrated>::Model as mmr::Model>::Data: Debug + Clone,
{
    let now = Utc::now();

    let mut tx = state.db.begin().await?;

    // Generate new UUID
    let uuid = Uuid::new_v4();

    // Create the battle
    let (match_id,) = sqlx::query_as::<_, (i32,)>(
        r#"
        INSERT INTO battle (inserted_at, updated_at, server_id, uuid, level_name, status)
        VALUES ($1, $1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(now)
    .bind(server_auth.id)
    .bind(uuid.hyphenated().to_string())
    .bind(&request.level_name)
    .bind(u8::from(BattleStatus::Ongoing))
    .fetch_one(&mut *tx)
    .await?;

    // register players
    let mut short_ids = HashSet::new();

    let mut participants = Vec::with_capacity(request.participants.len());
    for input_player in request.participants.into_iter() {
        let profile_user = get_user_by_public_key(&input_player.public_key, &mut *tx).await?;
        let Some(profile_user) = profile_user else {
            tx.rollback().await?;
            return Err(ErrorKind::MissingProfile(input_player.public_key).into());
        };

        if short_ids.contains(&input_player.user_id) {
            return Err(ErrorKind::DuplicateParticipant(input_player.user_id).into());
        }

        let user = sqlx::query_as::<_, (i32,)>(
            r#"
            SELECT id
            FROM user
            WHERE short_id = $1
            "#,
        )
        .bind(&input_player.user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((user_id,)) = user else {
            tx.rollback().await?;
            return Err(ErrorKind::MissingParticipant(input_player.user_id).into());
        };

        if let Some(skin) = input_player.skin.as_ref() {
            upsert_skin(skin, &mut *tx).await?;
        }

        // add player to match
        sqlx::query(
            r#"
            INSERT INTO participant (
                profile_id,
                match_id,
                user_id,
                name,
                team,
                skin,
                skin_color
            )
            SELECT p.id, $2, $3, $4, $5, $6, $7
            FROM profile p
            WHERE p.public_key = $1
            "#,
        )
        .bind(input_player.public_key.as_bytes())
        .bind(match_id)
        .bind(user_id)
        .bind(&input_player.name)
        .bind(u8::from(input_player.team))
        .bind(input_player.skin.as_ref().map(|s| &s.name))
        .bind(input_player.skin_color.as_ref())
        .execute(&mut *tx)
        .await?;

        // Track what short IDs we have seen
        short_ids.insert(input_player.user_id);

        // insert players to vec
        participants.push(Participant {
            user: User::from(profile_user),
            name: input_player.name,
            team: input_player.team,
            finish_time: None,
            no_contest: false,
            skin: input_player.skin,
            skin_color: input_player.skin_color,
        });
    }

    tx.commit().await?;

    // Create battle model
    let schema = BattleRow {
        id: match_id,
        server_id: server_auth.id,
        uuid: uuid.hyphenated().to_string(),
        level_name: request.level_name,
        status: BattleStatus::Ongoing,
        replay_hash: None,
        replay_filename: None,
        margin_score: 0,
        concluded_at: None,
        inserted_at: now,
        updated_at: now,
    };
    let mut battle = Battle::from(schema);
    battle.participants = participants;

    // Commit analytics
    let db_clone = state.db.clone();
    let model_clone = model.clone();
    tokio::spawn(async move {
        if let Err(err) = flush_analytics(match_id, &model_clone, db_clone).await {
            tracing::error!("got error flushing analytics: {}", err);
        }
    });

    Ok((StatusCode::CREATED, Json(battle)))
}

/// Updates a match.
#[instrument(skip(state, model))]
pub async fn update<T>(
    _auth_guard: ServerAuthentication,
    Path((uuid,)): Path<(Uuid,)>,
    Extension(model): Extension<Model<T>>,
    State(state): State<AppState>,
    Payload(request): Payload<UpdateBattleRequest>,
) -> Result<Json<Battle>, Error>
where
    T: ModelOrUnrated + Clone + Send + Sync,
    <T as ModelOrUnrated>::Model: mmr::Model + Debug,
    <<T as ModelOrUnrated>::Model as mmr::Model>::Data: Debug + Clone,
{
    let now = Utc::now();

    let mut tx = state.db.begin().await?;

    let battle_query = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT b.*
        FROM battle b
        WHERE uuid = $1
        "#,
    )
    .bind(uuid.hyphenated().to_string())
    .fetch_optional(&mut *tx)
    .await?;

    let Some(mut battle_query) = battle_query else {
        return Err(Error::not_found(format!("Match {} not found", uuid)));
    };
    let battle_id = battle_query.id;

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
            UPDATE participant
            SET no_contest = TRUE
            WHERE
                finish_time IS NULL
                AND match_id = $1
            "#,
        )
        .bind(battle_query.id)
        .execute(&mut *tx)
        .await?;

        set_concluded = Some(now);

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
            updated_at = $2,
            status = IFNULL($3, status),
            concluded_at = IFNULL($4, concluded_at),
            margin_score = IFNULL($5, margin_score)
        WHERE
            id = $1
        "#,
    )
    .bind(battle_query.id)
    .bind(now)
    .bind(request.status.map(|s| u8::from(s)))
    .bind(set_concluded)
    .bind(request.margin_score)
    .execute(&mut *tx)
    .await?;

    if let Some(model) = model.model() {
        if request.status == Some(BattleStatus::Concluded)
            || request.status == Some(BattleStatus::Cancelled)
        {
            update_participant_ratings(battle_query.id, model, &mut *tx).await?;
        }
    }

    // Create battle struct
    let mut battle = Battle::from(&battle_query);

    battle.replay_url = get_replay_url(&battle_query, &state.config);
    preload_participants(&mut battle, &mut *tx).await?;

    tx.commit().await?;

    // Commit analytics
    let db_clone = state.db.clone();
    let model_clone = model.clone();
    tokio::spawn(async move {
        if let Err(err) = flush_analytics(battle_id, &model_clone, db_clone).await {
            tracing::error!("got error flushing analytics: {}", err);
        }
    });

    Ok(Json(battle))
}

#[instrument(skip(model, db))]
async fn flush_analytics<T>(battle_id: i32, model: &Model<T>, db: SqlitePool) -> Result<(), Error>
where
    T: ModelOrUnrated,
    <T as ModelOrUnrated>::Model: mmr::Model + Debug,
    <<T as ModelOrUnrated>::Model as mmr::Model>::Data: Debug + Clone,
{
    tracing::debug!("flushing analytics");
    let mut conn = db.acquire().await?;
    get_analytics(battle_id, model, &mut *conn).await?;
    Ok(())
}
