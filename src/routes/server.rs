//! Server operations.

use std::collections::HashMap;

use axum::extract::State;

use chrono::Utc;
use duelchannel_model::{
    request::server::UpdateServerRequest,
    server::{BannedStatus, MapConfig, Server},
};
use sqlx::{FromRow, SqliteConnection};

use crate::{
    app::{AppJson, AppState, Payload},
    auth::api_key::ServerAuthentication,
    error::Error,
};

#[derive(FromRow)]
struct MapConfigQuery {
    pub lumpname: String,
    #[sqlx(try_from = "u8")]
    pub status: BannedStatus,
    pub note: Option<String>,
}

/// Gets the current server.
pub async fn show_self(
    auth: ServerAuthentication,
    State(state): State<AppState>,
) -> Result<AppJson<Server>, Error> {
    let mut conn = state.db.acquire().await.map_err(Error::new)?;

    let mut server = Server {
        id: auth.id,
        name: auth.server_name,
        bans: HashMap::new(),
    };

    preload_map_configs(&mut server, &mut *conn).await?;

    Ok(AppJson(server))
}

/// Updates the current server.
pub async fn update(
    auth: ServerAuthentication,
    State(state): State<AppState>,
    Payload(mut request): Payload<UpdateServerRequest>,
) -> Result<AppJson<Server>, Error> {
    let mut tx = state.db.begin().await.map_err(Error::new)?;

    let now = Utc::now();

    // Fetch current server information.
    let mut to_commit = false;
    let mut server = Server {
        id: auth.id,
        name: auth.server_name,
        bans: HashMap::new(),
    };

    preload_map_configs(&mut server, &mut *tx).await?;

    if let Some(name) = request.name.take() {
        server.name = name;
        to_commit = true;
    }

    if to_commit {
        // Write changes
        sqlx::query(
            r#"
            UPDATE server
            SET server_name = $3, updated_at = $2
            WHERE id = $1
            "#,
        )
        .bind(server.id)
        .bind(now)
        .bind(&server.name)
        .execute(&mut *tx)
        .await
        .map_err(Error::new)?;
    }

    // Apply bans if applicable
    if let Some(bans) = request.bans {
        let mut new_bans = HashMap::with_capacity(server.bans.len());

        for (lumpname, config) in bans {
            if let Some(old_ban) = server.bans.remove(&lumpname) {
                // Update old ban info
                if old_ban != config {
                    sqlx::query(
                        r#"
                        UPDATE map_config
                        SET updated_at = $1, status = $4, note = $5
                        WHERE lumpname = $2 AND parent_id = $3
                        "#,
                    )
                    .bind(now)
                    .bind(&lumpname)
                    .bind(server.id)
                    .bind(u8::from(config.status))
                    .bind(config.note.as_ref())
                    .execute(&mut *tx)
                    .await
                    .map_err(Error::new)?;
                }
            } else {
                // This is a fresh ban
                sqlx::query(
                    r#"
                    INSERT INTO map_config (parent_id, lumpname, status, note, inserted_at, updated_at)
                    VALUES ($2, $3, $4, $5, $1, $1)
                    "#
                )
                .bind(now)
                .bind(server.id)
                .bind(&lumpname)
                .bind(u8::from(config.status))
                .bind(config.note.as_ref())
                .execute(&mut *tx)
                .await
                .map_err(Error::new)?;
            }

            new_bans.insert(lumpname, config);
        }

        // Empty old bans
        std::mem::swap(&mut server.bans, &mut new_bans);
        for (lumpname, _) in new_bans {
            sqlx::query(
                r#"
                DELETE FROM map_config
                WHERE lumpname = $1 AND parent_id = $2
                "#,
            )
            .bind(lumpname)
            .bind(server.id)
            .execute(&mut *tx)
            .await
            .map_err(Error::new)?;
        }
    }

    tx.commit().await.map_err(Error::new)?;

    Ok(AppJson(server))
}

async fn preload_map_configs(
    server: &mut Server,
    conn: &mut SqliteConnection,
) -> Result<(), Error> {
    let res = sqlx::query_as::<_, MapConfigQuery>(
        r#"
        SELECT *
        FROM map_config mc
        WHERE mc.parent_id = $1
        "#,
    )
    .bind(server.id)
    .fetch_all(&mut *conn)
    .await
    .map_err(Error::new)?;

    for row in res {
        server.bans.insert(
            row.lumpname,
            MapConfig {
                status: row.status,
                note: row.note,
            },
        );
    }

    Ok(())
}
