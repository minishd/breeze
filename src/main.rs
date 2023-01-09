use std::sync::Arc;

extern crate axum;

#[macro_use]
extern crate log;

extern crate simplelog;

use simplelog::*;

use axum::{
    routing::{get, post},
    Router,
};
use bytes::Bytes;
use memory_cache::MemoryCache;
use tokio::sync::Mutex;

mod cache;
mod new;
mod state;
mod view;

#[tokio::main]
async fn main() {
    // initialise logger
    TermLogger::init(
        LevelFilter::Debug,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .unwrap();

    // create cache
    let cache: MemoryCache<String, Bytes> = MemoryCache::with_full_scan(cache::FULL_SCAN_FREQ);

    // create appstate
    let state = state::AppState {
        cache: Mutex::new(cache),
    };

    // build main router
    let app = Router::new()
        .route("/new", post(new::new))
        .route("/p/:name", get(view::view))
        .route("/", get(index))
        .with_state(Arc::new(state));

    // start web server
    axum::Server::bind(&"127.0.0.1:8000".parse().unwrap()) // don't forget to change this! it's local for now
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn index() -> &'static str {
    "hi world!"
}
