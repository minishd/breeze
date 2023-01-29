use std::{sync::Arc, path::PathBuf, time::Duration, process::exit};

extern crate axum;

#[macro_use]
extern crate log;

extern crate simplelog;

use engine::Engine;

use axum::{
    routing::{get, post},
    Router,
};
use simplelog::*;

mod engine;
mod index;
mod new;
mod view;

#[tokio::main]
async fn main() {
    // Initialise logger
    TermLogger::init(
        LevelFilter::Warn,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .unwrap();

    // Create engine
    let engine = Engine::new( // TODO: Read config from env vars
        "http://127.0.0.1:8000".to_string(),
        PathBuf::from("./uploads/"),
        80_000_000, // Main instance is going to use this
        Duration::from_secs(8), // CHANGE THIS!!!!!!!
        Duration::from_secs(1), // THIS TOO!!!!!!!!!!!!!!!
    );

    // Build main router
    let app = Router::new()
        .route("/new", post(new::new))
        .route("/p/:name", get(view::view))
        .route("/", get(index::index))
        .route("/exit", get(exit_abc))
        .with_state(Arc::new(engine));

    // Start web server
    axum::Server::bind(&"127.0.0.1:8000".parse().unwrap()) // don't forget to change this! it's local for now
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn exit_abc() {
    exit(123);
}