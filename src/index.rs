use std::sync::{atomic::Ordering, Arc};

use axum::extract::State;

/// Show index status page with amount of uploaded files
pub async fn index(State(engine): State<Arc<crate::engine::Engine>>) -> String {
    let count = engine.upl_count.load(Ordering::Relaxed);

    let motd = engine.cfg.motd.clone();

    motd.replace("%version%", env!("CARGO_PKG_VERSION"))
        .replace("%uplcount%", &count.to_string())
}

pub async fn robots_txt() -> &'static str {
    /// robots.txt that tells web crawlers not to list uploads
    const ROBOTS_TXT: &str = concat!(
        "User-Agent: *\n",
        "Disallow: /p/*\n",
        "Allow: /\n"
    );

    ROBOTS_TXT
}
