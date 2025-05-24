use std::{path::PathBuf, sync::Arc};

use argh::FromArgs;
use color_eyre::eyre::{self, bail, Context};
use engine::Engine;

use axum::{
    routing::{get, post},
    Router,
};
use tokio::{fs, net::TcpListener, signal};
use tracing::{info, warn};

mod cache;
mod config;
mod delete;
mod disk;
mod engine;
mod index;
mod new;
mod view;

#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

/// breeze file server.
#[derive(FromArgs, Debug)]
struct Args {
    /// the path to *.toml configuration file
    #[argh(option, short = 'c', arg_name = "file")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Install color-eyre
    color_eyre::install()?;

    // Read & parse args
    let args: Args = argh::from_env();

    // Read & parse config
    let cfg: config::Config = {
        let config_str = fs::read_to_string(args.config).await.wrap_err(
            "failed to read config file! make sure it exists and you have read permissions",
        )?;

        toml::from_str(&config_str).wrap_err(
            "invalid config! ensure proper fields and structure. reference config is in readme",
        )?
    };

    // Set up tracing
    tracing_subscriber::fmt()
        .with_max_level(cfg.logger.level)
        .init();

    // Check config
    {
        let save_path = cfg.engine.disk.save_path.clone();
        if !save_path.exists() || !save_path.is_dir() {
            bail!("the save path does not exist or is not a directory! this is invalid");
        }
    }
    if cfg.engine.upload_key.is_empty() {
        warn!("engine upload_key is empty! no key will be required for uploading new files");
    }

    // Create engine
    let engine = Engine::with_config(cfg.engine);

    // Build main router
    let app = Router::new()
        .route("/new", post(new::new))
        .route("/p/{saved_name}", get(view::view))
        .route("/del", get(delete::delete))
        .route("/", get(index::index))
        .route("/robots.txt", get(index::robots_txt))
        .with_state(Arc::new(engine));

    // Start web server
    info!("starting server.");
    let listener = TcpListener::bind(&cfg.http.listen_on)
        .await
        .wrap_err("failed to bind to given `http.listen_on` address! make sure it's valid, and the port isn't already bound")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .wrap_err("failed to start server")?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to add SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to add SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutting down!");
}
