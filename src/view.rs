use std::{
    ffi::OsStr,
    path::{Component, PathBuf},
    sync::Arc,
};

use axum::{
    body::StreamBody,
    extract::{Path, State},
    response::{IntoResponse, Response}, debug_handler, http::HeaderValue,
};
use bytes::{buf::Reader, Bytes};
use hyper::StatusCode;
use mime_guess::{mime, Mime};
use tokio::fs::File;
use tokio_util::io::ReaderStream;

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
        println!("lol NOPE");
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

        let reader = ReaderStream::new(file);
        let stream = StreamBody::new(reader);

        println!("from disk");

        return stream.into_response();
    }

    println!("from cache! :D");

    let data = cache_item.unwrap().clone();

    drop(cache);

    let contentType = mime_guess::from_path(name).first().unwrap_or(mime::APPLICATION_OCTET_STREAM).to_string();

    let mut res = data.into_response();

    let mut headers = res.headers_mut();

    headers.clear();
    headers.insert("content-type", HeaderValue::from(contentType));

    return res;
}

/* pub async fn view(
    State(mem_cache): State<Arc<crate::cache::MemCache>>,
    Path(original_path): Path<PathBuf>,
) -> Response {
    for component in original_path.components() {
        println!("{:?}", component);
    }

    // (hopefully) prevent path traversal, just check for any non-file components
    if original_path
        .components()
        .into_iter()
        .any(|x| !matches!(x, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response()
    }

    // this causes an obscure bug where filenames like hiworld%2fnamehere.png will still load namehere.png
    // i could limit the path components to 1 and sort of fix this
    let name = original_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    let cache = mem_cache.cache.lock().unwrap();

    let cache_item = cache.get(&name);

    if cache_item.is_some() {
        println!("they requested something in the cache!");

        let data = cache_item.unwrap().clone();

        return data.into_response()
    }

    let mut path = PathBuf::new();
    path.push("uploads/");
    path.push(name);

    if !path.exists() || !path.is_file() {
        return StatusCode::NOT_FOUND.into_response()
    }

    let file = File::open(path).await.unwrap();

    let reader = ReaderStream::new(file);
    let stream = StreamBody::new(reader);

    stream.into_response()
}
 */
