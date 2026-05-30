use std::{env, fmt::Debug, io, net::SocketAddr, path::PathBuf, sync::Arc};

use eyre::OptionExt as _;
use http::{HeaderValue, Method, header};

use opendal::Operator;
// :(
use time::Duration;

use clap::{CommandFactory as _, Parser};

use axum::{
    Extension, Router,
    extract::{MatchedPath, Request},
    middleware::{Next, from_fn},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};

use axum_server::Handle;

use duelchannel::{
    app::{AppState, Model, ModelOrUnrated, Unrated},
    auth::oauth2::OauthState,
    cli::{self, AnalyticsCommand, Args, Command, MmrCommand, MmrDump},
    config::{Config, RatingModelConfig, StorageService, read_config},
    error::Error,
    routes,
    schema::user::mmr::{self, dump_rating, glicko2::Glicko2, init_rating, openskill::OpenSkill},
};

use sqlx::{Sqlite, pool::PoolOptions};

use tokio::{main, select, signal};

use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use tower_sessions::{CachingSessionStore, Expiry, SessionManagerLayer, cookie::SameSite};
use tower_sessions_moka_store::MokaStore;
use tower_sessions_sqlx_store::SqliteStore;

use cookie::Key;

use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    fmt,
    layer::SubscriberExt,
};

const OPENAPI_FILE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/openapi/openapi.yaml"));

const CARGO_PKG_NAME: &str = env!("CARGO_PKG_NAME");

const GIT_COMMIT_HASH: &str = env!("GIT_COMMIT_HASH");
const GIT_COMMIT_MESSAGE: Option<&str> = option_env!("GIT_COMMIT_MESSAGE");

#[main]
async fn main() -> eyre::Result<()> {
    dotenv::dotenv().ok();

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();

    let registry = tracing_subscriber::registry();

    let fmt_layer = fmt::layer().with_writer(io::stderr);
    let filter_layer = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    #[cfg(feature = "tracy")]
    let registry = registry.with(tracing_tracy::TracyLayer::default());

    let registry = registry.with(filter_layer).with(fmt_layer);
    tracing::subscriber::set_global_default(registry)?;

    let cli = Args::parse();

    let config_path = match &cli.config {
        Some(path) => path.to_owned(),
        None => PathBuf::from("config.toml"),
    };

    if let Some(msg) = GIT_COMMIT_MESSAGE {
        tracing::info!("{} {} {}", CARGO_PKG_NAME, &GIT_COMMIT_HASH[0..8], msg);
    } else {
        tracing::info!("{} {}", CARGO_PKG_NAME, GIT_COMMIT_HASH);
    }

    // Read config file
    let config = read_config(config_path)?;

    // Setup MMR w/ config
    match &config.mmr {
        RatingModelConfig::Unrated => with_rating_model(cli, config, Unrated).await,
        RatingModelConfig::Glicko2(mmr_config) => {
            let model = Glicko2::new(mmr_config.clone());
            with_rating_model(cli, config, model).await
        }
        RatingModelConfig::OpenSkill(mmr_config) => {
            let model = OpenSkill::new(mmr_config.clone()).await?;
            tracing::info!("using openskill model");
            with_rating_model(cli, config, model).await
        }
    }
}

async fn with_rating_model<T>(cli: Args, mut config: Config, model: T) -> eyre::Result<()>
where
    T: Debug + Clone + Send + Sync + ModelOrUnrated + 'static,
    <T as ModelOrUnrated>::Model: Debug,
    <T::Model as mmr::Model>::Data: Debug + Clone,
{
    let database_url = config
        .server
        .database_url
        .take()
        .ok_or_eyre("No `DATABASE_URL` set!")?;

    // Connect to sqlite database
    let db = PoolOptions::<Sqlite>::new().connect(&database_url).await?;

    // Load object storage
    let object_storage = match &config.cdn.service {
        StorageService::Filesystem(fs) => {
            tracing::info!("using filesystem object storage @ {}", fs.root.display());

            std::fs::create_dir_all(&fs.root)?;
            let fs = opendal::services::Fs::default()
                // WTF? No OsString???
                .root(
                    fs.root
                        .to_str()
                        .ok_or_eyre("cannot parse filesystem root path")?,
                );
            Operator::new(fs)?.finish()
        }
        StorageService::S3(s3) => {
            tracing::info!("using amazon s3 storage");

            let s3 = opendal::services::S3::default()
                .bucket(&s3.bucket)
                .region(&s3.region)
                .access_key_id(&s3.access_key_id)
                .secret_access_key(&s3.access_key_secret);
            Operator::new(s3)?.finish()
        }
    };

    let app_model = Model::new(model.clone());

    // Run any pending commands
    if let Some(command) = cli.command.as_ref() {
        match command {
            Command::RegisterServer(server) => {
                // establish connection
                let mut tx = db.begin().await?;

                tracing::info!("registering server {}", server.server_name);

                cli::register_server(server, &mut *tx).await?;

                tx.commit().await?;
            }
            Command::Analytics(
                analytics @ cli::Analytics {
                    command: Some(AnalyticsCommand::Battle(_)),
                    ..
                },
            ) => {
                cli::run_battle_analytics(&analytics, &app_model, &db).await?;
            }
            Command::Analytics(cli::Analytics { command: None, .. }) => {
                Args::command().print_help().unwrap();
            }
            Command::GenerateKey(_) => {
                tracing::info!("generated! set ENCRYPTION_KEY or server.encryption_key on boot");

                let key = Key::generate();
                let key = base16::encode_lower(key.master());
                println!("{}", key);
            }
            Command::Mmr(cli::Mmr {
                command: Some(MmrCommand::Reset(_)),
            }) => {
                // establish connection
                let mut tx = db.begin().await?;

                tracing::info!("resetting all mmr...");

                // clear records
                sqlx::query("DELETE FROM rating").execute(&mut *tx).await?;
                sqlx::query("DELETE FROM rating_period")
                    .execute(&mut *tx)
                    .await?;

                // update all players ratings
                let player_ids = sqlx::query_as::<_, (i32,)>("SELECT id FROM user")
                    .fetch_all(&mut *tx)
                    .await?;

                if let Some(model) = model.model() {
                    for (id,) in player_ids {
                        // init player rating
                        init_rating(id, model, &mut *tx).await?;
                    }
                }

                tx.commit().await?;
            }
            Command::Mmr(cli::Mmr {
                command: Some(MmrCommand::Dump(MmrDump { exclude })),
            }) => {
                // establish connection
                let mut tx = db.begin().await?;

                // delete excluded participants
                for id in exclude {
                    sqlx::query(
                        r#"
                        DELETE FROM participant
                        WHERE player_id IN (
                            SELECT id
                            FROM player
                            WHERE short_id = $1
                        )
                        "#,
                    )
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                }

                if let Some(model) = model.model() {
                    dump_rating(std::io::stdout(), model, &mut *tx).await?;
                }

                // rollback transaction
                tx.rollback().await?;
            }
            Command::Mmr(cli::Mmr { command: None }) => {
                Args::command().print_help().unwrap();
            }
        }

        return Ok(());
    }

    let encryption_key = if let Some(key_str) = config.server.encryption_key.take() {
        if key_str.len() > 128 {
            tracing::error!("encryption key too long! generate with `ring-channel generate-key`");
            std::process::exit(1);
        }

        let mut key = [0u8; 64];
        base16::decode_slice(&key_str[..], &mut key)?;

        match Key::try_from(&key[..]) {
            Ok(key) => key,
            Err(err) => {
                tracing::error!("bad encryption key: {}", err);
                std::process::exit(1);
            }
        }
    } else {
        tracing::warn!(
            "generating runtime encryption key! sessions will stop working after restart"
        );
        tracing::warn!("generate a permanent key with `ring-channel generate-key`");
        Key::generate()
    };

    tracing::info!("establishing connection to database");

    // Create app state
    let state = AppState {
        config: Arc::new(config.clone()),
        object_storage,
        db: db.clone(),
    };

    // Build routes
    let mut api_routes = Router::<AppState>::new()
        // .nest(
        //     "/players",
        //     Router::<AppState>::new()
        //         .route("/", post(routes::player::register::<T>))
        //         .route("/{player_id}", get(routes::player::show::<T>)),
        // )
        // .nest(
        //     "/chat",
        //     Router::<AppState>::new().route("/messages", post(routes::chat::create::<T>)),
        // )
        .nest(
            "/matches",
            Router::<AppState>::new()
                .route("/", get(routes::battle::list))
                .route("/", post(routes::battle::create::<T>))
                .route("/analytics", get(routes::battle::analytics::show))
                .nest(
                    "/{battle_id}",
                    Router::<AppState>::new()
                        .route("/", get(routes::battle::show))
                        .route("/", patch(routes::battle::update::<T>))
                        .route("/players/{short_id}", patch(routes::battle::player::update))
                        .route("/replay", post(routes::battle::replay::upload)),
                ),
        )
        .nest(
            "/servers",
            Router::<AppState>::new()
                .route("/", get(routes::server::list))
                .route("/~me", get(routes::server::show_self))
                .route("/~me", patch(routes::server::update_self)),
        )
        .nest(
            "/players",
            Router::<AppState>::new()
                .route("/", post(routes::player::create::<T>))
                .route("/", get(routes::player::list))
                .route("/~me", get(routes::player::show_self))
                .route("/{short_id}", get(routes::player::show)),
        )
        .with_state(state.clone());

    if let Some(discord_config) = config.discord.as_ref() {
        let oauth_state = OauthState::new(&config.server.base_url, db.clone(), &discord_config)?
            .with_redirect_to(config.server.redirect_url.clone());

        let oauth_router = Router::<OauthState>::new()
            .route("/auth/~redirect", get(routes::auth::redirect))
            .route("/auth/~login", get(routes::auth::login))
            .with_state(oauth_state);

        api_routes = api_routes.merge(oauth_router);

        tracing::info!(
            client_id = { discord_config.client_id },
            "discord integration setup"
        );
    }

    // Create session management
    let db_session_store = SqliteStore::new(db.clone())
        .with_table_name("_session")
        .map_err(eyre::Report::msg)?;
    db_session_store.migrate().await?;

    let caching_session_store = MokaStore::new(Some(2_000));

    let session_store = CachingSessionStore::new(caching_session_store, db_session_store);
    let session_layer = SessionManagerLayer::new(session_store)
        .with_name("id")
        .with_expiry(Expiry::OnInactivity(Duration::days(30)))
        .with_http_only(true)
        .with_same_site(SameSite::Lax)
        .with_private(encryption_key)
        .with_secure(config.server.secure_sessions);

    // Finalize router
    let router = Router::new()
        .merge(api_routes.layer(from_fn(security_headers)))
        // serve openapi spec
        .merge(
            Router::new()
                .route("/openapi.yaml", get(serve_openapi))
                .layer(
                    CorsLayer::new()
                        .allow_methods([Method::GET])
                        .allow_origin(Any),
                ),
        )
        .layer(Extension(app_model))
        .layer(session_layer)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &Request| {
                    let method = req.method();
                    let uri = req.uri();

                    // axum automatically adds this extension.
                    let matched_path = req
                        .extensions()
                        .get::<MatchedPath>()
                        .map(|matched_path| matched_path.as_str());

                    tracing::debug_span!("request", %method, %uri, matched_path)
                })
                // By default `TraceLayer` will log 5xx responses but we're doing our specific
                // logging of errors so disable that
                .on_failure(()),
        )
        .layer(from_fn(log_app_errors));

    let handle = Handle::new();

    // run shutdown task to detect shutdowns
    tokio::spawn(shutdown_signal(handle.clone()));

    let addr: SocketAddr = ([0, 0, 0, 0], config.http.port).into();

    tracing::info!("listening on {} (http)", addr);

    axum_server::bind(addr)
        .handle(handle)
        .serve(router.into_make_service())
        .await?;

    tracing::info!("shutting down");

    db.close().await;

    Ok(())
}

async fn serve_openapi() -> impl IntoResponse {
    (
        [(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"openapi.yaml\"",
        )],
        OPENAPI_FILE,
    )
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut res = next.run(request).await;

    res.headers_mut().extend([
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        (
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'none'"),
        ),
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
        (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
        // (
        //     header::STRICT_TRANSPORT_SECURITY,
        //     HeaderValue::try_from(format!("max-age={}", hsts_time)).expect("valid hsts time"),
        // ),
    ]);

    res
}

// Stolen from: https://github.com/tokio-rs/axum/blob/main/examples/error-handling/src/main.rs
async fn log_app_errors(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    // If the response contains an Error Extension, log it.
    if let Some(err) = response.extensions().get::<Arc<Error>>() {
        tracing::error!(?err, "an unexpected error occurred inside a handler");
    }
    response
}

// Stolen from: https://github.com/maxcountryman/tower-sessions-stores/tree/main/sqlx-store
// Lol
async fn shutdown_signal(handle: Handle) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    select! {
        _ = ctrl_c => { handle.shutdown() }
        _ = terminate => { handle.shutdown() }
    }
}
