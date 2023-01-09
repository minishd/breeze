use std::{
    ffi::OsStr,
    path::{Component, PathBuf},
    sync::Arc,
};

use axum::{
    body::StreamBody,
    extract::{Path, State},
    http::HeaderValue,
    response::{IntoResponse, Response},
};

use bytes::{Bytes, BytesMut};
use hyper::StatusCode;
use mime_guess::mime;
use tokio::{fs::File, io::AsyncReadExt};
use tokio_util::io::ReaderStream;

use crate::cache;

/* pub enum ViewResponse {
    FromDisk(StreamBody<ReaderStream<File>>),
    FromCache(Bytes)
}

impl IntoResponse for ViewResponse {
    fn into_response(self) -> Response {
        match self {
            ViewResponse::FromDisk(stream) => stream.into_response(),
            ViewResponse::FromCache(data) => data.into_response()
        }
    }
} */

#[axum::debug_handler]
pub async fn view(
    State(state): State<Arc<crate::state::AppState>>,
    Path(original_path): Path<PathBuf>,
) -> Response {
    // (hopefully) prevent path traversal, just check for any non-file components
    if original_path
        .components()
        .into_iter()
        .any(|x| !matches!(x, Component::Normal(_)))
    {
        error!(target: "view", "a request attempted path traversal");
        return StatusCode::NOT_FOUND.into_response();
    }

    let name = original_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    let mut cache = state.cache.lock().await;

    let cache_item = cache.get(&name.clone());

    if cache_item.is_none() {
        let mut path = PathBuf::new();
        path.push("uploads/");
        path.push(name.clone());

        if !path.exists() || !path.is_file() {
            return StatusCode::NOT_FOUND.into_response();
        }

        let mut file = File::open(path).await.unwrap();
        let file_len = file.metadata().await.unwrap().len() as usize;

        if file_len < cache::MAX_LENGTH {
            info!(target: "view", "recaching upload from disk");

            let mut data = BytesMut::zeroed(file_len);
            file.read_buf(&mut data.as_mut()).await.unwrap();
            let data = data.freeze();

            cache.insert(name.clone(), data.clone(), Some(cache::DURATION));

            return cache::get_response(&mut cache, original_path);
        } else {
            let reader = ReaderStream::new(file);
            let stream = StreamBody::new(reader);

            info!(target: "view", "reading upload from disk");

            return stream.into_response();
        }
    }

    info!(target: "view", "reading upload from cache");

    return cache::get_response(&mut cache, original_path);
}

/* #[axum::debug_handler]
pub async fn view(
    State(state): State<Arc<crate::state::AppState>>,
    Path(original_path): Path<PathBuf>,
) -> Response {
    // (hopefully) prevent path traversal, just check for any non-file components
    if original_path
        .components()
        .into_iter()
        .any(|x| !matches!(x, Component::Normal(_)))
    {
        error!(target: "view", "a request attempted path traversal");
        return StatusCode::NOT_FOUND.into_response();
    }

    let name = original_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    let cache = state.cache.lock().await;

    let cache_item = cache.get(&name);

    if cache_item.is_none() {
        let mut path = PathBuf::new();
        path.push("uploads/");
        path.push(name);

        if !path.exists() || !path.is_file() {
            return StatusCode::NOT_FOUND.into_response();
        }

        let file = File::open(path).await.unwrap();

        if file.metadata().await.unwrap().len() < (cache::MAX_LENGTH as u64) {
            info!("file can be cached");
        }

        let reader = ReaderStream::new(file);
        let stream = StreamBody::new(reader);

        info!(target: "view", "reading upload from disk");

        return stream.into_response();
    }

    info!(target: "view", "reading upload from cache");

    let data = cache_item.unwrap().clone();

    let content_type = mime_guess::from_path(original_path)
        .first()
        .unwrap_or(mime::APPLICATION_OCTET_STREAM)
        .to_string();

    let mut res = data.into_response();
    let headers = res.headers_mut();

    headers.clear();
    headers.insert(
        "content-type",
        HeaderValue::from_str(content_type.as_str()).unwrap(),
    );

    return res;
} */
