//! Placement API.

use axum::{
    Extension,
    extract::{Path, State},
};

use duelchannel_model::{
    battle::{BattleStatus, Participant},
    request::battle::UpdatePlayerPlacementRequest,
};

use sqlx::FromRow;

use tracing::instrument;

use uuid::Uuid;

use crate::{
    app::{AppJson, AppState, Model, Payload},
    auth::api_key::ServerAuthentication,
    error::{Error, ErrorKind},
    schema::{battle::get_participant_by_short_id, user::mmr},
};

/// Updates the placement of a player for a given match.
#[instrument(skip(state, model))]
pub async fn update<T>(
    _auth_guard: ServerAuthentication,
    Path((uuid, short_id)): Path<(Uuid, String)>,
    Extension(model): Extension<Model<T>>,
    State(state): State<AppState>,
    Payload(request): Payload<UpdatePlayerPlacementRequest>,
) -> Result<AppJson<Participant>, Error>
where
    T: mmr::Model + 'static,
{
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
    let participant = get_participant_by_short_id(battle.id, &short_id, &model, &mut *tx).await?;
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

    Ok(AppJson(participant.into()))
}
