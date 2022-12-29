use std::sync::Arc;

extern crate axum;

use axum::{
    routing::{get, post},
    Router,
};
use bytes::Bytes;
use memory_cache::MemoryCache;
use tokio::sync::Mutex;

mod state;
mod new;
mod view;

#[tokio::main]
async fn main() {
    let mut cache: MemoryCache<String, Bytes> = MemoryCache::new();

    let state = state::AppState {
        cache: Mutex::new(cache)
    };

    let app = Router::new()
        .route("/new", post(new::new))
        .route("/p/:name", get(view::view))
        .route("/", get(index))
        .with_state(Arc::new(state));

    axum::Server::bind(&"127.0.0.1:8000".parse().unwrap()) // don't forget to change this! it's local for now
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn index() -> &'static str {
    "hi world!"
}
