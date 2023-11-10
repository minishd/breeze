use std::{path::PathBuf, sync::Arc};

extern crate axum;

use clap::Parser;
use engine::Engine;

use axum::{
    routing::{get, post},
    Router,
};
use tokio::{fs, signal};
use tracing::{info, warn};
use tracing_subscriber::filter::LevelFilter;

mod config;
mod engine;
mod index;
mod new;
mod view;

#[derive(Parser, Debug)]
struct Args {
    /// The path to configuration file
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    // read & parse args
    let args = Args::parse();

    // read & parse config
    let config_str = fs::read_to_string(args.config.unwrap_or("./breeze.toml".into()))
        .await
        .expect("failed to read config file! make sure it exists and you have read permissions");

    let c: config::Config = toml::from_str(&config_str).expect("invalid config! check that you have included all required options and structured it properly (no config options expecting a number getting a string, etc.)");

    tracing_subscriber::fmt()
        .with_max_level(c.logger.level.unwrap_or(LevelFilter::WARN))
        .init();

    if !c.engine.save_path.exists() || !c.engine.save_path.is_dir() {
        panic!("the save path does not exist or is not a directory! this is invalid");
    }

    if c.engine.upload_key.is_none() {
        warn!("engine upload_key is empty! no key will be required for uploading new files");
    }

    // create engine
    let engine = Engine::new(
        c.engine.base_url,
        c.engine.save_path,
        c.engine.upload_key.unwrap_or_default(),
        c.cache.max_length,
        c.cache.upload_lifetime,
        c.cache.scan_freq,
        c.cache.mem_capacity,
    );

    // build main router
    let app = Router::new()
        .route("/new", post(new::new))
        .route("/p/:name", get(view::view))
        .route("/", get(index::index))
        .route("/robots.txt", get(index::robots_txt))
        .with_state(Arc::new(engine));

    // start web server
    axum::Server::bind(&"0.0.0.0:8000".parse().unwrap())
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
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
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutting down!");
}
