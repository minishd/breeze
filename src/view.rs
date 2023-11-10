use std::{
    path::{Component, PathBuf},
    sync::Arc,
};

use axum::{
    body::StreamBody,
    extract::{Path, State},
    response::{IntoResponse, Response},
};

use bytes::Bytes;
use hyper::{http::HeaderValue, StatusCode};
use tokio::{fs::File, runtime::Handle};
use tokio_util::io::ReaderStream;
use tracing::{error, debug, warn};

/// Responses for a successful view operation
pub enum ViewSuccess {
    /// A file read from disk, suitable for larger files.
    /// 
    /// The file provided will be streamed from disk and
    /// back to the viewer.
    /// 
    /// This is only ever used if a file exceeds the
    /// cache's maximum file size.
    FromDisk(File),

    /// A file read from in-memory cache, best for smaller files.
    /// 
    /// The file is taken from the cache in its entirety
    /// and sent back to the viewer.
    /// 
    /// If a file can be fit into cache, this will be
    /// used even if it's read from disk.
    FromCache(Bytes),
}

/// Responses for a failed view operation
pub enum ViewError {
    /// Will send status code 404 witha plaintext "not found" message.
    NotFound,

    /// Will send status code 500 with a plaintext "internal server error" message.
    InternalServerError,
}

impl IntoResponse for ViewSuccess {
    fn into_response(self) -> Response {
        match self {
            ViewSuccess::FromDisk(file) => {
                // get handle to current tokio runtime
                // i use this to block on futures here (not async)
                let handle = Handle::current();
                let _ = handle.enter();

                // read the metadata of the file on disk
                // this function isn't async
                // .. so we have to use handle.block_on() to get the metadata
                let metadata = futures::executor::block_on(file.metadata());

                // if we error then return 500
                if metadata.is_err() {
                    error!("failed to read metadata from disk");
                    return ViewError::InternalServerError.into_response();
                }

                // unwrap (which we know is safe) and read the file size as a string
                let metadata = metadata.unwrap();
                let len_str = metadata.len().to_string();

                debug!("file is {} bytes on disk", &len_str);

                // HeaderValue::from_str will never error if only visible ASCII characters are passed (32-127)
                // .. so unwrapping this should be fine
                let content_length = HeaderValue::from_str(&len_str).unwrap();

                // create a streamed body response (we want to stream larger files)
                let reader = ReaderStream::new(file);
                let stream = StreamBody::new(reader);

                // extract mutable headers from the response
                let mut res = stream.into_response();
                let headers = res.headers_mut();

                // clear headers, browser can imply content type
                headers.clear();

                // insert Content-Length header
                // that way the browser shows how big a file is when it's being downloaded
                headers.insert("Content-Length", content_length);

                res
            }
            ViewSuccess::FromCache(data) => {
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
) -> Result<ViewSuccess, ViewError> {
    // (hopefully) prevent path traversal, just check for any non-file components
    if original_path
        .components()
        .any(|x| !matches!(x, Component::Normal(_)))
    {
        warn!("a request attempted path traversal");
        return Err(ViewError::NotFound);
    }

    // get result from the engine!
    engine.get_upload(&original_path).await
}
