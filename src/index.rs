use std::sync::{atomic::Ordering, Arc};

use axum::extract::State;

// show index status page with amount of uploaded files
pub async fn index(State(engine): State<Arc<crate::engine::Engine>>) -> String {
    let count = engine.upl_count.load(Ordering::Relaxed);

    format!("minish's image host, currently hosting {} files", count)
}

// robots.txt that tells web crawlers not to list uploads
const ROBOTS_TXT: &str = concat!(
    "User-Agent: *\n",
    "Disallow: /p/*\n",
    "Allow: /\n"
);

pub async fn robots_txt() -> &'static str {
    ROBOTS_TXT
}