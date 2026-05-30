//! Server operations.

use std::collections::HashMap;

use axum::extract::State;

use chrono::Utc;
use duelchannel_model::server::{BannedStatus, MapConfig, Server};
use garde::Validate;
use serde::Deserialize;
use sqlx::{FromRow, SqliteConnection};
use tracing::instrument;

use crate::{
    app::AppState,
    auth::api_key::ServerAuthentication,
    body::{Form, Json, Payload},
    error::Error,
    validate::Valid,
};

/// An update server request.
#[derive(Clone, Debug, Deserialize, Validate)]
#[garde(context(AppState as state))]
pub struct UpdateServerRequest {
    /// The new name of the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(length(min = 1, max = 255))]
    pub name: Option<String>,
    /// The list of map bans.
    ///
    /// These are replaced as-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub maps: Option<HashMap<String, UpdateMapConfig>>,
}

/// Map config in [`UpdateServerRequest`].
#[derive(Clone, Debug, Deserialize, Validate)]
#[garde(context(AppState as state))]
pub struct UpdateMapConfig {
    /// The status of the map.
    #[garde(skip)]
    pub status: BannedStatus,
    /// A modified wincon for the map.
    #[serde(default)]
    #[garde(range(min = 1, max = 255))]
    pub win_condition: Option<i32>,
    /// A skill range the map config targets.
    #[serde(default)]
    #[garde(dive)]
    pub skill_range: SkillRange,
    /// A user-defined note.
    #[serde(default)]
    #[garde(length(min = 0, max = 2048))]
    pub note: Option<String>,
}

impl From<UpdateMapConfig> for MapConfig {
    fn from(value: UpdateMapConfig) -> Self {
        MapConfig {
            status: value.status,
            win_condition: value.win_condition,
            skill_range: duelchannel_model::server::SkillRange {
                lower: value.skill_range.lower,
                upper: value.skill_range.upper,
            },
            note: value.note,
        }
    }
}

/// A range of MMRs.
#[derive(Clone, Debug, Default, Deserialize, Validate)]
#[garde(context(AppState as state))]
pub struct SkillRange {
    /// The lower bound of the range.
    #[garde(range(min = 0, max = 9999))]
    pub lower: Option<i32>,
    /// The upper bound of the range.
    #[garde(range(min = 0, max = 9999))]
    pub upper: Option<i32>,
}

/// A query for [`list`].
#[derive(Deserialize, Debug, Validate)]
#[garde(context(AppState as state))]
pub struct ListServersQuery {
    #[garde(range(min = 1, max = 50))]
    #[serde(default = "list_server_count_default")]
    pub count: i32,
}

fn list_server_count_default() -> i32 {
    50
}

#[derive(FromRow)]
struct ServerRow {
    pub id: i32,
    pub server_name: String,
}

#[derive(FromRow)]
struct MapConfigQuery {
    pub lumpname: String,
    #[sqlx(try_from = "u8")]
    pub status: BannedStatus,
    pub note: Option<String>,
    pub win_condition: Option<i32>,
    pub skill_lower: Option<i32>,
    pub skill_upper: Option<i32>,
}

/// Lists all matches.
#[instrument(skip(state))]
pub async fn list(
    State(state): State<AppState>,
    Valid(Form(query)): Valid<Form<ListServersQuery>>,
) -> Result<Json<Vec<Server>>, Error> {
    let mut conn = state.db.acquire().await.map_err(Error::new)?;

    let mut result = sqlx::query_as::<_, ServerRow>(
        r#"
        SELECT * FROM server
        "#,
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|s| Server {
        id: s.id,
        name: s.server_name,
        maps: HashMap::new(),
    })
    .collect::<Vec<_>>();

    for server in result.iter_mut() {
        preload_map_configs(server, &mut *conn).await?;
    }

    Ok(Json(result))
}

/// Gets the current server.
pub async fn show_self(
    auth: ServerAuthentication,
    State(state): State<AppState>,
) -> Result<Json<Server>, Error> {
    let mut conn = state.db.acquire().await.map_err(Error::new)?;

    let mut server = Server {
        id: auth.id,
        name: auth.server_name,
        maps: HashMap::new(),
    };

    preload_map_configs(&mut server, &mut *conn).await?;

    Ok(Json(server))
}

/// Updates the current server.
pub async fn update_self(
    auth: ServerAuthentication,
    State(state): State<AppState>,
    Valid(Payload(mut request)): Valid<Payload<UpdateServerRequest>>,
) -> Result<Json<Server>, Error> {
    let mut tx = state.db.begin().await.map_err(Error::new)?;

    let now = Utc::now();

    // Fetch current server information.
    let mut to_commit = false;
    let mut server = Server {
        id: auth.id,
        name: auth.server_name,
        maps: HashMap::new(),
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
    if let Some(bans) = request.maps {
        let mut new_bans = HashMap::with_capacity(server.maps.len());

        for (lumpname, config) in bans {
            let config: MapConfig = config.into();
            if let Some(old_ban) = server.maps.remove(&lumpname) {
                // Update old ban info
                if old_ban != config {
                    sqlx::query(
                        r#"
                        UPDATE map_config
                        SET
                            updated_at = $1,
                            status = $4,
                            note = $5,
                            win_condition = $6,
                            skill_lower = $7,
                            skill_upper = $8
                        WHERE lumpname = $2 AND parent_id = $3
                        "#,
                    )
                    .bind(now)
                    .bind(&lumpname)
                    .bind(server.id)
                    .bind(u8::from(config.status))
                    .bind(config.note.as_ref())
                    .bind(config.win_condition)
                    .bind(config.skill_range.lower)
                    .bind(config.skill_range.upper)
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
        std::mem::swap(&mut server.maps, &mut new_bans);
        for (lumpname, _) in new_bans {
            if server.maps.contains_key(&lumpname) {
                continue;
            }
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

    Ok(Json(server))
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
        let skill_range = duelchannel_model::server::SkillRange {
            lower: row.skill_lower,
            upper: row.skill_upper,
        };

        server.maps.insert(
            row.lumpname,
            MapConfig {
                status: row.status,
                note: row.note,
                win_condition: row.win_condition,
                skill_range,
            },
        );
    }

    Ok(())
}
