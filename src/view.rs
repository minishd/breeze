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

pub enum ViewResponse {
    FromDisk(File),
    FromCache(Bytes),
}

impl IntoResponse for ViewResponse {
    fn into_response(self) -> Response {
        match self {
            ViewResponse::FromDisk(file) => {
                // create a streamed body response (we want to stream larger files)
                let reader = ReaderStream::new(file);
                let stream = StreamBody::new(reader);

                stream.into_response()
            }
            ViewResponse::FromCache(data) => {
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
        warn!(target: "view", "a request attempted path traversal");
        return Err(StatusCode::NOT_FOUND);
    }

    engine.get_upload(&original_path).await
}
