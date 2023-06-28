use std::{env, path::PathBuf, sync::Arc, time::Duration};

extern crate axum;

#[macro_use]
extern crate log;

use engine::Engine;

use axum::{
    routing::{get, post},
    Router,
};
use tokio::signal;

mod engine;
mod index;
mod new;
mod view;

#[tokio::main]
async fn main() {
    // initialise logger
    pretty_env_logger::init();

    // read env vars
    let base_url = env::var("BRZ_BASE_URL").expect("missing BRZ_BASE_URL! base url for upload urls (ex: http://127.0.0.1:8000 for http://127.0.0.1:8000/p/abcdef.png, http://picture.wtf for http://picture.wtf/p/abcdef.png)");
    let save_path = env::var("BRZ_SAVE_PATH").expect("missing BRZ_SAVE_PATH! this should be a path where uploads are saved to disk (ex: /srv/uploads, C:\\brzuploads)");
    let upload_key = env::var("BRZ_UPLOAD_KEY").unwrap_or_default();
    let cache_max_length = env::var("BRZ_CACHE_UPL_MAX_LENGTH").expect("missing BRZ_CACHE_UPL_MAX_LENGTH! this is the max length an upload can be in bytes before it won't be cached (ex: 80000000 for 80MB)");
    let cache_upl_lifetime = env::var("BRZ_CACHE_UPL_LIFETIME").expect("missing BRZ_CACHE_UPL_LIFETIME! this indicates how long an upload will stay in cache (ex: 1800 for 30 minutes, 60 for 1 minute)");
    let cache_scan_freq = env::var("BRZ_CACHE_SCAN_FREQ").expect("missing BRZ_CACHE_SCAN_FREQ! this is the frequency of full cache scans, which scan for and remove expired uploads (ex: 60 for 1 minute)");
    let cache_mem_capacity = env::var("BRZ_CACHE_MEM_CAPACITY").expect("missing BRZ_CACHE_MEM_CAPACITY! this is the amount of memory the cache will hold before dropping entries");

    // parse env vars
    let save_path = PathBuf::from(save_path);
    let cache_max_length = usize::from_str_radix(&cache_max_length, 10).expect("failed parsing BRZ_CACHE_UPL_MAX_LENGTH! it should be a positive number without any separators");
    let cache_upl_lifetime = Duration::from_secs(u64::from_str_radix(&cache_upl_lifetime, 10).expect("failed parsing BRZ_CACHE_UPL_LIFETIME! it should be a positive number without any separators"));
    let cache_scan_freq = Duration::from_secs(u64::from_str_radix(&cache_scan_freq, 10).expect("failed parsing BRZ_CACHE_SCAN_FREQ! it should be a positive number without any separators"));
    let cache_mem_capacity = usize::from_str_radix(&cache_mem_capacity, 10).expect("failed parsing BRZ_CACHE_MEM_CAPACITY! it should be a positive number without any separators");

    if !save_path.exists() || !save_path.is_dir() {
        panic!("the save path does not exist or is not a directory! this is invalid");
    }

    if upload_key.is_empty() {
        // i would prefer this to be a warning but the default log level hides those
        error!("upload key (BRZ_UPLOAD_KEY) is empty! no key will be required for uploading new files");
    }

    // create engine
    let engine = Engine::new(
        base_url,
        save_path,
        upload_key,
        cache_max_length,
        cache_upl_lifetime,
        cache_scan_freq,
        cache_mem_capacity,
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