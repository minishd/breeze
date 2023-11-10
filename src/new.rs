use std::{collections::HashMap, ffi::OsStr, path::PathBuf, sync::Arc};

use axum::{
    extract::{BodyStream, Query, State},
    http::HeaderValue,
};
use hyper::{header, HeaderMap, StatusCode};

/// The request handler for the /new path.
/// This handles all new uploads.
#[axum::debug_handler]
pub async fn new(
    State(engine): State<Arc<crate::engine::Engine>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    stream: BodyStream,
) -> Result<String, StatusCode> {
    let key = params.get("key");

    const EMPTY_STRING: &String = &String::new();

    // check upload key, if i need to
    if !engine.upload_key.is_empty() && key.unwrap_or(EMPTY_STRING) != &engine.upload_key {
        return Err(StatusCode::FORBIDDEN);
    }

    let original_name = params.get("name");

    // the original file name wasn't given, so i can't work out what the extension should be
    if original_name.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let original_path = PathBuf::from(original_name.unwrap());

    let path = engine.gen_path(&original_path).await;
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    let url = format!("{}/p/{}", engine.base_url, name);

    // read and parse content-length, and if it fails just assume it's really high so it doesn't cache
    let content_length = headers
        .get(header::CONTENT_LENGTH)
        .unwrap_or(&HeaderValue::from_static(""))
        .to_str()
        .map(|s| s.parse::<usize>())
        .unwrap()
        .unwrap_or(usize::MAX);

    // pass it off to the engine to be processed!
    engine
        .process_upload(path, name, content_length, stream)
        .await;

    Ok(url)
}
