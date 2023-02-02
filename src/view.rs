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
use hyper::StatusCode;
use tokio::fs::File;
use tokio_util::io::ReaderStream;

pub enum ViewSuccess {
    FromDisk(File),
    FromCache(Bytes),
}

pub enum ViewError {
    NotFound, // 404
    InternalServerError, // 500
}

impl IntoResponse for ViewSuccess {
    fn into_response(self) -> Response {
        match self {
            ViewSuccess::FromDisk(file) => {
                // create a streamed body response (we want to stream larger files)
                let reader = ReaderStream::new(file);
                let stream = StreamBody::new(reader);

                stream.into_response()
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
            ViewError::NotFound => {
                // convert string into response, change status code
                let mut res = "not found!".into_response();
                *res.status_mut() = StatusCode::NOT_FOUND;

                res
            }
            ViewError::InternalServerError => {
                // convert string into response, change status code
                let mut res = "internal server error!".into_response();
                *res.status_mut() = StatusCode::NOT_FOUND;

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
