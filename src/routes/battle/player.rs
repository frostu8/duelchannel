//! Placement API.

use axum::extract::{Path, State};

use duelchannel_model::{
    ApiError,
    battle::{BattleStatus, Participant},
    request::battle::UpdatePlayerPlacementRequest,
};

use tracing::instrument;

use uuid::Uuid;

use crate::{
    app::AppState,
    auth::api_key::ServerAuthentication,
    body::{Json, Payload},
    entity::battle::get_participant_by_short_id,
    error::{Error, ErrorKind},
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
    let mut tx = state.db.begin().await?;

    // find match first
    let row = sqlx::query_as::<_, (i32, u8)>(
        r#"
        SELECT id, status
        FROM battle
        WHERE uuid = $1
        "#,
    )
    .bind(uuid.hyphenated().to_string())
    .fetch_optional(&mut *tx)
    .await
    .map_err(Error::from)
    .and_then(|row| {
        row.map(|(id, status)| Ok((id, BattleStatus::try_from(status).map_err(Error::new)?)))
            .transpose()
    })?;

    let Some((battle_id, status)) = row else {
        return Err(Error::not_found(format!("Match {} not found", uuid)));
    };

    // if the battle is closed, it cannot be updated anymore
    if status != BattleStatus::Ongoing {
        return Err(ErrorKind::AlreadyConcluded(uuid).into());
    }

    // find the battle participant
    let participant = get_participant_by_short_id(battle_id, &short_id, &mut *tx).await?;
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
    if let Some(roulette) = request.roulette {
        // Append roulette
        participant.extend_roulette(roulette, &mut *tx).await?;
    } else {
        // Remember to preload the roulette
        participant.preload_roulette(&mut *tx).await?;
    }

    // UPDATE THAT SHIT KAKAROT!
    let res = sqlx::query(
        r#"
        UPDATE participant
        SET finish_time = IFNULL($3, finish_time)
        WHERE id = $1 AND match_id = $2
        "#,
    )
    .bind(participant.id)
    .bind(battle_id)
    .bind(request.finish_time)
    .execute(&mut *tx)
    .await?;

    // Check to make this skin fuckery never happen again
    assert!(res.rows_affected() > 0);

    tx.commit().await?;

    Ok(Json(Participant::try_from(participant)?))
}
