use std::{ffi::OsStr, path::PathBuf, sync::Arc};

use axum::{
    body::Body,
    extract::{Path, State},
    response::{IntoResponse, Response},
};

use http::{HeaderValue, StatusCode};
use tokio_util::io::ReaderStream;

use crate::engine::UploadData;

/// Responses for a failed view operation
pub enum ViewError {
    /// Will send status code 404 with a plaintext "not found" message.
    NotFound,

    /// Will send status code 500 with a plaintext "internal server error" message.
    InternalServerError,
}

impl IntoResponse for UploadData {
    fn into_response(self) -> Response {
        match self {
            UploadData::Disk(file, len) => {
                // create our content-length header
                let len_str = len.to_string();
                let content_length = HeaderValue::from_str(&len_str).unwrap();

                // create a streamed body response (we want to stream larger files)
                let stream = ReaderStream::new(file);
                let body = Body::from_stream(stream);

                // extract mutable headers from the response
                let mut res = body.into_response();
                let headers = res.headers_mut();

                // clear headers, browser can imply content type
                headers.clear();

                // insert Content-Length header
                // that way the browser shows how big a file is when it's being downloaded
                headers.insert("Content-Length", content_length);

                res
            }
            UploadData::Cache(data) => {
                // extract mutable headers from the response
                let mut res = data.into_response();
                let headers = res.headers_mut();

                // clear the headers, let the browser imply it
                headers.clear();

                res
            }
        }
    }
}

impl IntoResponse for ViewError {
    fn into_response(self) -> Response {
        match self {
            ViewError::NotFound => (
                StatusCode::NOT_FOUND,
                "not found!"
            ).into_response(),
            
            ViewError::InternalServerError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error!"
            ).into_response(),
        }
    }
}

/// The request handler for /p/* path.
/// All file views are handled here.
#[axum::debug_handler]
pub async fn view(
    State(engine): State<Arc<crate::engine::Engine>>,
    Path(original_path): Path<PathBuf>,
) -> Result<UploadData, ViewError> {
    let saved_name = if let Some(Some(n)) = original_path.file_name().map(OsStr::to_str) {
        n
    } else {
        return Err(ViewError::NotFound);
    };

    // get result from the engine!
    match engine.get(saved_name).await {
        Ok(Some(u)) => Ok(u),
        Ok(None) => Err(ViewError::NotFound),
        Err(_) => Err(ViewError::InternalServerError),
    }
}
