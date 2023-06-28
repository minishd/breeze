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

pub enum ViewSuccess {
    FromDisk(File),
    FromCache(Bytes),
}

pub enum ViewError {
    NotFound,            // 404
    InternalServerError, // 500
}

impl IntoResponse for ViewSuccess {
    fn into_response(self) -> Response {
        match self {
            ViewSuccess::FromDisk(file) => {
                // get handle to current runtime
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
                let mut res = data.clone().into_response();
                let headers = res.headers_mut();

                // clear the headers, let the browser imply it
                headers.clear();

                /* // we do not need this for FromCache because it works fine
                // read the length of the data as a string
                let len_str = data.len().to_string();

                // HeaderValue::from_str will never error if only visible ASCII characters are passed (32-127)
                // .. so this should be fine
                let content_length = HeaderValue::from_str(&len_str).unwrap();
                headers.append("Content-Length", content_length);
                */

                res
            }
        }
    }
}

impl IntoResponse for ViewError {
    fn into_response(self) -> Response {
        match self {
            ViewError::NotFound => {
                // convert string into response, change status code
                let mut res = "not found!".into_response();
                *res.status_mut() = StatusCode::NOT_FOUND;

                res
            }
            ViewError::InternalServerError => {
                // convert string into response, change status code
                let mut res = "internal server error!".into_response();
                *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;

                res
            }
        }
    }
}

#[axum::debug_handler]
pub async fn view(
    State(engine): State<Arc<crate::engine::Engine>>,
    Path(original_path): Path<PathBuf>,
) -> Result<ViewSuccess, ViewError> {
    // (hopefully) prevent path traversal, just check for any non-file components
    if original_path
        .components()
        .into_iter()
        .any(|x| !matches!(x, Component::Normal(_)))
    {
        warn!("a request attempted path traversal");
        return Err(ViewError::NotFound);
    }

    // get result from the engine!
    engine.get_upload(&original_path).await
}
