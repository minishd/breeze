use std::sync::{atomic::Ordering, Arc};

use axum::extract::State;

// show index status page
pub async fn index(State(engine): State<Arc<crate::engine::Engine>>) -> String {
    let count = engine.upl_count.load(Ordering::Relaxed);

    format!("minish's image host, currently hosting {} files", count)
}
