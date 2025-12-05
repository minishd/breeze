use std::{ffi::OsStr, path::PathBuf, sync::Arc};

use axum::{
    body::Body,
    extract::{Path, State},
    response::{IntoResponse, Response},
};

use axum_extra::TypedHeader;
use headers::Range;
use http::{HeaderValue, StatusCode};
use tokio_util::io::ReaderStream;
use tracing::error;

use crate::engine::{Engine, GetOutcome, UploadData, UploadResponse};

/// Responses for a failed view operation
pub enum ViewError {
    /// Will send status code 404 with a plaintext "not found" message.
    NotFound,

    /// Will send status code 500 with a plaintext "internal server error" message.
    InternalServerError,

    /// Sends status code 206 with a plaintext "range not satisfiable" message.
    RangeNotSatisfiable,
}

impl IntoResponse for ViewError {
    fn into_response(self) -> Response {
        match self {
            ViewError::NotFound => (StatusCode::NOT_FOUND, "Not found!").into_response(),

            ViewError::InternalServerError => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error!").into_response()
            }

            ViewError::RangeNotSatisfiable => {
                (StatusCode::RANGE_NOT_SATISFIABLE, "Range not satisfiable!").into_response()
            }
        }
    }
}

impl IntoResponse for UploadResponse {
    fn into_response(self) -> Response {
        let (start, end) = self.range;
        let range_len = (end - start) + 1;

        let mut res = match self.data {
            UploadData::Cache(data) => data.into_response(),
            UploadData::Disk(file) => {
                let reader_stream = ReaderStream::new(file);
                let body = Body::from_stream(reader_stream);
                let mut res = body.into_response();
                let headers = res.headers_mut();

                // add Content-Length header so the browser shows how big a file is when it's being downloaded
                let content_length = HeaderValue::from_str(&range_len.to_string())
                    .expect("construct content-length header failed");
                headers.insert("Content-Length", content_length);

                res
            }
        };

        let headers = res.headers_mut();

        // remove content-type, browser can imply content type
        headers.remove("Content-Type");
        headers.insert("Accept-Ranges", HeaderValue::from_static("bytes"));
        // ^-- indicate that byte ranges are supported. maybe unneeded, but probably good

        // if it is not the full size, add relevant headers/status for range request
        if range_len != self.full_len {
            let content_range =
                HeaderValue::from_str(&format!("bytes {}-{}/{}", start, end, self.full_len))
                    .expect("construct content-range header failed");

            headers.insert("Content-Range", content_range);
            *res.status_mut() = StatusCode::PARTIAL_CONTENT;
        }

        res
    }
}

/// GET request handler for /p/* path.
/// All file views are handled here.
pub async fn view(
    State(engine): State<Arc<Engine>>,
    Path(original_path): Path<PathBuf>,
    range: Option<TypedHeader<Range>>,
) -> Result<UploadResponse, ViewError> {
    // try to extract the file name (if it's the only component)
    // this makes paths like `asdf%2fabcdef.png` invalid
    let saved_name = match original_path.file_name().map(OsStr::to_str) {
        Some(Some(n)) if original_path.components().count() == 1 => n,
        _ => return Err(ViewError::NotFound),
    };

    let range = range.map(|TypedHeader(range)| range);

    // get result from the engine
    match engine.get(saved_name, range).await {
        Ok(GetOutcome::Success(res)) => Ok(res),
        Ok(GetOutcome::NotFound) => Err(ViewError::NotFound),
        Ok(GetOutcome::RangeNotSatisfiable) => Err(ViewError::RangeNotSatisfiable),
        Err(err) => {
            error!("failed to get upload!! {err:#}");
            Err(ViewError::InternalServerError)
        }
    }
}
