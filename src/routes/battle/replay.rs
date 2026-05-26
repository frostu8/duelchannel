//! Replay object storage.

use std::io::Write;

use axum::extract::{Path, State};

use bytes::Bytes;
use futures_util::StreamExt as _;

use duelchannel_model::Battle;

use sha2::{Digest as _, Sha256};

use uuid::Uuid;

use crate::{
    app::AppState,
    auth::api_key::ServerAuthentication,
    body::Json,
    error::{Error, ErrorKind},
    multipart::Multipart,
    schema::battle::{BattleRow, get_replay_url, preload_participants},
};

const MAX_REPLAY_SIZE: usize = 1024 * 1024 * 4;

/// Accepts a replay.
pub async fn upload(
    _auth_guard: ServerAuthentication,
    Path((match_id,)): Path<(Uuid,)>,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<Battle>, Error> {
    let mut tx = state.db.begin().await.map_err(Error::new)?;

    // Get associated battle
    let battle = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT *
        FROM battle b
        WHERE b.uuid = $1
        "#,
    )
    .bind(match_id.hyphenated().to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(mut row) = battle else {
        return Err(Error::not_found(format!("Match {} not found", match_id)));
    };

    let Some(mut field) = multipart.next_field().await? else {
        return Err(ErrorKind::InvalidData(format!("no `replay` field given")).into());
    };

    if field.name() != Some("replay") {
        return Err(ErrorKind::InvalidData(format!("expected `replay` field")).into());
    }

    // Begin consuming multipart file
    let Some(filename) = field.file_name().map(|s| s.to_owned()) else {
        return Err(ErrorKind::InvalidData(format!("expected `replay` field to be a file")).into());
    };

    // Files are expected to be less than 4M, so we can store it in memory
    let mut replay_data = Vec::<u8>::new();

    // Write to memory while calculating hash
    let mut hash = Sha256::new();
    while let Some(chunk) = field.next().await {
        let chunk = chunk.map_err(ErrorKind::MultipartParse)?;

        if replay_data.len() + chunk.len() > MAX_REPLAY_SIZE {
            return Err(ErrorKind::ReplayTooLarge.into());
        }

        hash.write(&chunk).map_err(Error::new)?;
        replay_data.extend(&chunk);
    }

    // Calculate hash
    let hash = base16::encode_lower(&hash.finalize());

    // If the battle already had a replay, delete the old replay
    if let Some((replay_hash, replay_filename)) =
        row.replay_hash.take().zip(row.replay_filename.take())
    {
        let s3_path = format!("{}/{}", replay_hash, replay_filename);
        state
            .object_storage
            .delete(&s3_path)
            .await
            .map_err(Error::new)?;
    }

    // Move file to a path expected by the app
    let s3_path = format!("{}/{}", hash, filename);
    state
        .object_storage
        .write(&s3_path, Bytes::from(replay_data))
        .await
        .map_err(Error::new)?;

    // Update database
    sqlx::query(
        r#"
        UPDATE battle
        SET replay_hash = $2, replay_filename = $3
        WHERE id = $1
        "#,
    )
    .bind(row.id)
    .bind(&hash)
    .bind(&filename)
    .execute(&mut *tx)
    .await?;

    // Replace old data
    row.replay_hash = Some(hash);
    row.replay_filename = Some(filename);

    let mut battle = Battle::from(&row);

    battle.replay_url = get_replay_url(&row, &state.config);
    preload_participants(&mut battle, &mut *tx).await?;

    tx.commit().await.map_err(Error::new)?;

    Ok(Json(battle))
}
