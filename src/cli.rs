//! Ring Channel server command-line interface.

use std::{
    cmp::min,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use chrono::Utc;

use clap::{Parser, Subcommand};

use eyre::Error;

use sqlx::{FromRow, SqliteConnection, SqlitePool};
use tokio::task::JoinSet;

use crate::{
    app::{Model, ModelOrUnrated},
    auth::api_key::{generate_api_key, hash_api_key},
    schema::{battle::analytics::get_analytics, user::mmr},
};

/// The command line arguments.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Configuration file path.
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// The command to run.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Operational commands.
#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(name = "register")]
    RegisterServer(RegisterServer),
    #[command(name = "analytics")]
    Analytics(Analytics),
    #[command(name = "generate-key")]
    GenerateKey(GenerateKey),
    #[command(name = "mmr")]
    Mmr(Mmr),
}

/// Registers a server with the ring channel API.
#[derive(clap::Args, Debug)]
pub struct RegisterServer {
    /// The name of the server to register.
    pub server_name: String,
}

/// Generates an encryption key to encrypt cookies.
///
/// Does nothing but spit out a key that can be read back by the server later.
#[derive(clap::Args, Debug)]
pub struct GenerateKey;

fn default_report_every() -> usize {
    100
}

/// Runs analytics.
#[derive(clap::Args, Debug)]
pub struct Analytics {
    /// The command to run.
    #[command(subcommand)]
    pub command: Option<AnalyticsCommand>,
    /// Report progress every `x` entries.
    #[arg(short, long, default_value_t = default_report_every())]
    pub report_every: usize,
}

#[derive(Subcommand, Debug)]
pub enum AnalyticsCommand {
    #[command(name = "battle")]
    Battle(BattleAnalytics),
}

#[derive(clap::Args, Debug)]
pub struct BattleAnalytics;

/// Does some Mmr things.
#[derive(clap::Args, Debug)]
pub struct Mmr {
    /// The command to run.
    #[command(subcommand)]
    pub command: Option<MmrCommand>,
}

#[derive(Subcommand, Debug)]
pub enum MmrCommand {
    #[command(name = "reset")]
    Reset(MmrReset),
    #[command(name = "dump")]
    Dump(MmrDump),
}

/// Sample's a given player's MMR.
#[derive(clap::Args, Debug)]
pub struct MmrDump {
    /// Exclude certain short IDs.
    #[arg(short, long)]
    pub exclude: Vec<String>,
}

/// Resets the MMR of the server.
#[derive(clap::Args, Debug)]
pub struct MmrReset;

/// Recalculates battle analytics.
pub async fn run_battle_analytics<T>(
    command: &Analytics,
    model: &Model<T>,
    db: &SqlitePool,
) -> Result<(), Error>
where
    T: ModelOrUnrated + Clone + Send + Sync,
    <T::Model as mmr::Model>::Data: Clone,
{
    #[derive(Clone, FromRow)]
    struct BattleRow {
        pub id: i32,
        pub uuid: String,
    }

    let battles = sqlx::query_as::<_, BattleRow>(
        r#"
        SELECT id, uuid
        FROM battle
        ORDER BY inserted_at DESC
        "#,
    )
    .fetch_all(db)
    .await?;

    let battles_len = battles.len();
    tracing::info!("discovered {} battles", battles_len);

    let processed_count = Arc::new(AtomicUsize::new(0));

    let Analytics { report_every, .. } = *command;

    // Split work up
    let num_tasks = min(num_cpus::get(), db.num_idle());
    let mut tasks = JoinSet::new();

    for i in 0..num_tasks {
        let battles = battles
            .iter()
            .skip(i)
            .step_by(num_tasks)
            .cloned()
            .collect::<Vec<_>>();

        let db_clone = db.clone();
        let model_clone = Model::<T>::clone(model);
        let processed_count_clone = Arc::clone(&processed_count);

        tasks.spawn(async move {
            let mut conn = db_clone.acquire().await?;

            for row in battles {
                match get_analytics(row.id, &model_clone, &mut *conn).await {
                    Ok(_data) => (),
                    Err(err) => tracing::error!("failed to process \"{}\": {}", row.uuid, err),
                }

                let res = processed_count_clone.fetch_add(1, Ordering::AcqRel);
                if res != 0 && res % report_every == 0 {
                    tracing::info!("processed {}/{} battles", res, battles_len);
                }
            }

            conn.close().await?;

            Ok::<_, Error>(())
        });
    }

    for result in tasks.join_all().await {
        match result {
            Ok(_) => (),
            Err(err) => tracing::error!("failed to process battles: {}", err),
        }
    }

    Ok(())
}

/// Registers a server.
pub async fn register_server(
    command: &RegisterServer,
    conn: &mut SqliteConnection,
) -> Result<(), Error> {
    // generate api token
    let api_key = generate_api_key();
    let hash = hash_api_key(&api_key);

    let now = Utc::now();

    // insert new server
    sqlx::query(
        r#"
        INSERT INTO server (server_name, key_hash, inserted_at, updated_at)
        VALUES ($1, $2, $3, $3)
        "#,
    )
    .bind(&command.server_name)
    .bind(hash)
    .bind(now)
    .execute(&mut *conn)
    .await?;

    // export key
    println!("{}", api_key);

    Ok(())
}
