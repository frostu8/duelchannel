//! Analytics!

use axum::{extract::State, response::IntoResponse};
use axum_streams::StreamBodyAs;
use duelchannel_model::battle::BattlePoint;
use futures_util::{StreamExt, stream};
use tokio::sync::mpsc::unbounded_channel;

use crate::{app::AppState, entity::battle::analytics::stream_analytics, error::Error};

/// Fetches analytics about battles.
///
/// Streams a JSON array of battle statistics points.
#[utoipa::path(
    get,
    path = "/matches/analytics",
    tag = "match",
    responses(
        (status = 200, description = "A stream of battle statistics", body = Vec<BattlePoint>),
    ),
)]
pub async fn show(State(state): State<AppState>) -> Result<impl IntoResponse, Error> {
    let (tx, rx) = unbounded_channel();

    let mut conn = state.db.acquire().await?;
    tokio::spawn(async move {
        let mut stream = stream_analytics(&mut conn);
        while let Some(res) = stream.next().await {
            match res {
                Ok(data) if !data.statistics.is_empty() => {
                    if let Err(_) = tx.send(data) {
                        break;
                    }
                }
                // skip empty points
                Ok(_data) => (),
                Err(err) => {
                    tracing::warn!("got error while streaming: {}", err);
                }
            }
        }
    });

    Ok(StreamBodyAs::json_array(stream::unfold(
        rx,
        |mut rx| async move { rx.recv().await.map(|s| (s, rx)) },
    )))
}
