use std::{collections::HashMap, ffi::OsStr, path::PathBuf, sync::Arc};

use axum::{
    extract::{BodyStream, Query, State},
    http::HeaderValue,
};
use hyper::{HeaderMap, StatusCode, header};

#[axum::debug_handler]
pub async fn new(
    State(engine): State<Arc<crate::engine::Engine>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    stream: BodyStream,
) -> Result<String, StatusCode> {
    if !params.contains_key("name") {
        return Err(StatusCode::BAD_REQUEST);
    }

    let original_name = params.get("name").unwrap();
    let original_path = PathBuf::from(original_name);

    let path = engine.gen_path(&original_path).await;
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    let url = format!("{}/p/{}", engine.base_url, name);

    let content_length = headers
        .get(header::CONTENT_LENGTH)
        .unwrap_or(&HeaderValue::from_static(""))
        .to_str()
        .and_then(|s| Ok(usize::from_str_radix(s, 10)))
        .unwrap()
        .unwrap_or(usize::MAX);

    engine
        .process_upload(path, name, content_length, stream)
        .await;

    Ok(url)
}
