use std::{
    path::{Component, PathBuf},
    sync::Arc,
};

use axum::{
    body::StreamBody,
    extract::{Path, State},
    http::HeaderValue,
    response::{IntoResponse, Response},
};

use bytes::Bytes;
use hyper::StatusCode;
use mime_guess::mime;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

pub enum ViewResponse {
    FromDisk(File),
    FromCache(PathBuf, Bytes),
}

impl IntoResponse for ViewResponse {
    fn into_response(self) -> Response {
        match self {
            ViewResponse::FromDisk(file) => {
                let reader = ReaderStream::new(file);
                let stream = StreamBody::new(reader);
        
                stream.into_response()
            }
            ViewResponse::FromCache(original_path, data) => {
                // guess the content-type using the original path
                // (axum handles this w/ streamed file responses but caches are octet-stream by default)
                let content_type = mime_guess::from_path(original_path)
                    .first()
                    .unwrap_or(mime::APPLICATION_OCTET_STREAM)
                    .to_string();

                // extract mutable headers from the response
                let mut res = data.into_response();
                let headers = res.headers_mut();

                // clear the headers and add our content-type
                headers.clear();
                headers.insert(
                    "content-type",
                    HeaderValue::from_str(content_type.as_str()).unwrap(),
                );

                res
            }
        }
    }
}

#[axum::debug_handler]
pub async fn view(
    State(engine): State<Arc<crate::engine::Engine>>,
    Path(original_path): Path<PathBuf>,
) -> Result<ViewResponse, StatusCode> {
    // (hopefully) prevent path traversal, just check for any non-file components
    if original_path
        .components()
        .into_iter()
        .any(|x| !matches!(x, Component::Normal(_)))
    {
        error!(target: "view", "a request attempted path traversal");
        return Err(StatusCode::NOT_FOUND);
    }
    
    engine.get_upload(&original_path).await
}
