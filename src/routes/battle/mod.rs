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
    ApiError, User,
    battle::{Battle, BattleStatus, Participant},
    profile::Skin,
    request::battle::{CreateBattleRequest, UpdateBattleRequest},
};

use http::StatusCode;

use serde::Deserialize;

use sqlx::{SqliteConnection, SqlitePool};

use tracing::instrument;

use utoipa::IntoParams;

use uuid::Uuid;

use std::{collections::HashSet, fmt::Debug};

use crate::{
    app::AppState,
    auth::api_key::ServerAuthentication,
    body::{Form, Json, Payload},
    entity::{
        battle::{BattleEntity, analytics::get_analytics, build_battle, get_replay_url},
        user::{get_user_by_public_key, mmr::RatingService},
    },
    error::{Error, ErrorKind},
    validate::Valid,
};

/// A query for [`list`].
#[derive(Deserialize, Debug, Validate, IntoParams)]
#[garde(context(AppState as state))]
pub struct ListBattlesQuery {
    /// The maximum number of matches to return.
    #[garde(range(min = 1, max = 50))]
    #[serde(default = "list_battle_count_default")]
    #[param(minimum = 1, maximum = 50, default = 50)]
    pub count: i32,
    /// Only return matches inserted before this time.
    #[garde(skip)]
    #[param(value_type = String, format = "date-time")]
    pub before: Option<DateTime<Utc>>,
    /// Only return matches inserted after this time.
    #[garde(skip)]
    #[param(value_type = String, format = "date-time")]
    pub after: Option<DateTime<Utc>>,
}

fn list_battle_count_default() -> i32 {
    50
}

/// Lists all matches.
#[utoipa::path(
    get,
    path = "/matches",
    tag = "match",
    params(ListBattlesQuery),
    responses(
        (status = 200, description = "A list of matches", body = Vec<Battle>),
        (status = 400, description = "Invalid query parameters", body = ApiError),
    ),
)]
#[instrument(skip(state))]
pub async fn list(
    State(state): State<AppState>,
    Valid(Form(query)): Valid<Form<ListBattlesQuery>>,
) -> Result<Json<Vec<Battle>>, Error> {
    let mut conn = state.db.acquire().await?;

    let rows = sqlx::query_as::<_, BattleEntity>(
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
    for mut battle in rows {
        // Create battle response
        battle.preload_participants(&mut conn).await?;
        let replay_url = get_replay_url(&battle, &state.config);

        let mut battle = Battle::try_from(battle)?;
        battle.replay_url = replay_url;

        battles.push(battle);
    }

    Ok(Json(battles))
}

/// Shows an existing match.
#[utoipa::path(
    get,
    path = "/matches/{battle_id}",
    tag = "match",
    params(
        ("battle_id" = Uuid, Path, description = "The UUID of the match"),
    ),
    responses(
        (status = 200, description = "The match", body = Battle),
        (status = 404, description = "Match not found", body = ApiError),
    ),
)]
#[instrument(skip(state))]
pub async fn show(
    Path((uuid,)): Path<(Uuid,)>,
    State(state): State<AppState>,
) -> Result<Json<Battle>, Error> {
    let mut conn = state.db.acquire().await?;

    let battle = sqlx::query_as::<_, BattleEntity>(
        r#"
        SELECT b.*
        FROM battle b
        WHERE uuid = $1
        "#,
    )
    .bind(uuid.hyphenated().to_string())
    .fetch_optional(&mut *conn)
    .await?;

    let Some(mut battle) = battle else {
        return Err(Error::not_found(format!("Match {} not found", uuid)));
    };

    // Create battle response
    battle.preload_participants(&mut conn).await?;
    let replay_url = get_replay_url(&battle, &state.config);

    let mut battle = Battle::try_from(battle)?;
    battle.replay_url = replay_url;

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
#[utoipa::path(
    post,
    path = "/matches",
    tag = "match",
    request_body = CreateBattleRequest,
    responses(
        (status = 201, description = "The match was created", body = Battle),
        (status = 400, description = "Invalid request body, missing profile, or duplicate/missing participant", body = ApiError),
        (status = 401, description = "Missing or invalid API key", body = ApiError),
        (status = 415, description = "Missing or unsupported request content type", body = ApiError),
    ),
    security(("apiKey" = [])),
)]
#[instrument(skip(state, model))]
pub async fn create<T>(
    server_auth: ServerAuthentication,
    Extension(model): Extension<T>,
    State(state): State<AppState>,
    Payload(request): Payload<CreateBattleRequest>,
) -> Result<(StatusCode, Json<Battle>), Error>
where
    T: RatingService + Clone + Send + Sync + 'static,
{
    let mut tx = state.db.begin().await?;
    let now = Utc::now();

    // Create the battle
    let battle = build_battle(request.level_name)
        .server_id(server_auth.id)
        .timestamp(now)
        .create(&mut *tx)
        .await?;
    let battle_id = battle.id;

    // register players
    let mut short_ids = HashSet::new();

    let mut participants = Vec::with_capacity(request.participants.len());
    for input_player in request.participants.into_iter() {
        let profile_user = get_user_by_public_key(&input_player.public_key, &mut *tx).await?;
        let Some(mut profile_user) = profile_user else {
            tx.rollback().await?;
            return Err(ErrorKind::MissingProfile(input_player.public_key).into());
        };
        profile_user.preload_statistics(&mut *tx).await?;

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
        .bind(battle_id)
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
            user: User::try_from(profile_user)?,
            roulette: Vec::new(),
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
    let battle = Battle {
        participants: participants,
        ..Battle::try_from(battle)?
    };

    // Commit analytics
    let db_clone = state.db.clone();
    let model_clone = model.clone();
    tokio::spawn(async move {
        if let Err(err) = flush_analytics(battle_id, &model_clone, db_clone).await {
            tracing::error!("got error flushing analytics: {}", err);
        }
    });

    Ok((StatusCode::CREATED, Json(battle)))
}

/// Updates a match.
#[utoipa::path(
    patch,
    path = "/matches/{battle_id}",
    tag = "match",
    params(
        ("battle_id" = Uuid, Path, description = "The UUID of the match"),
    ),
    request_body = UpdateBattleRequest,
    responses(
        (status = 200, description = "The updated match", body = Battle),
        (status = 400, description = "Invalid request body or match already concluded", body = ApiError),
        (status = 401, description = "Missing or invalid API key", body = ApiError),
        (status = 404, description = "Match not found", body = ApiError),
        (status = 415, description = "Missing or unsupported request content type", body = ApiError),
    ),
    security(("apiKey" = [])),
)]
#[instrument(skip(state, model))]
pub async fn update<T>(
    _auth_guard: ServerAuthentication,
    Path((uuid,)): Path<(Uuid,)>,
    Extension(model): Extension<T>,
    State(state): State<AppState>,
    Payload(request): Payload<UpdateBattleRequest>,
) -> Result<Json<Battle>, Error>
where
    T: RatingService + Clone + Send + Sync + 'static,
{
    let now = Utc::now();

    let mut tx = state.db.begin().await?;

    let battle = sqlx::query_as::<_, BattleEntity>(
        r#"
        SELECT b.*
        FROM battle b
        WHERE uuid = $1
        "#,
    )
    .bind(uuid.hyphenated().to_string())
    .fetch_optional(&mut *tx)
    .await?;

    let Some(mut battle) = battle else {
        return Err(Error::not_found(format!("Match {} not found", uuid)));
    };

    battle.preload_participants(&mut *tx).await?;
    let battle_id = battle.id;

    // Verify changes
    let is_status_changed = request.status.map(|s| s != battle.status).unwrap_or(false);
    if battle.status != BattleStatus::Ongoing {
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
        .bind(battle.id)
        .execute(&mut *tx)
        .await?;

        set_concluded = Some(now);

        // Update base schema value
        battle.status = new_status;
    }

    // Update margin score if it is changed
    if let Some(margin_score) = request.margin_score {
        battle.margin_score = margin_score;
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
    .bind(battle.id)
    .bind(now)
    .bind(request.status.map(|s| u8::from(s)))
    .bind(set_concluded)
    .bind(request.margin_score)
    .execute(&mut *tx)
    .await?;

    if request.status == Some(BattleStatus::Concluded)
        || request.status == Some(BattleStatus::Cancelled)
    {
        let participants = battle.participants.as_ref().expect("preloaded");
        let user_ids = participants
            .into_iter()
            .map(|p| p.user_id)
            .collect::<Vec<_>>();

        model
            .update_ratings(user_ids.as_slice(), &state.config, &mut *tx)
            .await?;
    }

    // Create battle response
    let replay_url = get_replay_url(&battle, &state.config);

    let mut battle = Battle::try_from(battle)?;
    battle.replay_url = replay_url;

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
async fn flush_analytics<T>(battle_id: i32, model: &T, db: SqlitePool) -> Result<(), Error>
where
    T: RatingService + Clone + Send + Sync,
{
    tracing::debug!("flushing analytics");
    let mut conn = db.acquire().await?;
    get_analytics(battle_id, model, &mut *conn).await?;
    Ok(())
}
