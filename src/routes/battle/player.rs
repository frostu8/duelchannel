//! Placement API.

use axum::extract::{Path, State};

use duelchannel_model::{
    ApiError,
    battle::{BattleStatus, Participant},
    request::battle::UpdatePlayerPlacementRequest,
};

use sqlx::FromRow;

use tracing::instrument;

use uuid::Uuid;

use crate::{
    app::AppState,
    auth::api_key::ServerAuthentication,
    body::{Json, Payload},
    error::{Error, ErrorKind},
    entity::battle::get_participant_by_short_id,
};

/// Updates the placement of a player for a given match.
#[utoipa::path(
    patch,
    path = "/matches/{battle_id}/players/{short_id}",
    tag = "match",
    params(
        ("battle_id" = Uuid, Path, description = "The UUID of the match"),
        ("short_id" = String, Path, description = "The short ID of the player"),
    ),
    request_body = UpdatePlayerPlacementRequest,
    responses(
        (status = 200, description = "The updated participant", body = Participant),
        (status = 400, description = "Invalid request body or match already concluded", body = ApiError),
        (status = 401, description = "Missing or invalid API key", body = ApiError),
        (status = 404, description = "Match or participant not found", body = ApiError),
        (status = 415, description = "Missing or unsupported request content type", body = ApiError),
    ),
    security(("apiKey" = [])),
)]
#[instrument(skip(state))]
pub async fn update(
    _auth_guard: ServerAuthentication,
    Path((uuid, short_id)): Path<(Uuid, String)>,
    State(state): State<AppState>,
    Payload(request): Payload<UpdatePlayerPlacementRequest>,
) -> Result<Json<Participant>, Error> {
    #[derive(FromRow)]
    struct BattleRow {
        id: i32,
        #[sqlx(try_from = "u8")]
        status: BattleStatus,
    }

    let mut tx = state.db.begin().await?;

    // find match first
    let battle = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT id, status
        FROM battle
        WHERE uuid = $1
        "#,
    )
    .bind(uuid.hyphenated().to_string())
    .fetch_optional(&mut *tx)
    .await?;

    let Some(battle) = battle else {
        return Err(Error::not_found(format!("Match {} not found", uuid)));
    };

    // if the battle is closed, it cannot be updated anymore
    if battle.status != BattleStatus::Ongoing {
        return Err(ErrorKind::AlreadyConcluded(uuid).into());
    }

    // find the battle participant
    let participant = get_participant_by_short_id(battle.id, &short_id, &mut *tx).await?;
    let Some(mut participant) = participant else {
        // The player with that RRID does not exist.
        return Err(Error::not_found(format!(
            "player w/ id {} does not exist or not participating in match",
            short_id
        )));
    };

    if let Some(finish_time) = request.finish_time {
        participant.finish_time = Some(finish_time);
    }

    // UPDATE THAT SHIT KAKAROT!
    sqlx::query(
        r#"
        UPDATE participant
        SET finish_time = IFNULL($2, finish_time)
        WHERE id = $1
        "#,
    )
    .bind(participant.id)
    .bind(request.finish_time)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(Participant::try_from(participant)?))
}
